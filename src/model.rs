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
///
/// # Storage hooks
///
/// A storage-backed navigator can answer some step shapes with a prefix scan
/// or point get instead of letting the evaluator enumerate nodes. The family
/// is closed; every other shape materializes under [`Limits`](crate::Limits):
///
/// | Shape (name test resolved, context node a root or element) | Hook |
/// |---|---|
/// | any name test | [`name_matches`](Self::name_matches) |
/// | `child::name` | [`children_named`](Self::children_named) |
/// | `child::name[k = e]...` (leading key run, see [`ast::Step::key_run`](crate::ast::Step::key_run)) | [`child_by_key`](Self::child_by_key) |
/// | `descendant::name`, `descendant-or-self::name`, `//name` | [`descendants_named`](Self::descendants_named) |
/// | `count(name)` | [`count_children_named`](Self::count_children_named) |
/// | `name` in boolean position (`and`/`or` operand, `boolean()`, `not()`, `[name]`) | [`exists_child_named`](Self::exists_child_named) |
///
/// Each hook defaults to declining (`None`), the evaluator falls back to the
/// scan on `None` with the work budget it spent on the attempt refunded, and
/// an answer must agree exactly with that scan. The `conformance` feature's
/// kit checks this contract for an implementation.
///
/// A navigator is used by one thread per evaluation; it need not be `Sync`.
/// [`FunctionTable`](crate::FunctionTable) built with
/// [`new_sync`](crate::FunctionTable::new_sync) is `Send + Sync`, so it can be
/// shared across threads; [`Namespaces`](crate::Namespaces),
/// [`NodeSetPool`](crate::NodeSetPool) and [`Context`](crate::Context) are
/// per-thread objects for one set of evaluations.
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
    /// The element descendants of `node` whose expanded name is `name`, in
    /// document order, without duplicates, or `None` if the adapter cannot
    /// answer without a scan. Does not include `node` itself.
    ///
    /// The evaluator asks this first for a `descendant::name` or
    /// `descendant-or-self::name` step, and for the `//name` spelling
    /// (`descendant-or-self::node()/child::name` with no predicates on the
    /// `child::` step). It falls back to a descendant scan plus
    /// [`name_matches`](Self::name_matches) on `None`. `Some` must contain
    /// exactly the element descendants a scan would select, in document
    /// order. The default is `None`.
    fn descendants_named(
        &self,
        _node: &Self::Node,
        _name: &ExpandedName,
    ) -> Option<Vec<Self::Node>> {
        None
    }
    /// Number of element children of `parent` named `name`, or `None` to
    /// decline. Must equal `children_named(parent, name).len()` when both
    /// answer.
    ///
    /// The evaluator asks this for `count(name)` (a relative, single
    /// `child::name` step without predicates). The default is `None`.
    fn count_children_named(&self, _parent: &Self::Node, _name: &ExpandedName) -> Option<usize> {
        None
    }
    /// Whether `parent` has at least one element child named `name`, or
    /// `None` to decline. Must equal `!children_named(parent, name).is_empty()`
    /// when both answer.
    ///
    /// The evaluator asks this for a relative, single `child::name` step
    /// without predicates in boolean position: an operand of `and` / `or`, the
    /// argument of `boolean()` or `not()`, or a predicate `[name]`. The
    /// default is `None`.
    fn exists_child_named(&self, _parent: &Self::Node, _name: &ExpandedName) -> Option<bool> {
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
    /// The evaluator treats the answer as the only candidate: it applies any
    /// remaining predicates to the returned entry alone, at position 1 of 1.
    /// So the navigator must answer only when `keys` determine at most one
    /// entry: it must return `None` (decline) when `keys` do not cover every
    /// key leaf of the list or name a key leaf more than once, and the
    /// evaluator then falls back to a scan.
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
