//! Navigator conformance kit (`conformance` feature).
//!
//! A storage-backed [`Navigator`] implements up to seven hooks
//! ([`name_matches`](Navigator::name_matches),
//! [`children_named`](Navigator::children_named),
//! [`child_by_key`](Navigator::child_by_key),
//! [`descendants_named`](Navigator::descendants_named),
//! [`count_children_named`](Navigator::count_children_named),
//! [`exists_child_named`](Navigator::exists_child_named) and
//! [`compare_document_order`](Navigator::compare_document_order)) whose only
//! contract is "agree with the scan". This kit makes that contract executable:
//!
//! - [`run`] builds the host's navigator from each pinned Jaxen fixture
//!   document and runs the full Jaxen corpus (296 assertion definitions in 62
//!   contexts) against it, then the hook-consistency matrix of
//!   [`check_hooks`] on every fixture.
//! - [`check_hooks`] checks one navigator: for every root or element node
//!   and every element name present (plus one absent name), each hook that
//!   answers must equal the scan of [`children`](Navigator::children) and
//!   [`name`](Navigator::name); `compare_document_order` must be a total
//!   order consistent with the tree. Hosts can run it on their own data too.
//! - [`ScanHooks`] wraps a navigator and answers every hook from scans of it:
//!   a reference "all hooks answered" implementation.
//!
//! The XML adapter is the reference implementation and passes the kit:
//!
//! ```
//! use xpath1::{conformance, xml::Document};
//! let report = conformance::run(|source| {
//!     let document = Document::parse(source)?;
//!     let root = document.root();
//!     Ok((document, root))
//! });
//! report.assert_passed();
//! assert_eq!(report.definitions(), 296);
//! ```
//!
//! The fixtures are rendered for the host as XML text ([`DOCUMENTS`]); the
//! host's builder must model them faithfully (elements, attributes,
//! namespace nodes, text, comments and processing instructions), as the
//! corpus expectations depend on all of them. The catalog's extension
//! functions (`document`, `evaluate`, `upper-case`, `lower-case`,
//! `ends-with`) are supplied by the kit.
use crate::{
    value::string_to_number, Context, Error, ErrorKind, ExpandedName, Navigator, NodeKind, Result,
    Value, XPath,
};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// The pinned Jaxen fixture documents the corpus refers to, as
/// `(catalog path, XML source)`.
pub const DOCUMENTS: &[(&str, &str)] = &[
    ("xml/axis.xml", include_str!("../vendor/jaxen/xml/axis.xml")),
    (
        "xml/contents.xml",
        include_str!("../vendor/jaxen/xml/contents.xml"),
    ),
    (
        "xml/defaultNamespace.xml",
        include_str!("../vendor/jaxen/xml/defaultNamespace.xml"),
    ),
    (
        "xml/evaluate.xml",
        include_str!("../vendor/jaxen/xml/evaluate.xml"),
    ),
    ("xml/fibo.xml", include_str!("../vendor/jaxen/xml/fibo.xml")),
    ("xml/id.xml", include_str!("../vendor/jaxen/xml/id.xml")),
    (
        "xml/jaxen24.xml",
        include_str!("../vendor/jaxen/xml/jaxen24.xml"),
    ),
    (
        "xml/jaxen3.xml",
        include_str!("../vendor/jaxen/xml/jaxen3.xml"),
    ),
    ("xml/lang.xml", include_str!("../vendor/jaxen/xml/lang.xml")),
    (
        "xml/message.xml",
        include_str!("../vendor/jaxen/xml/message.xml"),
    ),
    (
        "xml/moreover.xml",
        include_str!("../vendor/jaxen/xml/moreover.xml"),
    ),
    (
        "xml/much_ado.xml",
        include_str!("../vendor/jaxen/xml/much_ado.xml"),
    ),
    (
        "xml/namespaces.xml",
        include_str!("../vendor/jaxen/xml/namespaces.xml"),
    ),
    ("xml/nitf.xml", include_str!("../vendor/jaxen/xml/nitf.xml")),
    (
        "xml/numbers.xml",
        include_str!("../vendor/jaxen/xml/numbers.xml"),
    ),
    ("xml/pi.xml", include_str!("../vendor/jaxen/xml/pi.xml")),
    ("xml/pi2.xml", include_str!("../vendor/jaxen/xml/pi2.xml")),
    (
        "xml/simple.xml",
        include_str!("../vendor/jaxen/xml/simple.xml"),
    ),
    (
        "xml/testNamespaces.xml",
        include_str!("../vendor/jaxen/xml/testNamespaces.xml"),
    ),
    ("xml/text.xml", include_str!("../vendor/jaxen/xml/text.xml")),
    (
        "xml/underscore.xml",
        include_str!("../vendor/jaxen/xml/underscore.xml"),
    ),
    ("xml/web.xml", include_str!("../vendor/jaxen/xml/web.xml")),
];

