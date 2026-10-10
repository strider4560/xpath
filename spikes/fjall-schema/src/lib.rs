//! Experimental compiled-schema projection, stored in Fjall and walked by XPath.
#![forbid(unsafe_code)]

mod model;
mod navigator;
mod plan;
mod store;

pub use model::{Export, FlatNode, Module, NodeKind, NodeRecord, SchemaId, View};
pub use navigator::{NodeHandle, SchemaView};
pub use plan::{Criterion, PlanCursor, PlanRow, SelectionPlan};
pub use store::{DataValue, FlatCursor, SchemaSnapshot, SchemaStore};

#[derive(Debug)]
pub enum Error {
    Storage(fjall::Error),
    Json(serde_json::Error),
    XPath(xpath1::Error),
    Invalid(String),
    Missing(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(e) => write!(f, "storage: {e}"),
            Self::Json(e) => write!(f, "schema JSON: {e}"),
            Self::XPath(e) => write!(f, "XPath: {e}"),
            Self::Invalid(e) => write!(f, "invalid schema projection: {e}"),
            Self::Missing(e) => write!(f, "not found: {e}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<fjall::Error> for Error {
    fn from(e: fjall::Error) -> Self {
        Self::Storage(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<xpath1::Error> for Error {
    fn from(e: xpath1::Error) -> Self {
        Self::XPath(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
