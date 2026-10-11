use crate::{Result, Value};
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
    /// Does `node`'s expanded name equal `name`? Used by the evaluator for every
    /// name test, so an adapter can compare against its stored name without
    /// allocating an owned [`ExpandedName`] per candidate.
    ///
    /// The default compares [`name`](Self::name) to `name`; a name that cannot
    /// be read does not match. Override it to compare in place. It must agree
    /// with `name` for every node.
    fn name_matches(&self, node: &Self::Node, name: &ExpandedName) -> bool {
        matches!(self.name(node), Ok(Some(ref actual)) if actual == name)
    }
    /// The children of `parent` whose expanded name is `name`, in document
    /// order, or `None` if the adapter cannot answer without a scan.
    ///
    /// The evaluator asks this first for a `child::name` step and falls back to
    /// [`children`](Self::children) plus [`name_matches`](Self::name_matches)
    /// on `None`. `Some` must contain exactly the element children that a scan
    /// would select. The default is `None`.
    fn children_named(
        &self,
        _parent: &Self::Node,
        _name: &ExpandedName,
    ) -> Option<Vec<Self::Node>> {
        None
    }
    /// Point lookup of the child element `name` of `parent` whose key children
    /// equal the given values, used for a step `name[k1 = e1][k2 = e2]...`.
    ///
    /// Each entry of `keys` is a key child's expanded name and the value of its
    /// context-independent right-hand side exactly as evaluated (a string,
    /// number, boolean or node-set; not converted or canonicalized). The adapter
    /// decides equality, for example by canonicalizing the value for the key's
    /// type. Results must agree with XPath `=` semantics for the data it serves.
    ///
    /// Keys identify at most one entry: the evaluator applies any remaining
    /// predicates to the returned entry alone, at position 1 of 1.
    ///
    /// Return `None` when the lookup is not supported here (the evaluator then
    /// scans normally), `Some(None)` when no entry matches, and `Some(Some(n))`
    /// for the matching entry. The default is `None`.
    fn child_by_key(
        &self,
        _parent: &Self::Node,
        _name: &ExpandedName,
        _keys: &[(ExpandedName, Value<Self::Node>)],
    ) -> Option<Option<Self::Node>> {
        None
    }
    /// Resolve an ID according to the embedding data model. Ordinary `id`
    /// attributes are not automatically XML IDs.
    fn element_by_id(&self, _context: &Self::Node, _id: &str) -> Result<Option<Self::Node>> {
        Ok(None)
    }
}