/// The pinned Jaxen catalog of contexts and assertions.
const CATALOG: &str = include_str!("../vendor/jaxen/xml/test/tests.xml");
const VARIABLES: &str = "https://github.com/jaxen-xpath/jaxen/test-harness/var";
/// Assertions that depend on the catalog's extension functions. Descendants of
/// one of these inherit its classification.
const EXTENSIONS: &[&str] = &[
    "evaluate('//jumps/*')",
    "evaluate('//jumps/object/dog')",
    "evaluate('//jumps/object')/evaluate",
    "evaluate('//jumps/object')/dog",
    "evaluate('//jumps/*')/dog",
    "//metatest[ evaluate(@select) = . ]",
    "document('xml/web.xml')",
    "document('xml/web.xml')/web-app/servlet[1]/servlet-name",
    "concat(./@id,document('xml/web.xml')/web-app/servlet[1]/servlet-name,./@id)",
    "document('xml/message.xml')/message/body/data/items/item[name/text()='parentinfo']/value",
    "upper-case( servlet-class )",
    "lower-case( servlet-class )",
    "upper-case( servlet-class, 'fr' )",
    "upper-case( servlet-class, 'fr-CA' )",
    "upper-case( servlet-class, 'es-ES-Traditional_WIN' )",
    "ends-with( servlet-class, 'Servlet' )",
    "ends-with( servlet-class, 'S' )",
];
/// The corpus accounting a complete run must reproduce: contexts, core,
/// extension, expected-exception and nested assertion executions.
const EXPECTED: (usize, usize, usize, usize, usize) = (62, 277, 19, 9, 15);

/// The outcome of [`run`] or [`check_hooks`].
#[derive(Debug, Default, Clone)]
pub struct Report {
    /// Context selections executed.
    pub contexts: usize,
    /// Core XPath 1.0 assertion executions.
    pub core: usize,
    /// Extension-function assertion executions.
    pub extensions: usize,
    /// Expected-exception executions (included in `core` / `extensions`).
    pub exceptions: usize,
    /// Nested assertion executions (included in `core` / `extensions`).
    pub nested: usize,
    /// Corpus assertions that failed, with their location.
    pub failures: Vec<String>,
    /// Hook-consistency comparisons made.
    pub hook_checks: usize,
    /// Hook-consistency comparisons that failed.
    pub violations: Vec<String>,
}

impl Report {
    /// Assertion executions, core and extension.
    pub fn definitions(&self) -> usize {
        self.core + self.extensions
    }
    /// No corpus failures and no hook violations.
    pub fn passed(&self) -> bool {
        self.failures.is_empty() && self.violations.is_empty()
    }
    /// Panic with the report unless it [`passed`](Self::passed).
    pub fn assert_passed(&self) {
        assert!(self.passed(), "{self}");
    }
    fn merge(&mut self, other: Report) {
        self.contexts += other.contexts;
        self.core += other.core;
        self.extensions += other.extensions;
        self.exceptions += other.exceptions;
        self.nested += other.nested;
        self.failures.extend(other.failures);
        self.hook_checks += other.hook_checks;
        self.violations.extend(other.violations);
    }
    fn violation(&mut self, message: String) {
        self.violations.push(message);
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Jaxen: {} contexts; {} core and {} extension executions; {} expected exceptions; \
             {} nested executions; {} failures. Hooks: {} checks; {} violations.",
            self.contexts,
            self.core,
            self.extensions,
            self.exceptions,
            self.nested,
            self.failures.len(),
            self.hook_checks,
            self.violations.len()
        )?;
        const SHOWN: usize = 50;
        for line in self.failures.iter().chain(&self.violations).take(SHOWN) {
            writeln!(f, "  {line}")?;
        }
        let total = self.failures.len() + self.violations.len();
        if total > SHOWN {
            writeln!(f, "  ... and {} more", total - SHOWN)?;
        }
        Ok(())
    }
}

/// Build the host's navigator for every fixture in [`DOCUMENTS`], run the
/// Jaxen corpus against them, then [`check_hooks`] on each one.
///
/// `build` receives a fixture's XML source and returns the navigator and
/// its root node. A fixture that fails to build is reported as a failure.
pub fn run<N, B>(mut build: B) -> Report
where
    N: Navigator,
    B: FnMut(&str) -> Result<(N, N::Node)>,
{
    let mut report = Report::default();
    let mut documents = Vec::new();
    for (path, source) in DOCUMENTS {
        match build(source) {
            Ok((navigator, root)) => documents.push(Fixture {
                path,
                navigator,
                root,
            }),
            Err(error) => report
                .failures
                .push(format!("{path}: building the navigator failed: {error}")),
        }
    }
    let corpus = Corpus { documents };
    corpus.run(&mut report);
    for fixture in &corpus.documents {
        let mut hooks = check_hooks(&fixture.navigator, &fixture.root);
        for violation in &mut hooks.violations {
            *violation = format!("{}: {violation}", fixture.path);
        }
        report.merge(hooks);
    }
    report
}

// ---------------------------------------------------------------------------
// The Jaxen corpus over a host navigator.

struct Fixture<N: Navigator> {
    path: &'static str,
    navigator: N,
    root: N::Node,
}

