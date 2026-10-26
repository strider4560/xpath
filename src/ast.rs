//! The compiled, read-only expression tree exposed by [`XPath::ast`](crate::XPath::ast).
//!
//! Embedders may walk it for static analysis (for example, resolving name tests
//! against a schema at install time or inferring an expression's result kind).
//! Values of these types are produced only by the crate's parser; an `XPath`
//! cannot be built from a hand-made tree, so the tree reached through
//! `XPath::ast` is exactly what the evaluator runs.
use crate::Axis;

/// A lexical QName as written in the expression; the prefix is not yet resolved.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct QName {
    pub prefix: Option<String>,
    pub local: String,
}

/// An XPath 1.0 expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A string literal.
    String(String),
    /// A numeric literal.
    Number(f64),
    /// A variable reference `$name`.
    Variable(QName),
    /// A function call with its arguments.
    Function(QName, Vec<Expr>),
    /// Unary minus.
    Unary(Box<Expr>),
    /// A binary operation.
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    /// A location path: optional filter-expression base (`base/steps`),
    /// absolute (`/steps`) or relative to the context node. `//` appears as an
    /// explicit `descendant-or-self::node()` step; `.` and `..` as
    /// `self::node()` and `parent::node()`.
    Path {
        base: Option<Box<Expr>>,
        absolute: bool,
        steps: Vec<Step>,
    },
    /// A filter expression `base[predicate]...`.
    Filter {
        base: Box<Expr>,
        predicates: Vec<Expr>,
    },
}

/// One location step: `axis::test[predicate]...`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Step {
    pub axis: Axis,
    pub test: NodeTest,
    pub predicates: Vec<Expr>,
}

/// The node test of a location step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeTest {
    /// `*`
    Any,
    /// A (possibly prefixed) name.
    Name(QName),
    /// `prefix:*`; the string is the prefix.
    NamespaceWildcard(String),
    /// `node()`
    Node,
    /// `text()`
    Text,
    /// `comment()`
    Comment,
    /// `processing-instruction(target?)`
    ProcessingInstruction(Option<String>),
}

/// Binary operators, including `|` (union).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Union,
}

impl Step {
    /// Length of the leading run of predicates of the form `k = e` / `e = k`
    /// that [`Navigator::child_by_key`](crate::Navigator::child_by_key) can
    /// answer: `k` a one-step relative `child::` name test without
    /// predicates, `e` [context-independent](context_independent).
    ///
    /// This is the evaluator's own recognizer, for hosts (planners,
    /// access-control path matchers) that must agree with it. It is `0` for a
    /// step that is not `child::name`. Namespace resolution is not applied
    /// here: the evaluator additionally ends the run at a key whose prefix is
    /// unbound.
    pub fn key_run(&self) -> usize {
        if self.axis != Axis::Child || !matches!(self.test, NodeTest::Name(_)) {
            return 0;
        }
        self.predicates
            .iter()
            .take_while(|predicate| key_pair(predicate).is_some())
            .count()
    }
}

/// The name of `k` or `child::k`: a one-step relative `child::` name test
/// without predicates. Also the shape answered by the R9 count and existence
/// hooks.
pub(crate) fn single_child_name(expr: &Expr) -> Option<&QName> {
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

/// `(key name, right-hand side)` for a predicate `k = e` or `e = k`, with `k`
/// a one-step relative `child::` name test without predicates and `e`
/// [context-independent](context_independent); else `None`.
pub fn key_pair(predicate: &Expr) -> Option<(&QName, &Expr)> {
    let Expr::Binary(BinaryOp::Eq, left, right) = predicate else {
        return None;
    };
    match (single_child_name(left), single_child_name(right)) {
        (Some(key), _) if context_independent(right) => Some((key, right)),
        (_, Some(key)) if context_independent(left) => Some((key, left)),
        _ => None,
    }
}

/// A string or number literal, a variable, `current()`, or a path rooted at
/// `current()`.
///
/// `current()` is assumed to be the YANG function returning the context's
/// initial node, so none of these depend on a predicate's focus.
pub fn context_independent(expr: &Expr) -> bool {
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
