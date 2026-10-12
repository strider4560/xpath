//! Shareable namespace bindings with a cache of resolved names.
use crate::{ast::QName, ExpandedName};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    rc::Rc,
};

/// An immutable set of namespace bindings that many [`Context`](crate::Context)s
/// can share through [`Context::with_namespaces`](crate::Context::with_namespaces),
/// typically one set per read.
///
/// The set remembers every name it has resolved, so a warm evaluation of the
/// same expression resolves its names without allocating. Because the bindings
/// cannot change after construction, the cache never goes stale. A context
/// that adds bindings of its own (`Context::with_namespace` or
/// `Context::with_default_element_namespace`) still resolves through this set
/// where its own bindings are silent, but never writes to the cache.
#[derive(Debug, Default)]
pub struct Namespaces {
    map: BTreeMap<String, String>,
    default_element_namespace: String,
    /// Names resolved with the null namespace for unprefixed names (variables,
    /// functions, attributes and prefixed element tests).
    resolved: RefCell<HashMap<QName, Rc<ExpandedName>>>,
    /// Unprefixed element tests, which take the default element namespace,
    /// keyed by local name.
    resolved_elements: RefCell<HashMap<String, Rc<ExpandedName>>>,
}

impl Namespaces {
    pub fn new() -> Self {
        Self::default()
    }
    /// Bind `prefix` to `uri`, replacing any earlier binding of `prefix`.
    pub fn with_namespace(mut self, prefix: impl Into<String>, uri: impl Into<String>) -> Self {
        self.map.insert(prefix.into(), uri.into());
        self
    }
    /// The namespace of unprefixed element tests; see
    /// [`Context::with_default_element_namespace`](crate::Context::with_default_element_namespace).
    pub fn with_default_element_namespace(mut self, uri: impl Into<String>) -> Self {
        self.default_element_namespace = uri.into();
        self
    }
    /// The URI bound to `prefix` in this set (the implicit `xml` binding excluded).
    pub fn namespace(&self, prefix: &str) -> Option<&str> {
        self.map.get(prefix).map(String::as_str)
    }
    pub fn default_element_namespace(&self) -> &str {
        &self.default_element_namespace
    }

    pub(crate) fn cached(&self, name: &QName) -> Option<Rc<ExpandedName>> {
        self.resolved.borrow().get(name).cloned()
    }
    pub(crate) fn cache(&self, name: &QName, resolved: Rc<ExpandedName>) {
        self.resolved.borrow_mut().insert(name.clone(), resolved);
    }
    pub(crate) fn cached_element(&self, local: &str) -> Option<Rc<ExpandedName>> {
        self.resolved_elements.borrow().get(local).cloned()
    }
    pub(crate) fn cache_element(&self, local: &str, resolved: Rc<ExpandedName>) {
        self.resolved_elements
            .borrow_mut()
            .insert(local.to_owned(), resolved);
    }
}