/// The fixture documents as one navigator, so `document()` can cross them.
/// Every method, hooks included, forwards to the host's navigator.
struct Corpus<N: Navigator> {
    documents: Vec<Fixture<N>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CorpusNode<H> {
    document: usize,
    node: H,
}

impl<N: Navigator> Corpus<N> {
    fn wrap(document: usize) -> impl Fn(N::Node) -> CorpusNode<N::Node> {
        move |node| CorpusNode { document, node }
    }
    fn nav(&self, node: &CorpusNode<N::Node>) -> &N {
        &self.documents[node.document].navigator
    }
    fn root(&self, path: &str) -> Result<CorpusNode<N::Node>> {
        self.documents
            .iter()
            .position(|fixture| fixture.path == path)
            .map(|document| CorpusNode {
                document,
                node: self.documents[document].root.clone(),
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Function,
                    format!("fixture document not found: {path}"),
                )
            })
    }
    fn wrap_all(document: usize, nodes: Vec<N::Node>) -> Vec<CorpusNode<N::Node>> {
        nodes.into_iter().map(Self::wrap(document)).collect()
    }

    fn run(&self, report: &mut Report) {
        let catalog = roxmltree::Document::parse(CATALOG).expect("pinned catalog parses");
        let mut contexts = 0;
        for document in catalog.root_element().children().filter(|n| n.is_element()) {
            let path = document.attribute("url").unwrap_or_default();
            for spec in document.children().filter(|n| n.is_element()) {
                contexts += 1;
                let select = spec.attribute("select").unwrap_or_default();
                let root = match self.root(path) {
                    Ok(root) => root,
                    Err(error) => {
                        report.failures.push(format!("{path}: {error}"));
                        continue;
                    }
                };
                match XPath::compile(select)
                    .and_then(|xpath| xpath.select(&self.context(root, spec)))
                {
                    Ok(nodes) if !nodes.is_empty() => {
                        for node in nodes {
                            for child in spec.children().filter(|n| is_assertion(*n)) {
                                self.assertion(node.clone(), child, path, report);
                            }
                        }
                    }
                    Ok(_) => report.failures.push(format!(
                        "{path} context {select:?}: context selection is empty"
                    )),
                    Err(error) => report
                        .failures
                        .push(format!("{path} context {select:?}: {error}")),
                }
            }
        }
        report.contexts += contexts;
        let accounting = (
            report.contexts,
            report.core,
            report.extensions,
            report.exceptions,
            report.nested,
        );
        if accounting != EXPECTED {
            report.failures.push(format!(
                "corpus accounting (contexts, core, extensions, exceptions, nested) \
                 is {accounting:?}, expected {EXPECTED:?}"
            ));
        }
    }

    fn context<'a>(
        &'a self,
        node: CorpusNode<N::Node>,
        specification: roxmltree::Node<'_, '_>,
    ) -> Context<'a, Self> {
        let mut namespaces = BTreeMap::new();
        for ns in specification.namespaces() {
            if let Some(prefix) = ns.name() {
                namespaces.insert(prefix.to_owned(), ns.uri().to_owned());
            }
        }
        let mut ctx = Context::new(self, node);
        for (prefix, uri) in &namespaces {
            ctx = ctx.with_namespace(prefix, uri);
        }
        let ancestors: Vec<_> = specification
            .ancestors()
            .filter(|n| n.is_element())
            .collect();
        for ancestor in ancestors.into_iter().rev() {
            for attr in ancestor
                .attributes()
                .filter(|a| a.namespace() == Some(VARIABLES))
            {
                ctx = ctx.with_variable(attr.name(), Value::String(attr.value().to_owned()));
            }
        }
        ctx = ctx.with_function(ExpandedName::local("document"), |c, args| {
            if args.len() != 1 {
                return Err(Error::new(
                    ErrorKind::Arity,
                    "document expects one argument",
                ));
            }
            let path = args[0].to_string(c.navigator)?;
            // Only documents already loaded from the pinned corpus.
            Ok(Value::Nodes(vec![c.navigator.root(&path)?]))
        });
        ctx = ctx.with_function(ExpandedName::local("evaluate"), move |c, args| {
            if args.len() != 1 {
                return Err(Error::new(
                    ErrorKind::Arity,
                    "evaluate expects one argument",
                ));
            }
            let mut inner =
                Context::new(c.navigator, c.node.clone()).with_focus(c.position, c.size)?;
            for (prefix, uri) in &namespaces {
                inner = inner.with_namespace(prefix, uri);
            }
            XPath::compile(&args[0].to_string(c.navigator)?)?.evaluate(&inner)
        });
        for name in ["upper-case", "lower-case", "ends-with"] {
            ctx = ctx.with_function(ExpandedName::local(name), move |c, args| {
                if args.is_empty() || args.len() > 2 || (name == "ends-with" && args.len() != 2) {
                    return Err(Error::new(
                        ErrorKind::Arity,
                        "invalid string extension arity",
                    ));
                }
                let value = args[0].to_string(c.navigator)?;
                Ok(match name {
                    "upper-case" => Value::String(value.to_uppercase()),
                    "lower-case" => Value::String(value.to_lowercase()),
                    _ => Value::Boolean(value.ends_with(&args[1].to_string(c.navigator)?)),
                })
            });
        }
        ctx
    }

    fn assertion(
        &self,
        node: CorpusNode<N::Node>,
        spec: roxmltree::Node<'_, '_>,
        path: &str,
        report: &mut Report,
    ) {
        let select = spec.attribute("select").unwrap_or_default();
        if extension(spec) {
            report.extensions += 1;
        } else {
            report.core += 1;
        }
        if spec.parent().is_some_and(is_assertion) {
            report.nested += 1;
        }
        let location = format!(
            "{path}:{} {select:?}",
            spec.document().text_pos_at(spec.range().start).row
        );
        let result =
            XPath::compile(select).and_then(|xpath| xpath.evaluate(&self.context(node, spec)));
        if spec.attribute("exception") == Some("true") {
            report.exceptions += 1;
            if result.is_ok() {
                report.failures.push(format!(
                    "{location}: expected exception, evaluation succeeded"
                ));
            }
            return;
        }
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                report.failures.push(format!("{location}: {error}"));
                return;
            }
        };
        if spec.tag_name().name() == "valueOf" {
            let expected: String = spec
                .children()
                .filter(|n| n.is_text())
                .filter_map(|n| n.text())
                .collect();
            match value.to_string(self) {
                Ok(actual) if actual == expected => {}
                Ok(actual) => report
                    .failures
                    .push(format!("{location}: expected {expected:?}, got {actual:?}")),
                Err(error) => report
                    .failures
                    .push(format!("{location}: string conversion failed: {error}")),
            }
            return;
        }
        let expected: Option<usize> = spec.attribute("count").and_then(|v| v.parse().ok());
        match value.as_nodes() {
            Ok(nodes) => {
                if let Some(expected) = expected {
                    if nodes.len() != expected {
                        report.failures.push(format!(
                            "{location}: expected count {expected}, got {}",
                            nodes.len()
                        ));
                    }
                }
                for child in spec.children().filter(|n| is_assertion(*n)) {
                    match nodes.first() {
                        Some(first) => self.assertion(first.clone(), child, path, report),
                        None => report.failures.push(format!(
                            "{location}: no first selected node for nested assertion"
                        )),
                    }
                }
            }
            Err(error) => report
                .failures
                .push(format!("{location}: expected node set: {error}")),
        }
    }
}

