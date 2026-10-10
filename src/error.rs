use std::{fmt, ops::Range};

/// The category of a compile or evaluation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    Syntax,
    Type,
    Namespace,
    Variable,
    Function,
    Arity,
    Navigation,
    Limit,
}

/// An XPath error. Syntax spans use byte offsets in the original expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub span: Option<Range<usize>>,
}
impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            span: None,
        }
    }
    pub fn syntax(message: impl Into<String>, span: Range<usize>) -> Self {
        Self {
            kind: ErrorKind::Syntax,
            message: message.into(),
            span: Some(span),
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(s) = &self.span {
            write!(f, " at bytes {}..{}", s.start, s.end)?;
        }
        Ok(())
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
