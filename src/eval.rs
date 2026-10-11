use crate::{
    ast::{BinaryOp, Expr, NodeTest, QName, Step},
    functions,
    value::string_to_number,
    Axis, Context, Error, ErrorKind, ExpandedName, FunctionContext, Navigator, NodeKind, Result,
    Value,
};
use std::{cmp::Ordering, collections::HashSet};

struct Focus<N> {
    node: N,
    position: usize,
    size: usize,
}

// Keep the original position/size for a predicate while compacting in place.
// Only already-visited slots are swapped; the remaining focus order is intact.
// If navigation fails, the partially compacted private buffer is discarded.
fn retain_nodes<T>(
    nodes: &mut Vec<T>,
    mut keep: impl FnMut(&T, usize, usize) -> Result<bool>,
) -> Result<()> {
    let size = nodes.len();
    let mut retained = 0;
    for index in 0..size {
        if keep(&nodes[index], index + 1, size)? {
            if retained != index {
                nodes.swap(retained, index);
            }
            retained += 1;
        }
    }
    nodes.truncate(retained);
    Ok(())
}

pub(crate) fn evaluate<N: Navigator>(expr: &Expr, ctx: &Context<'_, N>) -> Result<Value<N::Node>> {
    ctx.navigator.validate_node(&ctx.node)?;
    let mut evaluator = Evaluator {
        ctx,
        remaining: ctx.limits.max_operations,
    };
    evaluator.expr(
        expr,
        &Focus {
            node: ctx.node.clone(),
            position: ctx.position,
            size: ctx.size,
        },
        0,
    )
}

struct Evaluator<'a, 'b, N: Navigator> {
    ctx: &'a Context<'b, N>,
    remaining: usize,
}