fn is_assertion(n: roxmltree::Node<'_, '_>) -> bool {
    n.is_element() && matches!(n.tag_name().name(), "test" | "valueOf")
}

fn extension(n: roxmltree::Node<'_, '_>) -> bool {
    n.ancestors()
        .filter(|n| is_assertion(*n))
        .any(|n| EXTENSIONS.contains(&n.attribute("select").unwrap_or_default()))
}

impl<N: Navigator> Navigator for Corpus<N> {
    type Node = CorpusNode<N::Node>;
    fn validate_node(&self, node: &Self::Node) -> Result<()> {
        match self.documents.get(node.document) {
            Some(fixture) => fixture.navigator.validate_node(&node.node),
            None => Err(Error::new(
                ErrorKind::Navigation,
                "node is not in the fixture corpus",
            )),
        }
    }
    fn kind(&self, node: &Self::Node) -> NodeKind {
        self.nav(node).kind(&node.node)
    }
    fn parent(&self, node: &Self::Node) -> Result<Option<Self::Node>> {
        Ok(self
            .nav(node)
            .parent(&node.node)?
            .map(Self::wrap(node.document)))
    }
    fn children<'a>(
        &'a self,
        node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        let wrap = Self::wrap(node.document);
        self.nav(node)
            .children(&node.node)
            .map(move |child| child.map(&wrap))
    }
    fn attributes<'a>(
        &'a self,
        node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        let wrap = Self::wrap(node.document);
        self.nav(node)
            .attributes(&node.node)
            .map(move |child| child.map(&wrap))
    }
    fn namespaces<'a>(
        &'a self,
        node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        let wrap = Self::wrap(node.document);
        self.nav(node)
            .namespaces(&node.node)
            .map(move |child| child.map(&wrap))
    }
    fn name(&self, node: &Self::Node) -> Result<Option<ExpandedName>> {
        self.nav(node).name(&node.node)
    }
    fn qualified_name(&self, node: &Self::Node) -> Result<String> {
        self.nav(node).qualified_name(&node.node)
    }
    fn string_value(&self, node: &Self::Node) -> Result<String> {
        self.nav(node).string_value(&node.node)
    }
    fn compare_document_order(&self, a: &Self::Node, b: &Self::Node) -> Ordering {
        a.document
            .cmp(&b.document)
            .then_with(|| self.nav(a).compare_document_order(&a.node, &b.node))
    }
    fn name_matches(&self, node: &Self::Node, name: &ExpandedName) -> bool {
        self.nav(node).name_matches(&node.node, name)
    }
    fn children_named(&self, parent: &Self::Node, name: &ExpandedName) -> Option<Vec<Self::Node>> {
        let found = self.nav(parent).children_named(&parent.node, name)?;
        Some(Self::wrap_all(parent.document, found))
    }
    fn descendants_named(&self, node: &Self::Node, name: &ExpandedName) -> Option<Vec<Self::Node>> {
        let found = self.nav(node).descendants_named(&node.node, name)?;
        Some(Self::wrap_all(node.document, found))
    }
    fn count_children_named(&self, parent: &Self::Node, name: &ExpandedName) -> Option<usize> {
        self.nav(parent).count_children_named(&parent.node, name)
    }
    fn exists_child_named(&self, parent: &Self::Node, name: &ExpandedName) -> Option<bool> {
        self.nav(parent).exists_child_named(&parent.node, name)
    }
    fn child_by_key(
        &self,
        parent: &Self::Node,
        name: &ExpandedName,
        keys: &[(ExpandedName, Value<Self::Node>)],
    ) -> Option<Option<Self::Node>> {
        let mut inner = Vec::with_capacity(keys.len());
        for (key, value) in keys {
            let value = match value {
                Value::Nodes(nodes) => {
                    let mut unwrapped = Vec::with_capacity(nodes.len());
                    for node in nodes {
                        // A node of another document cannot reach the host.
                        if node.document != parent.document {
                            return None;
                        }
                        unwrapped.push(node.node.clone());
                    }
                    Value::Nodes(unwrapped)
                }
                Value::Boolean(b) => Value::Boolean(*b),
                Value::Number(n) => Value::Number(*n),
                Value::String(s) => Value::String(s.clone()),
            };
            inner.push((key.clone(), value));
        }
        let found = self.nav(parent).child_by_key(&parent.node, name, &inner)?;
        Some(found.map(Self::wrap(parent.document)))
    }
    fn element_by_id(&self, context: &Self::Node, id: &str) -> Result<Option<Self::Node>> {
        Ok(self
            .nav(context)
            .element_by_id(&context.node, id)?
            .map(Self::wrap(context.document)))
    }
}

