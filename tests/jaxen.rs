//! Execute the unchanged Jaxen XML oracle, including harness-only extensions.
use std::{cmp::Ordering, collections::BTreeMap};
use xpath1::{
    xml::{Document, NodeId},
    Context, Error, ErrorKind, ExpandedName, Navigator, NodeKind, Result, Value, XPath,
};

const CATALOG: &str = include_str!("../vendor/jaxen/xml/test/tests.xml");
const VARIABLES: &str = "https://github.com/jaxen-xpath/jaxen/test-harness/var";
// Exact selectors, not a substring-based skip policy. Descendants of a document()
// assertion inherit its extension classification because their context is external.
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

struct Corpus {
    documents: Vec<(String, Document)>,
}
impl Corpus {
    fn owner(&self, node: &NodeId) -> &Document {
        &self
            .documents
            .iter()
            .find(|(_, d)| d.contains_node(node))
            .expect("corpus node ownership")
            .1
    }
    fn root(&self, path: &str) -> Result<NodeId> {
        self.documents
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, d)| d.root())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Function,
                    format!("fixture document not found: {path}"),
                )
            })
    }
}
impl Navigator for Corpus {
    type Node = NodeId;
    fn validate_node(&self, node: &NodeId) -> Result<()> {
        match self.documents.iter().find(|(_, d)| d.contains_node(node)) {
            Some((_, d)) => d.validate_node(node),
            None => Err(Error::new(
                ErrorKind::Navigation,
                "node is not in the fixture corpus",
            )),
        }
    }
    fn kind(&self, node: &NodeId) -> NodeKind {
        self.owner(node).kind(node)
    }
    fn parent(&self, node: &NodeId) -> Result<Option<NodeId>> {
        self.owner(node).parent(node)
    }
    fn children<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.owner(node).children(node)
    }
    fn attributes<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.owner(node).attributes(node)
    }
    fn namespaces<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.owner(node).namespaces(node)
    }
    fn name(&self, node: &NodeId) -> Result<Option<ExpandedName>> {
        self.owner(node).name(node)
    }
    fn qualified_name(&self, node: &NodeId) -> Result<String> {
        self.owner(node).qualified_name(node)
    }
    fn string_value(&self, node: &NodeId) -> Result<String> {
        self.owner(node).string_value(node)
    }
    fn compare_document_order(&self, a: &NodeId, b: &NodeId) -> Ordering {
        let ai = self
            .documents
            .iter()
            .position(|(_, d)| d.contains_node(a))
            .unwrap();
        let bi = self
            .documents
            .iter()
            .position(|(_, d)| d.contains_node(b))
            .unwrap();
        ai.cmp(&bi)
            .then_with(|| self.documents[ai].1.compare_document_order(a, b))
    }
    fn element_by_id(&self, node: &NodeId, id: &str) -> Result<Option<NodeId>> {
        self.owner(node).element_by_id(node, id)
    }
}

