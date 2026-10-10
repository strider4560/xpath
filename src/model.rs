use crate::Result;
use std::{cmp::Ordering, hash::Hash};

/// XPath's seven node kinds. The document root is distinct from its element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Root,
    Element,
    Attribute,
    Namespace,
    Text,
    Comment,
    ProcessingInstruction,
}

/// The thirteen XPath 1.0 axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Child,
    Descendant,
    Parent,
    Ancestor,
    FollowingSibling,
    PrecedingSibling,
    Following,
    Preceding,
    Attribute,
    Namespace,
    SelfAxis,
    DescendantOrSelf,
    AncestorOrSelf,
}

/// An expanded name: namespace URI plus local part. Prefixes are not identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExpandedName {
    pub namespace_uri: String,
    pub local_name: String,
}
impl ExpandedName {
    pub fn new(namespace_uri: impl Into<String>, local_name: impl Into<String>) -> Self {
        Self {
            namespace_uri: namespace_uri.into(),
            local_name: local_name.into(),
        }
    }
    pub fn local(local_name: impl Into<String>) -> Self {
        Self::new("", local_name)
    }
}

/// Bridge to an application-owned, immutable snapshot of a tree.
///
/// Handles must have stable identity during evaluation. All axes returned here are
/// in document order, without duplicates. Attributes and namespace nodes have an
/// element parent but are never children. Namespace nodes are unique per element.
/// `compare_document_order` must define a total order consistent with children:
/// element, its namespaces, its attributes, then descendants. The relative order
/// within namespace/attribute groups may be implementation-defined but stable.
/// Iterators permit storage cursors and propagate I/O errors without an XML DOM.
pub trait Navigator {
    type Node: Clone + Eq + Hash + std::fmt::Debug;
    /// Reject stale or foreign handles before infallible kind/order operations.
    /// The default suits handles whose validity is guaranteed by their type.
    fn validate_node(&self, _node: &Self::Node) -> Result<()> {
        Ok(())
    }
    fn kind(&self, node: &Self::Node) -> NodeKind;
    fn parent(&self, node: &Self::Node) -> Result<Option<Self::Node>>;
    fn children<'a>(
        &'a self,
        node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a;
    fn attributes<'a>(
        &'a self,
        _node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        std::iter::empty()
    }
    fn namespaces<'a>(
        &'a self,
        _node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        std::iter::empty()
    }
    fn name(&self, node: &Self::Node) -> Result<Option<ExpandedName>>;
    /// A lexical QName for name(); may differ from the expression's prefix.
    fn qualified_name(&self, node: &Self::Node) -> Result<String> {
        Ok(self.name(node)?.map(|n| n.local_name).unwrap_or_default())
    }
    fn string_value(&self, node: &Self::Node) -> Result<String>;
    fn compare_document_order(&self, a: &Self::Node, b: &Self::Node) -> Ordering;
    /// Resolve an ID according to the embedding data model. Ordinary `id`
    /// attributes are not automatically XML IDs.
    fn element_by_id(&self, _context: &Self::Node, _id: &str) -> Result<Option<Self::Node>> {
        Ok(None)
    }
}