// ---------------------------------------------------------------------------
// The hook-consistency matrix.

/// Entries probed per (container, name) for `child_by_key`, and key leaves
/// per probe.
const KEY_ENTRIES: usize = 64;
const KEY_LEAVES: usize = 3;
/// Random node pairs compared per node for the document-order check.
const ORDER_SAMPLES: usize = 4;

/// The scan of one node: what every hook must agree with.
struct Scanned<H> {
    node: H,
    name: Option<ExpandedName>,
    /// Element children, in order.
    elements: Vec<usize>,
    /// For a root or element: the end (exclusive) of its subtree in the
    /// preorder container list.
    subtree_end: usize,
}

/// Check every storage hook of `navigator` against scans of its tree under
/// `root`, and `compare_document_order` against the tree's order.
///
/// For each root or element node and each element name present (plus one
/// absent name):
///
/// - `children_named`, when it answers, equals the element children with
///   that name, in order, without duplicates;
/// - `descendants_named` equals the element descendants with that name in
///   document order;
/// - `count_children_named` equals their number and `exists_child_named`
///   whether there is one;
/// - `child_by_key`, probed with up to three single-occurrence element
///   children of each entry (and one absent value) as string keys, answers
///   only when the keys select at most one entry, and then with that entry
///   (or `Some(None)` for none).
///
/// `name_matches` must agree with `name` for every node and name;
/// `parent` must return the node a scan reached it from; and
/// `compare_document_order` must be a total order of the nodes consistent
/// with the tree (each element, then its namespace nodes, its attributes and
/// its descendants; order within the namespace and attribute groups is free
/// but must be total), checked on every node against itself, on consecutive
/// nodes, within every group, and on a deterministic sample of other pairs.
pub fn check_hooks<N: Navigator>(navigator: &N, root: &N::Node) -> Report {
    let mut report = Report::default();
    let Some(tree) = Tree::scan(navigator, root, &mut report) else {
        return report;
    };
    tree.check_named(navigator, &mut report);
    tree.check_keys(navigator, &mut report);
    tree.check_order(navigator, &mut report);
    report
}

struct Tree<H> {
    /// Every node in document order (namespace and attribute groups in the
    /// navigator's iteration order), with its rank: nodes of one group share
    /// a rank, all others have distinct increasing ranks.
    order: Vec<(H, usize)>,
    /// Roots and elements in preorder.
    containers: Vec<Scanned<H>>,
    /// Element names present, plus one absent name.
    names: Vec<ExpandedName>,
    /// (first, end) index ranges in `order` of each namespace or attribute
    /// group.
    groups: Vec<(usize, usize)>,
}