impl<N: Navigator> Evaluator<'_, '_, N> {
    fn spend(&mut self, amount: usize) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or_else(|| Error::new(ErrorKind::Limit, "XPath evaluation work limit exceeded"))?;
        Ok(())
    }

    fn normalize(&self, mut nodes: Vec<N::Node>) -> Vec<N::Node> {
        // The navigator promises a total order consistent with node identity.
        // Sorting groups duplicate handles without hashing/cloning each one or
        // allocating a second collection. Empty/singleton sets need no work.
        if nodes.len() > 1 {
            nodes.sort_unstable_by(|a, b| self.ctx.navigator.compare_document_order(a, b));
            nodes.dedup();
        }
        nodes
    }

    fn canonicalize(&mut self, value: Value<N::Node>) -> Result<Value<N::Node>> {
        match value {
            Value::Nodes(nodes) => {
                self.spend(nodes.len())?;
                for node in &nodes {
                    self.ctx.navigator.validate_node(node)?;
                }
                Ok(Value::Nodes(self.normalize(nodes)))
            }
            other => Ok(other),
        }
    }

    fn namespace(&self, prefix: &str) -> Result<String> {
        if prefix == "xml" {
            return Ok("http://www.w3.org/XML/1998/namespace".into());
        }
        self.ctx.namespaces.get(prefix).cloned().ok_or_else(|| {
            Error::new(
                ErrorKind::Namespace,
                format!("unbound namespace prefix: {prefix}"),
            )
        })
    }

    fn name(&self, name: &QName) -> Result<ExpandedName> {
        Ok(ExpandedName::new(
            match &name.prefix {
                Some(p) => self.namespace(p)?,
                None => String::new(),
            },
            &name.local,
        ))
    }

    fn expr(
        &mut self,
        expr: &Expr,
        focus: &Focus<N::Node>,
        depth: usize,
    ) -> Result<Value<N::Node>> {
        // Invariant: every node-set leaving this function is distinct and in
        // document order. Caller-supplied variables/functions are canonicalized
        // at entry; step, union and filter preserve the invariant internally.
        self.spend(1)?;
        if depth >= self.ctx.limits.max_depth {
            return Err(Error::new(
                ErrorKind::Limit,
                "XPath evaluation depth limit exceeded",
            ));
        }
        match expr {
            Expr::String(s) => Ok(Value::String(s.clone())),
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::Variable(q) => {
                let name = self.name(q)?;
                let value = self.ctx.variables.get(&name).ok_or_else(|| {
                    Error::new(
                        ErrorKind::Variable,
                        format!("unbound variable: {}", q.local),
                    )
                })?;
                if let Value::Nodes(nodes) = value {
                    if nodes.len() > self.remaining {
                        return Err(Error::new(
                            ErrorKind::Limit,
                            "variable node-set exceeds work limit",
                        ));
                    }
                }
                self.canonicalize(value.clone())
            }
            Expr::Function(q, arguments) => {
                let name = self.name(q)?;
                let mut args = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    args.push(self.expr(argument, focus, depth + 1)?);
                }
                let call_context = FunctionContext {
                    navigator: self.ctx.navigator,
                    node: &focus.node,
                    initial_node: &self.ctx.node,
                    position: focus.position,
                    size: focus.size,
                };
                if name.namespace_uri.is_empty() {
                    if let Some(result) =
                        functions::call(&name.local_name, &args, &call_context, &mut self.remaining)
                    {
                        return self.canonicalize(result?);
                    }
                }
                let function = self.ctx.function(&name).ok_or_else(|| {
                    Error::new(
                        ErrorKind::Function,
                        format!("unknown function: {}", q.local),
                    )
                })?;
                self.canonicalize(function(&call_context, &args)?)
            }
            Expr::Unary(inner) => Ok(Value::Number(
                -self
                    .expr(inner, focus, depth + 1)?
                    .to_number(self.ctx.navigator)?,
            )),
            Expr::Binary(operator, left, right) => {
                let a = self.expr(left, focus, depth + 1)?;
                if *operator == BinaryOp::Or && a.to_boolean() {
                    return Ok(Value::Boolean(true));
                }
                if *operator == BinaryOp::And && !a.to_boolean() {
                    return Ok(Value::Boolean(false));
                }
                let b = self.expr(right, focus, depth + 1)?;
                self.binary(*operator, a, b)
            }
            Expr::Path {
                base,
                absolute,
                steps,
            } => {
                let mut nodes = if let Some(base) = base {
                    self.expr(base, focus, depth + 1)?.into_nodes()?
                } else if *absolute {
                    vec![self.root(&focus.node)?]
                } else {
                    vec![focus.node.clone()]
                };
                for step in steps {
                    nodes = self.step(nodes, step, depth + 1)?;
                }
                Ok(Value::Nodes(nodes))
            }
            Expr::Filter { base, predicates } => {
                let nodes = self.expr(base, focus, depth + 1)?.into_nodes()?;
                Ok(Value::Nodes(self.filter(nodes, predicates, depth + 1)?))
            }
        }
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        a: Value<N::Node>,
        b: Value<N::Node>,
    ) -> Result<Value<N::Node>> {
        use BinaryOp::*;
        let nav = self.ctx.navigator;
        match op {
            Or => Ok(Value::Boolean(a.to_boolean() || b.to_boolean())),
            And => Ok(Value::Boolean(a.to_boolean() && b.to_boolean())),
            Union => {
                let left = a.into_nodes()?;
                let right = b.into_nodes()?;
                self.spend(left.len() + right.len())?;
                // Both operands are already canonical. Merge them in linear
                // time; sorting their concatenation repeats order comparisons
                // that may be expensive for an application-backed navigator.
                let mut nodes = Vec::with_capacity(left.len() + right.len());
                let mut left = left.into_iter().peekable();
                let mut right = right.into_iter().peekable();
                while let (Some(a), Some(b)) = (left.peek(), right.peek()) {
                    match nav.compare_document_order(a, b) {
                        Ordering::Less => nodes.push(left.next().unwrap()),
                        Ordering::Greater => nodes.push(right.next().unwrap()),
                        Ordering::Equal => {
                            nodes.push(left.next().unwrap());
                            right.next();
                        }
                    }
                }
                nodes.extend(left);
                nodes.extend(right);
                Ok(Value::Nodes(nodes))
            }
            Eq | Ne => Ok(Value::Boolean(self.equality(&a, &b, op == Ne)?)),
            Lt | Le | Gt | Ge => Ok(Value::Boolean(self.relational(&a, &b, op)?)),
            Add | Sub | Mul | Div | Mod => {
                let x = a.to_number(nav)?;
                let y = b.to_number(nav)?;
                Ok(Value::Number(match op {
                    Add => x + y,
                    Sub => x - y,
                    Mul => x * y,
                    Div => x / y,
                    Mod => x % y,
                    _ => unreachable!(),
                }))
            }
        }
    }

    fn equality(&mut self, a: &Value<N::Node>, b: &Value<N::Node>, unequal: bool) -> Result<bool> {
        let nav = self.ctx.navigator;
        match (a, b) {
            (Value::Nodes(xs), Value::Nodes(ys)) => {
                let Some((first, rest)) = xs.split_first().filter(|_| !ys.is_empty()) else {
                    return Ok(false);
                };
                self.spend(1)?;
                let first = nav.string_value(first)?;

                // Keep the first row's short-circuit order: an early match
                // must not read later fallible node values. Build a string
                // index only if equality can require another row.
                let mut strings = HashSet::new();
                for y in ys {
                    self.spend(1)?;
                    let y = nav.string_value(y)?;
                    if (first == y) != unequal {
                        return Ok(true);
                    }
                    if !unequal && !rest.is_empty() {
                        strings.insert(y);
                    }
                }

                for x in rest {
                    self.spend(1)?;
                    let x = nav.string_value(x)?;
                    // If != reached here, every right value equals `first`.
                    // A differing left value suffices. != is not the negation
                    // of existential equality.
                    if if unequal {
                        x != first
                    } else {
                        strings.contains(&x)
                    } {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            (Value::Nodes(nodes), scalar) | (scalar, Value::Nodes(nodes)) => {
                if let Value::Boolean(value) = scalar {
                    return Ok((nodes.is_empty() != *value) != unequal);
                }
                for node in nodes {
                    self.spend(1)?;
                    let string = nav.string_value(node)?;
                    let equal = match scalar {
                        Value::Number(number) => string_to_number(&string) == *number,
                        Value::String(other) => string == *other,
                        _ => unreachable!(),
                    };
                    if equal != unequal {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => {
                let equal = if matches!(a, Value::Boolean(_)) || matches!(b, Value::Boolean(_)) {
                    a.to_boolean() == b.to_boolean()
                } else if matches!(a, Value::Number(_)) || matches!(b, Value::Number(_)) {
                    a.to_number(nav)? == b.to_number(nav)?
                } else {
                    a.to_string(nav)? == b.to_string(nav)?
                };
                Ok(equal != unequal)
            }
        }
    }

    fn relational(&mut self, a: &Value<N::Node>, b: &Value<N::Node>, op: BinaryOp) -> Result<bool> {
        let nav = self.ctx.navigator;
        let numbers = |value: &Value<N::Node>| -> Result<Vec<f64>> {
            match value {
                Value::Nodes(nodes) => nodes
                    .iter()
                    .map(|n| Ok(string_to_number(&nav.string_value(n)?)))
                    .collect(),
                _ => Ok(vec![value.to_number(nav)?]),
            }
        };
        let xs = numbers(a)?;
        let ys = numbers(b)?;
        for x in &xs {
            for y in &ys {
                self.spend(1)?;
                if match op {
                    BinaryOp::Lt => x < y,
                    BinaryOp::Le => x <= y,
                    BinaryOp::Gt => x > y,
                    BinaryOp::Ge => x >= y,
                    _ => unreachable!(),
                } {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn filter(
        &mut self,
        mut nodes: Vec<N::Node>,
        predicates: &[Expr],
        depth: usize,
    ) -> Result<Vec<N::Node>> {
        for predicate in predicates {
            retain_nodes(&mut nodes, |node, position, size| {
                let value = self.expr(
                    predicate,
                    &Focus {
                        node: node.clone(),
                        position,
                        size,
                    },
                    depth,
                )?;
                Ok(match value {
                    Value::Number(n) => n == position as f64,
                    _ => value.to_boolean(),
                })
            })?;
        }
        Ok(nodes)
    }

    fn step(&mut self, nodes: Vec<N::Node>, step: &Step, depth: usize) -> Result<Vec<N::Node>> {
        let test = self.resolve_test(&step.test, step.axis)?;
        let principal = match step.axis {
            Axis::Attribute => NodeKind::Attribute,
            Axis::Namespace => NodeKind::Namespace,
            _ => NodeKind::Element,
        };
        let keyed = self.key_run(step, &test);
        let mut result = Vec::new();
        for node in nodes {
            self.spend(1)?;
            if let Some((name, keys)) = &keyed {
                if let Some(found) = self.child_by_key(&node, name, keys, depth)? {
                    result.extend(self.filter(found, &step.predicates[keys.len()..], depth)?);
                    continue;
                }
            }
            let matching = match (&test, step.axis) {
                (ResolvedTest::Name(name), Axis::Child) => self.children_named(&node, name)?,
                _ => None,
            };
            let matching = match matching {
                Some(matching) => matching,
                None => {
                    let mut matching = self.axis(&node, step.axis)?;
                    retain_nodes(&mut matching, |candidate, _, _| {
                        self.matches(candidate, &test, principal)
                    })?;
                    matching
                }
            };
            result.extend(self.filter(matching, &step.predicates, depth)?);
        }
        Ok(self.normalize(result))
    }

    /// R2: the leading run of `[k = e]` predicates of a `child::name` step
    /// that a keyed lookup can answer, as (key name, right-hand side) pairs.
    ///
    /// Each predicate must be `=` between a one-step relative `child::k` name
    /// test without predicates and a context-independent expression (either
    /// operand order). The first predicate of any other shape, including every
    /// position-dependent one, ends the run. `None` if the run is empty.
    fn key_run<'e>(
        &self,
        step: &'e Step,
        test: &ResolvedTest,
    ) -> Option<(ExpandedName, Vec<(ExpandedName, &'e Expr)>)> {
        let ResolvedTest::Name(name) = test else {
            return None;
        };
        if step.axis != Axis::Child {
            return None;
        }
        let mut keys = Vec::new();
        for predicate in &step.predicates {
            let Expr::Binary(BinaryOp::Eq, left, right) = predicate else {
                break;
            };
            let pair = match (key_child(left), key_child(right)) {
                (Some(key), _) if context_independent(right) => (key, &**right),
                (_, Some(key)) if context_independent(left) => (key, &**left),
                _ => break,
            };
            // An unresolvable key prefix ends the run; ordinary evaluation
            // then reports it exactly when it would have before.
            match self.resolve_test(&NodeTest::Name(pair.0.clone()), Axis::Child) {
                Ok(ResolvedTest::Name(key)) => keys.push((key, pair.1)),
                _ => break,
            }
        }
        (!keys.is_empty()).then(|| (name.clone(), keys))
    }

    /// R2: evaluate each key's right-hand side once against `node` and ask the
    /// navigator for the entry. `None` means "scan normally": the hook is not
    /// supported here, or a right-hand side failed (ordinary evaluation then
    /// reports that failure only if it reaches it). The work budget spent on a
    /// declined attempt is refunded so the fallback is indistinguishable.
    fn child_by_key(
        &mut self,
        node: &N::Node,
        name: &ExpandedName,
        keys: &[(ExpandedName, &Expr)],
        depth: usize,
    ) -> Result<Option<Vec<N::Node>>> {
        let nav = self.ctx.navigator;
        if !matches!(nav.kind(node), NodeKind::Root | NodeKind::Element) {
            return Ok(None);
        }
        let budget = self.remaining;
        let focus = Focus {
            node: node.clone(),
            position: 1,
            size: 1,
        };
        let mut values = Vec::with_capacity(keys.len());
        for (key, rhs) in keys {
            match self.expr(rhs, &focus, depth) {
                Ok(value) => values.push((key.clone(), value)),
                Err(_) => {
                    self.remaining = budget;
                    return Ok(None);
                }
            }
        }
        match nav.child_by_key(node, name, &values) {
            None => {
                self.remaining = budget;
                Ok(None)
            }
            Some(None) => Ok(Some(Vec::new())),
            Some(Some(found)) => {
                self.spend(1)?;
                nav.validate_node(&found)?;
                Ok(Some(vec![found]))
            }
        }
    }

    /// R6: ask the navigator for the element children named `name`. `None`
    /// means the caller scans `children()` with `name_matches` instead.
    fn children_named(
        &mut self,
        node: &N::Node,
        name: &ExpandedName,
    ) -> Result<Option<Vec<N::Node>>> {
        let nav = self.ctx.navigator;
        if !matches!(nav.kind(node), NodeKind::Root | NodeKind::Element) {
            return Ok(Some(Vec::new()));
        }
        let Some(mut children) = nav.children_named(node, name) else {
            return Ok(None);
        };
        self.spend(children.len())?;
        for child in &children {
            nav.validate_node(child)?;
        }
        // Element children only, as the principal-kind test of a scan requires.
        children.retain(|child| nav.kind(child) == NodeKind::Element);
        Ok(Some(children))
    }

    fn resolve_test(&self, test: &NodeTest, axis: Axis) -> Result<ResolvedTest> {
        Ok(match test {
            NodeTest::Any => ResolvedTest::Any,
            NodeTest::Name(n) => ResolvedTest::Name(
                if n.prefix.is_none() && !matches!(axis, Axis::Attribute | Axis::Namespace) {
                    ExpandedName::new(&self.ctx.default_element_namespace, &n.local)
                } else {
                    self.name(n)?
                },
            ),
            NodeTest::NamespaceWildcard(p) => ResolvedTest::Namespace(self.namespace(p)?),
            NodeTest::Node => ResolvedTest::Node,
            NodeTest::Text => ResolvedTest::Kind(NodeKind::Text),
            NodeTest::Comment => ResolvedTest::Kind(NodeKind::Comment),
            NodeTest::ProcessingInstruction(target) => ResolvedTest::Pi(target.clone()),
        })
    }

    fn matches(&self, node: &N::Node, test: &ResolvedTest, principal: NodeKind) -> Result<bool> {
        let nav = self.ctx.navigator;
        let kind = nav.kind(node);
        Ok(match test {
            ResolvedTest::Any => kind == principal,
            ResolvedTest::Name(name) => kind == principal && nav.name_matches(node, name),
            ResolvedTest::Namespace(uri) => {
                kind == principal && nav.name(node)?.is_some_and(|n| n.namespace_uri == *uri)
            }
            ResolvedTest::Node => true,
            ResolvedTest::Kind(k) => kind == *k,
            ResolvedTest::Pi(target) => {
                kind == NodeKind::ProcessingInstruction
                    && match target {
                        None => true,
                        Some(target) => nav.name(node)?.is_some_and(|n| n.local_name == *target),
                    }
            }
        })
    }

    fn parent(&mut self, node: &N::Node) -> Result<Option<N::Node>> {
        self.spend(1)?;
        let parent = self.ctx.navigator.parent(node)?;
        if let Some(node) = &parent {
            self.ctx.navigator.validate_node(node)?;
        }
        Ok(parent)
    }

    fn root(&mut self, node: &N::Node) -> Result<N::Node> {
        let mut node = node.clone();
        while let Some(parent) = self.parent(&node)? {
            node = parent;
        }
        Ok(node)
    }

    fn children(&mut self, node: &N::Node) -> Result<Vec<N::Node>> {
        if !matches!(
            self.ctx.navigator.kind(node),
            NodeKind::Root | NodeKind::Element
        ) {
            return Ok(Vec::new());
        }
        let mut nodes = Vec::new();
        for child in self.ctx.navigator.children(node) {
            self.spend(1)?;
            let child = child?;
            self.ctx.navigator.validate_node(&child)?;
            nodes.push(child);
        }
        Ok(nodes)
    }

    fn descendants(&mut self, node: &N::Node, include_self: bool) -> Result<Vec<N::Node>> {
        let mut result = if include_self {
            vec![node.clone()]
        } else {
            Vec::new()
        };
        let mut pending = self.children(node)?;
        pending.reverse();
        while let Some(child) = pending.pop() {
            result.push(child.clone());
            let mut children = self.children(&child)?;
            children.reverse();
            pending.extend(children);
        }
        Ok(result)
    }

    fn siblings(&mut self, node: &N::Node, preceding: bool) -> Result<Vec<N::Node>> {
        if matches!(
            self.ctx.navigator.kind(node),
            NodeKind::Attribute | NodeKind::Namespace
        ) {
            return Ok(Vec::new());
        }
        let Some(parent) = self.parent(node)? else {
            return Ok(Vec::new());
        };
        let children = self.children(&parent)?;
        let Some(index) = children.iter().position(|n| n == node) else {
            return Ok(Vec::new());
        };
        Ok(if preceding {
            children[..index].iter().rev().cloned().collect()
        } else {
            children[index + 1..].to_vec()
        })
    }

    fn axis(&mut self, node: &N::Node, axis: Axis) -> Result<Vec<N::Node>> {
        use Axis::*;
        match axis {
            SelfAxis => Ok(vec![node.clone()]),
            Child => self.children(node),
            Descendant => self.descendants(node, false),
            DescendantOrSelf => self.descendants(node, true),
            Parent => Ok(self.parent(node)?.into_iter().collect()),
            Ancestor | AncestorOrSelf => {
                let mut result = if axis == AncestorOrSelf {
                    vec![node.clone()]
                } else {
                    Vec::new()
                };
                let mut current = node.clone();
                while let Some(parent) = self.parent(&current)? {
                    result.push(parent.clone());
                    current = parent;
                }
                Ok(result)
            }
            Attribute => {
                let mut result = Vec::new();
                if self.ctx.navigator.kind(node) == NodeKind::Element {
                    for n in self.ctx.navigator.attributes(node) {
                        self.spend(1)?;
                        let n = n?;
                        self.ctx.navigator.validate_node(&n)?;
                        result.push(n);
                    }
                }
                Ok(result)
            }
            Namespace => {
                let mut result = Vec::new();
                if self.ctx.navigator.kind(node) == NodeKind::Element {
                    for n in self.ctx.navigator.namespaces(node) {
                        self.spend(1)?;
                        let n = n?;
                        self.ctx.navigator.validate_node(&n)?;
                        result.push(n);
                    }
                }
                Ok(result)
            }
            FollowingSibling => self.siblings(node, false),
            PrecedingSibling => self.siblings(node, true),
            Following | Preceding => {
                let preceding = axis == Preceding;
                let mut result = Vec::new();
                let mut current = node.clone();
                if matches!(
                    self.ctx.navigator.kind(node),
                    NodeKind::Attribute | NodeKind::Namespace
                ) {
                    let Some(owner) = self.parent(node)? else {
                        return Ok(result);
                    };
                    if !preceding {
                        result.extend(self.descendants(&owner, false)?);
                    }
                    current = owner;
                }
                loop {
                    for sibling in self.siblings(&current, preceding)? {
                        let mut subtree = self.descendants(&sibling, true)?;
                        if preceding {
                            subtree.reverse();
                        }
                        result.extend(subtree);
                    }
                    match self.parent(&current)? {
                        Some(parent) => current = parent,
                        None => break,
                    }
                }
                Ok(result)
            }
        }
    }
}

/// `k` or `child::k`: a one-step relative child name test without predicates.
fn key_child(expr: &Expr) -> Option<&QName> {
    match expr {
        Expr::Path {
            base: None,
            absolute: false,
            steps,
        } => match steps.as_slice() {
            [Step {
                axis: Axis::Child,
                test: NodeTest::Name(name),
                predicates,
            }] if predicates.is_empty() => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// A string or number literal, a variable, `current()`, or a path rooted at
/// `current()`. `current()` is assumed to be the YANG function returning the
/// context's initial node, so none of these depend on the predicate's focus.
fn context_independent(expr: &Expr) -> bool {
    fn is_current(expr: &Expr) -> bool {
        matches!(expr, Expr::Function(name, args)
            if name.prefix.is_none() && name.local == "current" && args.is_empty())
    }
    match expr {
        Expr::String(_) | Expr::Number(_) | Expr::Variable(_) => true,
        Expr::Path {
            base: Some(base), ..
        } => is_current(base),
        other => is_current(other),
    }
}

enum ResolvedTest {
    Any,
    Name(ExpandedName),
    Namespace(String),
    Node,
    Kind(NodeKind),
    Pi(Option<String>),
}