fn context<'a>(
    nav: &'a Corpus,
    node: NodeId,
    specification: roxmltree::Node<'_, '_>,
) -> Context<'a, Corpus> {
    let mut namespaces = BTreeMap::new();
    for ns in specification.namespaces() {
        if let Some(prefix) = ns.name() {
            namespaces.insert(prefix.to_owned(), ns.uri().to_owned());
        }
    }
    let mut ctx = Context::new(nav, node);
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
        // Deliberately only documents already loaded from the pinned corpus.
        Ok(Value::Nodes(vec![c.navigator.root(&path)?]))
    });
    ctx = ctx.with_function(ExpandedName::local("evaluate"), move |c, args| {
        if args.len() != 1 {
            return Err(Error::new(
                ErrorKind::Arity,
                "evaluate expects one argument",
            ));
        }
        let mut inner = Context::new(c.navigator, *c.node).with_focus(c.position, c.size)?;
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

fn is_assertion(n: roxmltree::Node<'_, '_>) -> bool {
    n.is_element() && matches!(n.tag_name().name(), "test" | "valueOf")
}
fn extension(n: roxmltree::Node<'_, '_>) -> bool {
    n.ancestors()
        .filter(|n| is_assertion(*n))
        .any(|n| EXTENSIONS.contains(&n.attribute("select").unwrap()))
}

#[derive(Default)]
struct Accounting {
    core: usize,
    extensions: usize,
    exceptions: usize,
    nested: usize,
}

fn assertion(
    nav: &Corpus,
    node: NodeId,
    spec: roxmltree::Node<'_, '_>,
    path: &str,
    accounting: &mut Accounting,
    failures: &mut Vec<String>,
) {
    let select = spec.attribute("select").unwrap();
    if extension(spec) {
        accounting.extensions += 1;
    } else {
        accounting.core += 1;
    }
    if spec.parent().is_some_and(is_assertion) {
        accounting.nested += 1;
    }
    let location = format!(
        "{path}:{} {select:?}",
        spec.document().text_pos_at(spec.range().start).row
    );
    let result = XPath::compile(select).and_then(|xp| xp.evaluate(&context(nav, node, spec)));
    if spec.attribute("exception") == Some("true") {
        accounting.exceptions += 1;
        if result.is_ok() {
            failures.push(format!(
                "{location}: expected exception, evaluation succeeded"
            ));
        }
        return;
    }
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            failures.push(format!("{location}: {error}"));
            return;
        }
    };
    if spec.tag_name().name() == "valueOf" {
        let expected: String = spec
            .children()
            .filter(|n| n.is_text())
            .filter_map(|n| n.text())
            .collect();
        match value.to_string(nav) {
            Ok(actual) if actual == expected => {}
            Ok(actual) => {
                failures.push(format!("{location}: expected {expected:?}, got {actual:?}"))
            }
            Err(e) => failures.push(format!("{location}: string conversion failed: {e}")),
        }
    } else {
        let expected: Option<usize> = spec.attribute("count").map(|v| v.parse().unwrap());
        match value.as_nodes() {
            Ok(nodes) => {
                if let Some(expected) = expected {
                    if nodes.len() != expected {
                        failures.push(format!(
                            "{location}: expected count {expected}, got {}",
                            nodes.len()
                        ));
                    }
                }
                for child in spec.children().filter(|n| is_assertion(*n)) {
                    if let Some(first) = nodes.first() {
                        assertion(nav, *first, child, path, accounting, failures);
                    } else {
                        failures.push(format!(
                            "{location}: no first selected node for nested assertion"
                        ));
                    }
                }
            }
            Err(e) => failures.push(format!("{location}: expected node set: {e}")),
        }
    }
}

#[test]
fn complete_unmodified_jaxen_corpus() {
    let catalog = roxmltree::Document::parse(CATALOG).unwrap();
    let definitions: Vec<_> = catalog.descendants().filter(|n| is_assertion(*n)).collect();
    assert_eq!(
        definitions.len(),
        296,
        "catalog assertion accounting changed"
    );
    assert_eq!(definitions.iter().filter(|n| extension(**n)).count(), 19);
    assert_eq!(
        definitions
            .iter()
            .filter(|n| n.attribute("exception") == Some("true"))
            .count(),
        9
    );
    // Require every whitelist entry to correspond to a real direct assertion.
    for selector in EXTENSIONS {
        assert!(
            definitions
                .iter()
                .any(|n| n.attribute("select") == Some(*selector)),
            "stale extension whitelist entry: {selector}"
        );
    }
    let mut documents = BTreeMap::new();
    for spec in catalog.root_element().children().filter(|n| n.is_element()) {
        let path = spec.attribute("url").unwrap();
        documents.entry(path.to_owned()).or_insert_with(|| {
            let source = std::fs::read_to_string(format!(
                "{}/vendor/jaxen/{path}",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            Document::parse(&source).unwrap_or_else(|e| panic!("{path}: {e}"))
        });
    }
    let nav = Corpus {
        documents: documents.into_iter().collect(),
    };
    let mut accounting = Accounting::default();
    let mut failures = Vec::new();
    let mut contexts = 0;
    for document in catalog.root_element().children().filter(|n| n.is_element()) {
        let path = document.attribute("url").unwrap();
        for spec in document.children().filter(|n| n.is_element()) {
            assert_eq!(spec.tag_name().name(), "context");
            contexts += 1;
            let select = spec.attribute("select").unwrap();
            match XPath::compile(select)
                .and_then(|xp| xp.select(&context(&nav, nav.root(path).unwrap(), spec)))
            {
                Ok(nodes) if !nodes.is_empty() => {
                    for node in nodes {
                        for child in spec.children().filter(|n| is_assertion(*n)) {
                            assertion(&nav, node, child, path, &mut accounting, &mut failures);
                        }
                    }
                }
                Ok(_) => failures.push(format!(
                    "{path} context {select:?}: context selection is empty"
                )),
                Err(e) => failures.push(format!("{path} context {select:?}: {e}")),
            }
        }
    }
    assert_eq!(contexts, 62);
    eprintln!("Jaxen: {contexts} contexts; {} core and {} extension executions; {} expected exceptions; {} nested executions; {} failures", accounting.core, accounting.extensions, accounting.exceptions, accounting.nested, failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(
        (
            accounting.core,
            accounting.extensions,
            accounting.exceptions,
            accounting.nested
        ),
        (277, 19, 9, 15),
        "execution accounting differs from definitions"
    );
}