impl<H: Clone + Eq + std::hash::Hash + fmt::Debug> Tree<H> {
    fn scan<N: Navigator<Node = H>>(nav: &N, root: &H, report: &mut Report) -> Option<Self> {
        let mut tree = Tree {
            order: Vec::new(),
            containers: Vec::new(),
            names: Vec::new(),
            groups: Vec::new(),
        };
        let mut names = BTreeSet::new();
        let mut rank = 0;
        // (node, index of its parent's entry in containers)
        let mut pending: Vec<(H, Option<usize>)> = vec![(root.clone(), None)];
        // Containers whose subtree is still open, as indices into containers.
        let mut open: Vec<usize> = Vec::new();
        while let Some((node, parent)) = pending.pop() {
            report.hook_checks += 1;
            if let Err(error) = nav.validate_node(&node) {
                report.violation(format!("validate_node({node:?}) failed: {error}"));
            }
            // In preorder, the parent is on the stack of open subtrees; close
            // the ones above it.
            while let Some(&last) = open.last() {
                if Some(last) == parent {
                    break;
                }
                tree.containers[last].subtree_end = tree.containers.len();
                open.pop();
            }
            let kind = nav.kind(&node);
            let name = match nav.name(&node) {
                Ok(name) => name,
                Err(error) => {
                    report.violation(format!("name({node:?}) failed: {error}"));
                    return None;
                }
            };
            if kind == NodeKind::Element {
                if let Some(name) = &name {
                    names.insert(name.clone());
                }
                if let Some(parent) = parent {
                    let index = tree.containers.len();
                    tree.containers[parent].elements.push(index);
                }
            }
            tree.order.push((node.clone(), rank));
            rank += 1;
            if !matches!(kind, NodeKind::Root | NodeKind::Element) {
                continue;
            }
            let index = tree.containers.len();
            tree.containers.push(Scanned {
                node: node.clone(),
                name,
                elements: Vec::new(),
                subtree_end: usize::MAX,
            });
            open.push(index);
            if kind == NodeKind::Element {
                for group in [
                    collect(nav, &node, nav.namespaces(&node), "namespaces", report)?,
                    collect(nav, &node, nav.attributes(&node), "attributes", report)?,
                ] {
                    if group.is_empty() {
                        continue;
                    }
                    let first = tree.order.len();
                    tree.order.extend(group.into_iter().map(|n| (n, rank)));
                    tree.groups.push((first, tree.order.len()));
                    rank += 1;
                }
            }
            let children = collect(nav, &node, nav.children(&node), "children", report)?;
            pending.extend(children.into_iter().rev().map(|child| (child, Some(index))));
        }
        let end = tree.containers.len();
        for index in open {
            tree.containers[index].subtree_end = end;
        }
        tree.names = names.into_iter().collect();
        tree.names
            .push(ExpandedName::new("urn:xpath1:conformance", "absent"));
        Some(tree)
    }

    fn element_name(&self, index: usize) -> Option<&ExpandedName> {
        self.containers[index].name.as_ref()
    }

    fn handles(&self, indices: impl IntoIterator<Item = usize>) -> Vec<H> {
        indices
            .into_iter()
            .map(|i| self.containers[i].node.clone())
            .collect()
    }

    fn check_named<N: Navigator<Node = H>>(&self, nav: &N, report: &mut Report) {
        for (node, _) in &self.order {
            let actual = nav.name(node).ok().flatten();
            for name in &self.names {
                report.hook_checks += 1;
                let expected = actual.as_ref() == Some(name);
                if nav.name_matches(node, name) != expected {
                    report.violation(format!(
                        "name_matches({node:?}, {name:?}) = {}, name() says {expected}",
                        !expected
                    ));
                }
            }
        }
        for (index, container) in self.containers.iter().enumerate() {
            let node = &container.node;
            for name in &self.names {
                let children: Vec<usize> = container
                    .elements
                    .iter()
                    .copied()
                    .filter(|&e| self.element_name(e) == Some(name))
                    .collect();
                let descendants: Vec<usize> = (index + 1..container.subtree_end)
                    .filter(|&e| self.element_name(e) == Some(name))
                    .collect();
                report.hook_checks += 4;
                if let Some(got) = nav.children_named(node, name) {
                    let want = self.handles(children.iter().copied());
                    if got != want {
                        report.violation(format!(
                            "children_named({node:?}, {name:?}) = {got:?}, scan = {want:?}"
                        ));
                    }
                }
                if let Some(got) = nav.descendants_named(node, name) {
                    let want = self.handles(descendants);
                    if got != want {
                        report.violation(format!(
                            "descendants_named({node:?}, {name:?}) = {got:?}, scan = {want:?}"
                        ));
                    }
                }
                if let Some(got) = nav.count_children_named(node, name) {
                    if got != children.len() {
                        report.violation(format!(
                            "count_children_named({node:?}, {name:?}) = {got}, scan = {}",
                            children.len()
                        ));
                    }
                }
                if let Some(got) = nav.exists_child_named(node, name) {
                    if got == children.is_empty() {
                        report.violation(format!(
                            "exists_child_named({node:?}, {name:?}) = {got}, scan = {}",
                            !children.is_empty()
                        ));
                    }
                }
            }
        }
    }

    fn check_keys<N: Navigator<Node = H>>(&self, nav: &N, report: &mut Report) {
        for container in &self.containers {
            // Group the element children by name.
            let mut lists: BTreeMap<&ExpandedName, Vec<usize>> = BTreeMap::new();
            for &e in &container.elements {
                if let Some(name) = self.element_name(e) {
                    lists.entry(name).or_default().push(e);
                }
            }
            for (name, entries) in lists {
                for &entry in entries.iter().take(KEY_ENTRIES) {
                    let Some(leaves) = self.key_leaves(nav, entry) else {
                        continue;
                    };
                    for length in 1..=leaves.len() {
                        let keys = &leaves[..length];
                        self.probe_key(nav, container, name, &entries, keys, report);
                    }
                    let absent = [(leaves[0].0.clone(), "\u{1}absent".to_string())];
                    self.probe_key(nav, container, name, &entries, &absent, report);
                }
            }
        }
    }

