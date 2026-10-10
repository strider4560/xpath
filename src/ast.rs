use crate::Axis;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QName {
    pub(crate) prefix: Option<String>,
    pub(crate) local: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    String(String),
    Number(f64),
    Variable(QName),
    Function(QName, Vec<Expr>),
    Unary(Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    Path {
        base: Option<Box<Expr>>,
        absolute: bool,
        steps: Vec<Step>,
    },
    Filter {
        base: Box<Expr>,
        predicates: Vec<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Step {
    pub(crate) axis: Axis,
    pub(crate) test: NodeTest,
    pub(crate) predicates: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NodeTest {
    Any,
    Name(QName),
    NamespaceWildcard(String),
    Node,
    Text,
    Comment,
    ProcessingInstruction(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
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
