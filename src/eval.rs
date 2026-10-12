use crate::{
    ast::{BinaryOp, Expr, NodeTest, QName, Step},
    functions,
    value::string_to_number,
    Axis, Context, Error, ErrorKind, ExpandedName, FunctionContext, Namespaces, Navigator,
    NodeKind, Result, Value,
};
use std::{cmp::Ordering, collections::HashSet, rc::Rc};

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

impl<'a, N: Navigator> Evaluator<'a, '_, N> {
    fn spend(&mut self, amount: usize) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or_else(|| Error::new(ErrorKind::Limit, "XPath evaluation work limit exceeded"))?;
        Ok(())
    }

    /// An empty node-set buffer, from the context's pool if it has one.
    fn take(&self) -> Vec<N::Node> {
        self.ctx.pool.map(|pool| pool.take()).unwrap_or_default()
    }

    /// Give a finished intermediate buffer back to the pool (or drop it).
    fn give(&self, buffer: Vec<N::Node>) {
        if let Some(pool) = self.ctx.pool {
            pool.give(buffer);
        }
    }

    /// Give back a discarded value's node-set buffer, if it has one.
    fn give_value(&self, value: Value<N::Node>) {
        if let Value::Nodes(nodes) = value {
            self.give(nodes);
        }
    }

    /// Sorts and deduplicates in place, in the buffer it was given.
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

    /// The URI bound to `prefix`, borrowed from the context or its shared set.
    fn namespace(&self, prefix: &str) -> Result<&'a str> {
        if prefix == "xml" {
            return Ok(XML_NAMESPACE);
        }
        if let Some(uri) = self.ctx.namespaces.get(prefix) {
            return Ok(uri);
        }
        self.ctx
            .shared_namespaces
            .and_then(|shared| shared.namespace(prefix))
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Namespace,
                    format!("unbound namespace prefix: {prefix}"),
                )
            })
    }

    /// The shared set whose cache this evaluation may read and write: only
    /// when the context has no bindings of its own, so every resolution went
    /// through the shared set alone.
    fn cache(&self) -> Option<&Namespaces> {
        self.ctx.shared_namespaces.filter(|_| {
            self.ctx.namespaces.is_empty() && self.ctx.default_element_namespace.is_empty()
        })
    }

    /// Resolve a QName; an unprefixed name is in the null namespace.
    fn name(&self, name: &QName) -> Result<Rc<ExpandedName>> {
        let cache = self.cache();
        if let Some(found) = cache.and_then(|cache| cache.cached(name)) {
            return Ok(found);
        }
        let resolved = Rc::new(ExpandedName::new(
            match &name.prefix {
                Some(p) => self.namespace(p)?,
                None => "",
            },
            &name.local,
        ));
        if let Some(cache) = cache {
            cache.cache(name, resolved.clone());
        }
        Ok(resolved)
    }

    /// Resolve an unprefixed element test in the default element namespace.
    fn element_name(&self, local: &str) -> Rc<ExpandedName> {
        let cache = self.cache();
        if let Some(found) = cache.and_then(|cache| cache.cached_element(local)) {
            return found;
        }
        let default = match self.ctx.shared_namespaces {
            Some(shared) if self.ctx.default_element_namespace.is_empty() => {
                shared.default_element_namespace()
            }
            _ => &self.ctx.default_element_namespace,
        };
        let resolved = Rc::new(ExpandedName::new(default, local));
        if let Some(cache) = cache {
            cache.cache_element(local, resolved.clone());
        }
        resolved
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
                let value = self.ctx.variables.get(&*name).ok_or_else(|| {
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
                let value = match value {
                    Value::Nodes(nodes) => {
                        let mut copy = self.take();
                        copy.extend_from_slice(nodes);
                        Value::Nodes(copy)
                    }
                    other => other.clone(),
                };
                self.canonicalize(value)
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
                let core = if name.namespace_uri.is_empty() {
                    functions::call(&name.local_name, &args, &call_context, &mut self.remaining)
                } else {
                    None
                };
                let result = match core {
                    Some(result) => result,
                    None => {
                        let function = self.ctx.function(&name).ok_or_else(|| {
                            Error::new(
                                ErrorKind::Function,
                                format!("unknown function: {}", q.local),
                            )
                        })?;
                        function(&call_context, &args)
                    }
                };
                for argument in args {
                    self.give_value(argument);
                }
                self.canonicalize(result?)
            }
            Expr::Unary(inner) => {
                let value = self.expr(inner, focus, depth + 1)?;
                let number = value.to_number(self.ctx.navigator);
                self.give_value(value);
                Ok(Value::Number(-number?))
            }
            Expr::Binary(operator, left, right) => {
                let a = self.expr(left, focus, depth + 1)?;
                if *operator == BinaryOp::Or && a.to_boolean() {
                    self.give_value(a);
                    return Ok(Value::Boolean(true));
                }
                if *operator == BinaryOp::And && !a.to_boolean() {
                    self.give_value(a);
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
                } else {
                    let start = if *absolute {
                        self.root(&focus.node)?
                    } else {
                        focus.node.clone()
                    };
                    let mut nodes = self.take();
                    nodes.push(start);
                    nodes
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
        if op == Union {
            let mut left = a.into_nodes()?;
            let mut right = b.into_nodes()?;
            self.spend(left.len() + right.len())?;
            // Both operands are already canonical. Merge them in linear
            // time; sorting their concatenation repeats order comparisons
            // that may be expensive for an application-backed navigator.
            let mut nodes = self.take();
            nodes.reserve(left.len() + right.len());
            {
                let mut left = left.drain(..).peekable();
                let mut right = right.drain(..).peekable();
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
            }
            self.give(left);
            self.give(right);
            return Ok(Value::Nodes(nodes));
        }
        let result = self.scalar_binary(op, &a, &b);
        self.give_value(a);
        self.give_value(b);
        result
    }

    /// A binary operator other than union; the operands stay with the caller.
    fn scalar_binary(
        &mut self,
        op: BinaryOp,
        a: &Value<N::Node>,
        b: &Value<N::Node>,
    ) -> Result<Value<N::Node>> {
        use BinaryOp::*;
        let nav = self.ctx.navigator;
        match op {
            Or => Ok(Value::Boolean(a.to_boolean() || b.to_boolean())),
            And => Ok(Value::Boolean(a.to_boolean() && b.to_boolean())),
            Union => unreachable!("union is evaluated by binary"),
            Eq | Ne => Ok(Value::Boolean(self.equality(a, b, op == Ne)?)),
            Lt | Le | Gt | Ge => Ok(Value::Boolean(self.relational(a, b, op)?)),
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
                let keep = match value {
                    Value::Number(n) => n == position as f64,
                    _ => value.to_boolean(),
                };
                self.give_value(value);
                Ok(keep)
            })?;
        }
        Ok(nodes)
    }

    /// Consumes `nodes` (its buffer goes back to the pool) and returns a new
    /// buffer holding the step's result.
    fn step(&mut self, mut nodes: Vec<N::Node>, step: &Step, depth: usize) -> Result<Vec<N::Node>> {
        let test = self.resolve_test(&step.test, step.axis)?;
        let principal = match step.axis {
            Axis::Attribute => NodeKind::Attribute,
            Axis::Namespace => NodeKind::Namespace,
            _ => NodeKind::Element,
        };
        let keyed = self.key_run(step, &test);
        let mut result = self.take();
        for node in nodes.drain(..) {
            self.spend(1)?;
            if let Some((name, run)) = &keyed {
                let (keys, rest) = step.predicates.split_at(*run);
                if let Some(found) = self.child_by_key(&node, name, keys, depth)? {
                    let mut found = self.filter(found, rest, depth)?;
                    result.append(&mut found);
                    self.give(found);
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
            let mut matching = self.filter(matching, &step.predicates, depth)?;
            result.append(&mut matching);
            self.give(matching);
        }
        self.give(nodes);
        Ok(self.normalize(result))
    }

    /// R2: the step's name and the length of the leading run of `[k = e]`
    /// predicates of a `child::name` step that a keyed lookup can answer.
    ///
    /// Each predicate must be `=` between a one-step relative `child::k` name
    /// test without predicates and a context-independent expression (either
    /// operand order), see [`key_pair`]. The first predicate of any other
    /// shape, including every position-dependent one, ends the run, as does a
    /// key with an unbound prefix (ordinary evaluation then reports it exactly
    /// when it would have before). `None` if the run is empty. Allocates
    /// nothing: the run is re-read from the predicates by `child_by_key`.
    fn key_run(&self, step: &Step, test: &ResolvedTest) -> Option<(Rc<ExpandedName>, usize)> {
        let ResolvedTest::Name(name) = test else {
            return None;
        };
        if step.axis != Axis::Child {
            return None;
        }
        let run = step
            .predicates
            .iter()
            .take_while(|predicate| {
                key_pair(predicate).is_some_and(|(key, _)| {
                    key.prefix
                        .as_deref()
                        .is_none_or(|prefix| self.namespace(prefix).is_ok())
                })
            })
            .count();
        (run > 0).then(|| (name.clone(), run))
    }

    /// R2: evaluate each key's right-hand side once against `node` and ask the
    /// navigator for the entry. `keys` is a run accepted by `key_run`. `None`
    /// means "scan normally": the hook is not supported here, or a right-hand
    /// side failed (ordinary evaluation then reports that failure only if it
    /// reaches it). The work budget spent on a declined attempt is refunded so
    /// the fallback is indistinguishable.
    fn child_by_key(
        &mut self,
        node: &N::Node,
        name: &ExpandedName,
        keys: &[Expr],
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
        for predicate in keys {
            // Both always succeed for a run accepted by `key_run`; a failure
            // would just decline the lookup.
            let Some((key, rhs)) = key_pair(predicate) else {
                self.remaining = budget;
                return Ok(None);
            };
            let Ok(key) = self.test_name(key, Axis::Child) else {
                self.remaining = budget;
                return Ok(None);
            };
            match self.expr(rhs, &focus, depth + 1) {
                Ok(value) => values.push((ExpandedName::clone(&key), value)),
                Err(_) => {
                    self.remaining = budget;
                    return Ok(None);
                }
            }
        }
        let entry = nav.child_by_key(node, name, &values);
        for (_, value) in values {
            self.give_value(value);
        }
        match entry {
            None => {
                self.remaining = budget;
                Ok(None)
            }
            Some(None) => Ok(Some(self.take())),
            Some(Some(found)) => {
                self.spend(1)?;
                nav.validate_node(&found)?;
                let mut nodes = self.take();
                nodes.push(found);
                Ok(Some(nodes))
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

    /// The expanded name of a name test on `axis`: unprefixed element tests
    /// take the default element namespace.
    fn test_name(&self, name: &QName, axis: Axis) -> Result<Rc<ExpandedName>> {
        if name.prefix.is_none() && !matches!(axis, Axis::Attribute | Axis::Namespace) {
            Ok(self.element_name(&name.local))
        } else {
            self.name(name)
        }
    }

    /// Borrows wildcard URIs and PI targets, so resolving a test never
    /// allocates beyond a cold name resolution.
    fn resolve_test<'t>(&self, test: &'t NodeTest, axis: Axis) -> Result<ResolvedTest<'t>>
    where
        'a: 't,
    {
        Ok(match test {
            NodeTest::Any => ResolvedTest::Any,
            NodeTest::Name(n) => ResolvedTest::Name(self.test_name(n, axis)?),
            NodeTest::NamespaceWildcard(p) => ResolvedTest::Namespace(self.namespace(p)?),
            NodeTest::Node => ResolvedTest::Node,
            NodeTest::Text => ResolvedTest::Kind(NodeKind::Text),
            NodeTest::Comment => ResolvedTest::Kind(NodeKind::Comment),
            NodeTest::ProcessingInstruction(target) => ResolvedTest::Pi(target.as_deref()),
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
        let mut nodes = self.take();
        self.push_children(node, &mut nodes)?;
        Ok(nodes)
    }

    /// Append `node`'s children to `out` in document order.
    fn push_children(&mut self, node: &N::Node, out: &mut Vec<N::Node>) -> Result<()> {
        if !matches!(
            self.ctx.navigator.kind(node),
            NodeKind::Root | NodeKind::Element
        ) {
            return Ok(());
        }
        for child in self.ctx.navigator.children(node) {
            self.spend(1)?;
            let child = child?;
            self.ctx.navigator.validate_node(&child)?;
            out.push(child);
        }
        Ok(())
    }

    fn descendants(&mut self, node: &N::Node, include_self: bool) -> Result<Vec<N::Node>> {
        let mut result = self.take();
        if include_self {
            result.push(node.clone());
        }
        // A stack of unvisited nodes, each node's children pushed reversed so
        // they pop in document order.
        let mut pending = self.children(node)?;
        pending.reverse();
        while let Some(child) = pending.pop() {
            let start = pending.len();
            self.push_children(&child, &mut pending)?;
            pending[start..].reverse();
            result.push(child);
        }
        self.give(pending);
        Ok(result)
    }

    fn siblings(&mut self, node: &N::Node, preceding: bool) -> Result<Vec<N::Node>> {
        if matches!(
            self.ctx.navigator.kind(node),
            NodeKind::Attribute | NodeKind::Namespace
        ) {
            return Ok(self.take());
        }
        let Some(parent) = self.parent(node)? else {
            return Ok(self.take());
        };
        let mut children = self.children(&parent)?;
        match children.iter().position(|n| n == node) {
            None => children.clear(),
            Some(index) if preceding => {
                children.truncate(index);
                children.reverse();
            }
            Some(index) => {
                children.drain(..=index);
            }
        }
        Ok(children)
    }

    fn axis(&mut self, node: &N::Node, axis: Axis) -> Result<Vec<N::Node>> {
        use Axis::*;
        let mut result = self.take();
        match axis {
            SelfAxis => result.push(node.clone()),
            Child => self.push_children(node, &mut result)?,
            Descendant | DescendantOrSelf => {
                self.give(result);
                return self.descendants(node, axis == DescendantOrSelf);
            }
            Parent => result.extend(self.parent(node)?),
            Ancestor | AncestorOrSelf => {
                if axis == AncestorOrSelf {
                    result.push(node.clone());
                }
                let mut current = node.clone();
                while let Some(parent) = self.parent(&current)? {
                    result.push(parent.clone());
                    current = parent;
                }
            }
            Attribute => {
                if self.ctx.navigator.kind(node) == NodeKind::Element {
                    for n in self.ctx.navigator.attributes(node) {
                        self.spend(1)?;
                        let n = n?;
                        self.ctx.navigator.validate_node(&n)?;
                        result.push(n);
                    }
                }
            }
            Namespace => {
                if self.ctx.navigator.kind(node) == NodeKind::Element {
                    for n in self.ctx.navigator.namespaces(node) {
                        self.spend(1)?;
                        let n = n?;
                        self.ctx.navigator.validate_node(&n)?;
                        result.push(n);
                    }
                }
            }
            FollowingSibling | PrecedingSibling => {
                self.give(result);
                return self.siblings(node, axis == PrecedingSibling);
            }
            Following | Preceding => {
                let preceding = axis == Preceding;
                let mut current = node.clone();
                if matches!(
                    self.ctx.navigator.kind(node),
                    NodeKind::Attribute | NodeKind::Namespace
                ) {
                    let Some(owner) = self.parent(node)? else {
                        return Ok(result);
                    };
                    if !preceding {
                        let mut subtree = self.descendants(&owner, false)?;
                        result.append(&mut subtree);
                        self.give(subtree);
                    }
                    current = owner;
                }
                loop {
                    let siblings = self.siblings(&current, preceding)?;
                    for sibling in &siblings {
                        let mut subtree = self.descendants(sibling, true)?;
                        if preceding {
                            subtree.reverse();
                        }
                        result.append(&mut subtree);
                        self.give(subtree);
                    }
                    self.give(siblings);
                    match self.parent(&current)? {
                        Some(parent) => current = parent,
                        None => break,
                    }
                }
            }
        }
        Ok(result)
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

/// A predicate a keyed lookup can answer: `k = e` or `e = k`, with `k` a
/// one-step relative child name test and `e` context-independent, as
/// (key name, right-hand side).
fn key_pair(predicate: &Expr) -> Option<(&QName, &Expr)> {
    let Expr::Binary(BinaryOp::Eq, left, right) = predicate else {
        return None;
    };
    match (key_child(left), key_child(right)) {
        (Some(key), _) if context_independent(right) => Some((key, right)),
        (_, Some(key)) if context_independent(left) => Some((key, left)),
        _ => None,
    }
}

const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

enum ResolvedTest<'t> {
    Any,
    Name(Rc<ExpandedName>),
    Namespace(&'t str),
    Node,
    Kind(NodeKind),
    Pi(Option<&'t str>),
}