    /// Up to `KEY_LEAVES` element children of `entry` whose name occurs once
    /// among them, as (name, string value); `None` if there are none.
    fn key_leaves<N: Navigator<Node = H>>(
        &self,
        nav: &N,
        entry: usize,
    ) -> Option<Vec<(ExpandedName, String)>> {
        let elements = &self.containers[entry].elements;
        let mut leaves = Vec::new();
        for &leaf in elements {
            let Some(name) = self.element_name(leaf) else {
                continue;
            };
            let once = elements
                .iter()
                .filter(|&&e| self.element_name(e) == Some(name))
                .count()
                == 1;
            if once {
                let value = nav.string_value(&self.containers[leaf].node).ok()?;
                leaves.push((name.clone(), value));
                if leaves.len() == KEY_LEAVES {
                    break;
                }
            }
        }
        (!leaves.is_empty()).then_some(leaves)
    }

    fn probe_key<N: Navigator<Node = H>>(
        &self,
        nav: &N,
        container: &Scanned<H>,
        name: &ExpandedName,
        entries: &[usize],
        keys: &[(ExpandedName, String)],
        report: &mut Report,
    ) {
        let arguments: Vec<_> = keys
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect();
        report.hook_checks += 1;
        let Some(answer) = nav.child_by_key(&container.node, name, &arguments) else {
            return;
        };
        // XPath `=` between the key children and the string: existential.
        let selected: Vec<H> = entries
            .iter()
            .filter(|&&entry| {
                keys.iter().all(|(key, value)| {
                    self.containers[entry].elements.iter().any(|&leaf| {
                        self.element_name(leaf) == Some(key)
                            && nav.string_value(&self.containers[leaf].node).ok().as_ref()
                                == Some(value)
                    })
                })
            })
            .map(|&entry| self.containers[entry].node.clone())
            .collect();
        let consistent = match (&answer, selected.as_slice()) {
            (None, []) => true,
            (Some(found), [only]) => found == only,
            _ => false,
        };
        if !consistent {
            report.violation(format!(
                "child_by_key({:?}, {name:?}, {arguments:?}) = Some({answer:?}), \
                 but the keys select {selected:?} (a navigator must decline unless \
                 they determine at most one entry)",
                container.node
            ));
        }
    }

    fn check_order<N: Navigator<Node = H>>(&self, nav: &N, report: &mut Report) {
        let expect = |a: &H, b: &H, want: Ordering, why: &str, report: &mut Report| {
            report.hook_checks += 1;
            let got = nav.compare_document_order(a, b);
            if got != want {
                report.violation(format!(
                    "compare_document_order({a:?}, {b:?}) = {got:?}, expected {want:?} ({why})"
                ));
            }
        };
        for (node, _) in &self.order {
            expect(node, node, Ordering::Equal, "a node equals itself", report);
        }
        for pair in self.order.windows(2) {
            let ((a, ra), (b, rb)) = (&pair[0], &pair[1]);
            if ra != rb {
                expect(
                    a,
                    b,
                    Ordering::Less,
                    "consecutive in document order",
                    report,
                );
                expect(
                    b,
                    a,
                    Ordering::Greater,
                    "consecutive in document order",
                    report,
                );
            }
        }
        for &(first, end) in &self.groups {
            let group = &self.order[first..end];
            let mut sorted: Vec<&H> = group.iter().map(|(n, _)| n).collect();
            sorted.sort_by(|a, b| nav.compare_document_order(a, b));
            for (i, a) in sorted.iter().enumerate() {
                for b in &sorted[i + 1..] {
                    expect(a, b, Ordering::Less, "distinct nodes of one group", report);
                    expect(
                        b,
                        a,
                        Ordering::Greater,
                        "distinct nodes of one group",
                        report,
                    );
                }
            }
        }
        // A deterministic sample of other pairs (64-bit LCG).
        let count = self.order.len();
        if count < 2 {
            return;
        }
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize % count
        };
        for _ in 0..count * ORDER_SAMPLES {
            let (i, j) = (next(), next());
            let ((a, ra), (b, rb)) = (&self.order[i], &self.order[j]);
            if ra != rb {
                expect(a, b, ra.cmp(rb), "sampled pair", report);
            }
        }
    }
}

/// Collect an axis iterator, checking each node's parent.
fn collect<N: Navigator>(
    nav: &N,
    parent: &N::Node,
    nodes: impl Iterator<Item = Result<N::Node>>,
    axis: &str,
    report: &mut Report,
) -> Option<Vec<N::Node>> {
    let mut collected = Vec::new();
    for node in nodes {
        match node {
            Ok(node) => {
                report.hook_checks += 1;
                match nav.parent(&node) {
                    Ok(Some(ref p)) if p == parent => {}
                    other => report.violation(format!(
                        "parent({node:?}) = {other:?}, but {axis}({parent:?}) returned it"
                    )),
                }
                collected.push(node);
            }
            Err(error) => {
                report.violation(format!("{axis}({parent:?}) failed: {error}"));
                return None;
            }
        }
    }
    Some(collected)
}

// ---------------------------------------------------------------------------
// A reference navigator answering every hook.

/// Wraps a navigator and answers every storage hook from scans of it: the
/// reference "all hooks answered" implementation, for differential tests of
/// the evaluator's hook paths and as a model for a storage adapter.
///
/// Hooks answer `None` (decline) when a scan of the wrapped navigator fails,
/// so ordinary evaluation reports the error. `child_by_key` answers whenever
/// the keys, compared with XPath `=` semantics, select at most one entry.
#[derive(Debug, Clone)]
pub struct ScanHooks<N>(pub N);

