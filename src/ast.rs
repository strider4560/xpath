//! The compiled, read-only expression tree exposed by [`XPath::ast`](crate::XPath::ast).
//!
//! Embedders may walk it for static analysis (for example, resolving name tests
//! against a schema at install time or inferring an expression's result kind).
//! Values of these types are produced only by the crate's parser; an `XPath`
//! cannot be built from a hand-made tree, so the tree reached through
//! `XPath::ast` is exactly what the evaluator runs.
use crate::Axis;

/// A lexical QName as written in the expression; the prefix is not yet resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
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
