//! XPath 1.0 evaluation over application-defined trees.
//!
//! Compile an expression once with [`XPath::compile`], then evaluate it against
//! a [`Context`] and your implementation of [`Navigator`]. The optional `xml`
//! module provides an owned XML adapter.
#![forbid(unsafe_code)]

pub mod ast;
mod context;
mod error;
mod eval;
mod functions;
mod model;
pub mod namespaces;
mod parser;
mod value;
#[cfg(feature = "xml")]
pub mod xml;

pub use context::{Context, FunctionContext, FunctionTable, Limits};
pub use error::{Error, ErrorKind, Result};
pub use model::{Axis, ExpandedName, Navigator, NodeKind};
pub use namespaces::Namespaces;
pub use value::Value;

/// An immutable compiled XPath expression, reusable with different data and bindings.
#[derive(Debug, Clone)]
pub struct XPath {
    expression: ast::Expr,
    source: String,
}
impl XPath {
    pub fn compile(source: &str) -> Result<Self> {
        Ok(Self {
            expression: parser::parse(source)?,
            source: source.into(),
        })
    }
    /// The compiled expression tree, read-only, for static analysis.
    pub fn ast(&self) -> &ast::Expr {
        &self.expression
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn evaluate<N: Navigator>(&self, context: &Context<'_, N>) -> Result<Value<N::Node>> {
        eval::evaluate(&self.expression, context)
    }
    /// Evaluate an expression that returns a node-set, or return a type error.
    pub fn select<N: Navigator>(&self, context: &Context<'_, N>) -> Result<Vec<N::Node>> {
        match self.evaluate(context)? {
            Value::Nodes(nodes) => Ok(nodes),
            _ => Err(Error::new(
                ErrorKind::Type,
                "expression did not return a node-set",
            )),
        }
    }
}
impl std::str::FromStr for XPath {
    type Err = Error;
    fn from_str(source: &str) -> Result<Self> {
        Self::compile(source)
    }
}