impl<N: Navigator> ScanHooks<N> {
    pub fn new(navigator: N) -> Self {
        Self(navigator)
    }
    fn element_children(&self, parent: &N::Node, name: &ExpandedName) -> Option<Vec<N::Node>> {
        let nav = &self.0;
        if !matches!(nav.kind(parent), NodeKind::Root | NodeKind::Element) {
            return Some(Vec::new());
        }
        let mut found = Vec::new();
        for child in nav.children(parent) {
            let child = child.ok()?;
            if nav.kind(&child) == NodeKind::Element
                && nav.name(&child).ok()?.as_ref() == Some(name)
            {
                found.push(child);
            }
        }
        Some(found)
    }
    /// XPath `key = value` for the entry `entry`, or `None` on a read error.
    fn key_equals(
        &self,
        entry: &N::Node,
        key: &ExpandedName,
        value: &Value<N::Node>,
    ) -> Option<bool> {
        let leaves = self.element_children(entry, key)?;
        Some(match value {
            Value::Boolean(b) => leaves.is_empty() != *b,
            Value::Number(n) => {
                let mut equal = false;
                for leaf in &leaves {
                    equal |= string_to_number(&self.0.string_value(leaf).ok()?) == *n;
                }
                equal
            }
            Value::String(s) => {
                let mut equal = false;
                for leaf in &leaves {
                    equal |= self.0.string_value(leaf).ok()? == *s;
                }
                equal
            }
            Value::Nodes(nodes) => {
                let mut wanted = BTreeSet::new();
                for node in nodes {
                    wanted.insert(self.0.string_value(node).ok()?);
                }
                let mut equal = false;
                for leaf in &leaves {
                    equal |= wanted.contains(&self.0.string_value(leaf).ok()?);
                }
                equal
            }
        })
    }
}

impl<N: Navigator> Navigator for ScanHooks<N> {
    type Node = N::Node;
    fn validate_node(&self, node: &Self::Node) -> Result<()> {
        self.0.validate_node(node)
    }
    fn kind(&self, node: &Self::Node) -> NodeKind {
        self.0.kind(node)
    }
    fn parent(&self, node: &Self::Node) -> Result<Option<Self::Node>> {
        self.0.parent(node)
    }
    fn children<'a>(
        &'a self,
        node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        self.0.children(node)
    }
    fn attributes<'a>(
        &'a self,
        node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        self.0.attributes(node)
    }
    fn namespaces<'a>(
        &'a self,
        node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        self.0.namespaces(node)
    }
    fn name(&self, node: &Self::Node) -> Result<Option<ExpandedName>> {
        self.0.name(node)
    }
    fn qualified_name(&self, node: &Self::Node) -> Result<String> {
        self.0.qualified_name(node)
    }
    fn string_value(&self, node: &Self::Node) -> Result<String> {
        self.0.string_value(node)
    }
    fn compare_document_order(&self, a: &Self::Node, b: &Self::Node) -> Ordering {
        self.0.compare_document_order(a, b)
    }
    fn name_matches(&self, node: &Self::Node, name: &ExpandedName) -> bool {
        self.0.name_matches(node, name)
    }
    fn children_named(&self, parent: &Self::Node, name: &ExpandedName) -> Option<Vec<Self::Node>> {
        self.element_children(parent, name)
    }
    fn descendants_named(&self, node: &Self::Node, name: &ExpandedName) -> Option<Vec<Self::Node>> {
        let nav = &self.0;
        let mut found = Vec::new();
        let mut pending = Vec::new();
        if matches!(nav.kind(node), NodeKind::Root | NodeKind::Element) {
            for child in nav.children(node) {
                pending.push(child.ok()?);
            }
        }
        pending.reverse();
        while let Some(next) = pending.pop() {
            if nav.kind(&next) != NodeKind::Element {
                continue;
            }
            if nav.name(&next).ok()?.as_ref() == Some(name) {
                found.push(next.clone());
            }
            let start = pending.len();
            for child in nav.children(&next) {
                pending.push(child.ok()?);
            }
            pending[start..].reverse();
        }
        Some(found)
    }
    fn count_children_named(&self, parent: &Self::Node, name: &ExpandedName) -> Option<usize> {
        Some(self.element_children(parent, name)?.len())
    }
    fn exists_child_named(&self, parent: &Self::Node, name: &ExpandedName) -> Option<bool> {
        Some(!self.element_children(parent, name)?.is_empty())
    }
    fn child_by_key(
        &self,
        parent: &Self::Node,
        name: &ExpandedName,
        keys: &[(ExpandedName, Value<Self::Node>)],
    ) -> Option<Option<Self::Node>> {
        let mut selected = None;
        for entry in self.element_children(parent, name)? {
            let mut all = true;
            for (key, value) in keys {
                if !self.key_equals(&entry, key, value)? {
                    all = false;
                    break;
                }
            }
            if all {
                if selected.is_some() {
                    // The keys do not determine one entry.
                    return None;
                }
                selected = Some(entry);
            }
        }
        Some(selected)
    }
    fn element_by_id(&self, context: &Self::Node, id: &str) -> Result<Option<Self::Node>> {
        self.0.element_by_id(context, id)
    }
}
