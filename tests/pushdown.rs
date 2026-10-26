//! R8 (descendant name pushdown) and R9 (count and existence pushdown):
//! which shapes reach which hook, results equal to a scan, fallbacks, and
//! budget refunds on a declined attempt.
use std::{cell::Cell, cmp::Ordering};
use xpath1::{
    conformance::ScanHooks,
    xml::{Document, NodeId},
    Context, ErrorKind, ExpandedName, Limits, Navigator, NodeKind, NodeSetPool, Result, Value,
    XPath,
};

/// A borrowed document, so several wrappers can share one.
struct Borrow<'d>(&'d Document);

impl Navigator for Borrow<'_> {
    type Node = NodeId;
    fn validate_node(&self, node: &NodeId) -> Result<()> {
        self.0.validate_node(node)
    }
    fn kind(&self, node: &NodeId) -> NodeKind {
        self.0.kind(node)
    }
    fn parent(&self, node: &NodeId) -> Result<Option<NodeId>> {
        self.0.parent(node)
    }
    fn children<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.0.children(node)
    }
    fn attributes<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.0.attributes(node)
    }
    fn namespaces<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.0.namespaces(node)
    }
    fn name(&self, node: &NodeId) -> Result<Option<ExpandedName>> {
        self.0.name(node)
    }
    fn name_matches(&self, node: &NodeId, name: &ExpandedName) -> bool {
        self.0.name_matches(node, name)
    }
    fn qualified_name(&self, node: &NodeId) -> Result<String> {
        self.0.qualified_name(node)
    }
    fn string_value(&self, node: &NodeId) -> Result<String> {
        self.0.string_value(node)
    }
    fn compare_document_order(&self, a: &NodeId, b: &NodeId) -> Ordering {
        self.0.compare_document_order(a, b)
    }
}

#[derive(Clone, Copy, Default)]
struct Answer {
    children: bool,
    descendants: bool,
    count: bool,
    exists: bool,
}
const ALL: Answer = Answer {
    children: true,
    descendants: true,
    count: true,
    exists: true,
};
const NONE: Answer = Answer {
    children: false,
    descendants: false,
    count: false,
    exists: false,
};

/// Answers the selected hooks (from scans), declines the others, and counts
/// both hook calls and the evaluator's own `children` scans.
struct Hooks<'d> {
    inner: ScanHooks<Borrow<'d>>,
    answer: Answer,
    /// Decline `descendants_named` at an element with this local name whose
    /// parent has it too.
    decline_nested: Option<&'static str>,
    children: Cell<usize>,
    descendants: Cell<usize>,
    count: Cell<usize>,
    exists: Cell<usize>,
}

impl<'d> Hooks<'d> {
    fn new(document: &'d Document, answer: Answer) -> Self {
        Self {
            inner: ScanHooks::new(Borrow(document)),
            answer,
            decline_nested: None,
            children: Cell::new(0),
            descendants: Cell::new(0),
            count: Cell::new(0),
            exists: Cell::new(0),
        }
    }
    fn calls(&self) -> [usize; 4] {
        [
            self.children.get(),
            self.descendants.get(),
            self.count.get(),
            self.exists.get(),
        ]
    }
}

fn bump(cell: &Cell<usize>) {
    cell.set(cell.get() + 1);
}

impl Navigator for Hooks<'_> {
    type Node = NodeId;
    fn validate_node(&self, node: &NodeId) -> Result<()> {
        self.inner.validate_node(node)
    }
    fn kind(&self, node: &NodeId) -> NodeKind {
        self.inner.kind(node)
    }
    fn parent(&self, node: &NodeId) -> Result<Option<NodeId>> {
        self.inner.parent(node)
    }
    fn children<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        bump(&self.children);
        self.inner.children(node)
    }
    fn attributes<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.inner.attributes(node)
    }
    fn namespaces<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.inner.namespaces(node)
    }
    fn name(&self, node: &NodeId) -> Result<Option<ExpandedName>> {
        self.inner.name(node)
    }
    fn name_matches(&self, node: &NodeId, name: &ExpandedName) -> bool {
        self.inner.name_matches(node, name)
    }
    fn qualified_name(&self, node: &NodeId) -> Result<String> {
        self.inner.qualified_name(node)
    }
    fn string_value(&self, node: &NodeId) -> Result<String> {
        self.inner.string_value(node)
    }
    fn compare_document_order(&self, a: &NodeId, b: &NodeId) -> Ordering {
        self.inner.compare_document_order(a, b)
    }
    fn children_named(&self, parent: &NodeId, name: &ExpandedName) -> Option<Vec<NodeId>> {
        self.answer
            .children
            .then(|| self.inner.children_named(parent, name))?
    }
    fn descendants_named(&self, node: &NodeId, name: &ExpandedName) -> Option<Vec<NodeId>> {
        bump(&self.descendants);
        let named = |node: &NodeId, local: &str| {
            self.name(node)
                .unwrap()
                .is_some_and(|n| n.local_name == local)
        };
        let declined = self.decline_nested.is_some_and(|local| {
            named(node, local) && named(&self.parent(node).unwrap().unwrap(), local)
        });
        if !self.answer.descendants || declined {
            return None;
        }
        self.inner.descendants_named(node, name)
    }
    fn count_children_named(&self, parent: &NodeId, name: &ExpandedName) -> Option<usize> {
        bump(&self.count);
        self.answer
            .count
            .then(|| self.inner.count_children_named(parent, name))?
    }
    fn exists_child_named(&self, parent: &NodeId, name: &ExpandedName) -> Option<bool> {
        bump(&self.exists);
        self.answer
            .exists
            .then(|| self.inner.exists_child_named(parent, name))?
    }
}

const FIXTURE: &str = "<a>\
    <b><d>1</d><c/><d>2</d><b><d>3</d></b></b>\
    <b><c/><x:d xmlns:x='urn:x'/></b>\
    <d>4<d>5</d></d>\
    <e>text<!--c--></e>\
    </a>";

fn evaluate<N: Navigator>(nav: &N, node: N::Node, expression: &str) -> Result<Value<N::Node>> {
    XPath::compile(expression)
        .unwrap()
        .evaluate(&Context::new(nav, node))
}

fn same<N: Navigator<Node = NodeId>>(nav: &N, plain: &Document, node: NodeId, expression: &str) {
    let got = evaluate(nav, node, expression);
    let want = evaluate(plain, node, expression);
    match (&got, &want) {
        (Ok(got), Ok(want)) => assert_eq!(got, want, "{expression}"),
        (Err(got), Err(want)) => assert_eq!(got.kind, want.kind, "{expression}"),
        _ => panic!("{expression}: {got:?} vs {want:?}"),
    }
}

/// Every node of the document, in document order.
fn nodes(document: &Document) -> Vec<NodeId> {
    XPath::compile("/descendant-or-self::node() | //@* | //namespace::*")
        .unwrap()
        .select(&Context::new(document, document.root()))
        .unwrap()
}

// R8: descendant and descendant-or-self name steps, and `//name`, ask
// descendants_named and scan nothing when it answers.
#[test]
fn descendants_named_answers_descendant_steps() {
    let document = Document::parse(FIXTURE).unwrap();
    let root = document.root();
    for expression in [
        "//d",
        "/descendant::d",
        "/descendant-or-self::d",
        "/a//d",
        "//b//d",
        "/descendant::b/descendant-or-self::b",
    ] {
        let nav = Hooks::new(&document, ALL);
        same(&nav, &document, root, expression);
        assert!(nav.descendants.get() > 0, "{expression}");
        assert_eq!(nav.children.get(), 0, "{expression}: scanned children");
    }
    assert_eq!(
        evaluate(&Hooks::new(&document, ALL), root, "count(//d)").unwrap(),
        Value::Number(5.0)
    );
    // descendant-or-self includes a matching context node first.
    let b = evaluate(&document, root, "/a/b[1]")
        .unwrap()
        .as_nodes()
        .unwrap()[0];
    let nav = Hooks::new(&document, ALL);
    let got = evaluate(&nav, b, "descendant-or-self::b").unwrap();
    assert_eq!(got.as_nodes().unwrap().len(), 2);
    assert_eq!(got.as_nodes().unwrap()[0], b);
    same(&nav, &document, b, "descendant-or-self::b");
    same(&nav, &document, b, "descendant::b");
    // Every shape, from every node, agrees with the scan, with and without
    // the hook, also where the hook is not asked (non-element contexts).
    let all = nodes(&document);
    for answer in [ALL, NONE] {
        for &node in &all {
            for expression in [
                "descendant::d",
                "descendant-or-self::d",
                "descendant::d[2]",
                "descendant::d[last()]",
                "descendant-or-self::b[d]",
                ".//d",
                "//d[1]",
                "//d/d",
                "//x:d",
                "//c | //d",
                "count(.//d)",
            ] {
                let nav = Hooks::new(&document, answer);
                let ctx = Context::new(&nav, node).with_namespace("x", "urn:x");
                let plain = Context::new(&document, node).with_namespace("x", "urn:x");
                let compiled = XPath::compile(expression).unwrap();
                assert_eq!(
                    compiled.evaluate(&ctx).unwrap(),
                    compiled.evaluate(&plain).unwrap(),
                    "{node:?} {expression}"
                );
            }
        }
    }
}

// R8: shapes outside the family never reach the hook.
#[test]
fn descendants_named_is_not_asked_for_other_shapes() {
    let document = Document::parse(FIXTURE).unwrap();
    let root = document.root();
    for expression in [
        "//*",
        "//node()",
        "//text()",
        "/descendant::*",
        "/descendant::node()",
        "/a/b/d",
        "//@d",
        "/descendant-or-self::node()/self::d",
        "/descendant-or-self::node()[1]/child::d",
    ] {
        let nav = Hooks::new(&document, ALL);
        same(&nav, &document, root, expression);
        assert_eq!(nav.descendants.get(), 0, "{expression}");
    }
    // Only root and element contexts are asked: from text, comment,
    // attribute and namespace nodes the axis is empty and scanned.
    let nav = Hooks::new(&document, ALL);
    for node in nodes(&document) {
        if !matches!(document.kind(&node), NodeKind::Root | NodeKind::Element) {
            assert_eq!(
                evaluate(&nav, node, "descendant::d").unwrap(),
                Value::Nodes(vec![])
            );
        }
    }
    assert_eq!(nav.descendants.get(), 0);
}

// R8: a hook answer drops non-element nodes, as the principal node type
// requires, and its vector goes back to the pool.
#[test]
fn descendants_named_keeps_elements_and_pools_vectors() {
    struct Noisy<'d>(Hooks<'d>);
    impl Navigator for Noisy<'_> {
        type Node = NodeId;
        fn kind(&self, node: &NodeId) -> NodeKind {
            self.0.kind(node)
        }
        fn parent(&self, node: &NodeId) -> Result<Option<NodeId>> {
            self.0.parent(node)
        }
        fn children<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
            self.0.children(node)
        }
        fn name(&self, node: &NodeId) -> Result<Option<ExpandedName>> {
            self.0.name(node)
        }
        fn string_value(&self, node: &NodeId) -> Result<String> {
            self.0.string_value(node)
        }
        fn compare_document_order(&self, a: &NodeId, b: &NodeId) -> Ordering {
            self.0.compare_document_order(a, b)
        }
        fn descendants_named(&self, node: &NodeId, name: &ExpandedName) -> Option<Vec<NodeId>> {
            // A text node slipped in: the evaluator must drop it.
            let mut found = self.0.descendants_named(node, name)?;
            let text = self
                .0
                .inner
                .0
                 .0
                .children(node)
                .flatten()
                .find(|n| self.0.kind(n) == NodeKind::Text);
            found.extend(text);
            Some(found)
        }
    }
    let document = Document::parse("<a>t<d/><d/></a>").unwrap();
    let a = evaluate(&document, document.root(), "/a")
        .unwrap()
        .as_nodes()
        .unwrap()[0];
    let nav = Noisy(Hooks::new(&document, ALL));
    assert_eq!(
        evaluate(&nav, a, "descendant::d").unwrap(),
        evaluate(&document, a, "descendant::d").unwrap()
    );
    let pool = NodeSetPool::new();
    let ctx = Context::new(&nav, a).with_pool(&pool);
    let compiled = XPath::compile("count(descendant::d)").unwrap();
    assert_eq!(compiled.evaluate(&ctx).unwrap(), Value::Number(2.0));
    assert!(!pool.is_empty(), "hook vectors are given to the pool");
}

// R9: count(name) asks count_children_named; other count shapes do not.
#[test]
fn count_children_named_answers_count_of_child_name() {
    let document = Document::parse(FIXTURE).unwrap();
    let a = evaluate(&document, document.root(), "/a")
        .unwrap()
        .as_nodes()
        .unwrap()[0];
    for expression in ["count(b)", "count(child::b)", "count(d)", "count(q)"] {
        let nav = Hooks::new(&document, ALL);
        same(&nav, &document, a, expression);
        assert_eq!(nav.count.get(), 1, "{expression}");
        assert_eq!(nav.children.get(), 0, "{expression}: scanned children");
    }
    for expression in [
        "count(b[1])",
        "count(b/d)",
        "count(./b)",
        "count(/a)",
        "count(//b)",
        "count(@b)",
        "count(*)",
        "count(b | d)",
        "count(descendant::b)",
        "count(self::a)",
    ] {
        let nav = Hooks::new(&document, ALL);
        same(&nav, &document, a, expression);
        assert_eq!(nav.count.get(), 0, "{expression}");
    }
    // The root and every element are asked; other nodes are not, and their
    // child axis is empty.
    let nav = Hooks::new(&document, ALL);
    let all = nodes(&document);
    for &node in &all {
        same(&nav, &document, node, "count(d)");
        same(&nav, &document, node, "count(b) + count(c) * 10");
    }
    let containers = all
        .iter()
        .filter(|n| matches!(document.kind(n), NodeKind::Root | NodeKind::Element))
        .count();
    assert_eq!(nav.count.get(), 3 * containers);
    // count() with the wrong arity is still an arity error.
    let nav = Hooks::new(&document, ALL);
    assert_eq!(
        evaluate(&nav, a, "count(b, c)").unwrap_err().kind,
        ErrorKind::Arity
    );
    assert_eq!(
        evaluate(&nav, a, "count()").unwrap_err().kind,
        ErrorKind::Arity
    );
}

// R9: a bare `name` in boolean position asks exists_child_named.
#[test]
fn exists_child_named_answers_boolean_positions() {
    let document = Document::parse(FIXTURE).unwrap();
    let a = evaluate(&document, document.root(), "/a")
        .unwrap()
        .as_nodes()
        .unwrap()[0];
    for (expression, asked) in [
        ("boolean(b)", 1),
        ("not(b)", 1),
        ("not(q)", 1),
        ("b and c", 2),
        ("q and b", 1),
        ("b and q", 2),
        ("q or b", 2),
        ("b or q", 1),
        ("b[d]", 2),
        ("b[c][d]", 2 + 2),
        ("b[c and d]", 2 + 2),
        ("boolean(child::q)", 1),
    ] {
        let nav = Hooks::new(&document, ALL);
        same(&nav, &document, a, expression);
        assert_eq!(nav.exists.get(), asked, "{expression}");
        assert_eq!(nav.children.get(), 0, "{expression}: scanned children");
    }
    for expression in [
        "b",
        "string(b)",
        "b = 'x'",
        "number(b)",
        "*[1]",
        "*[d = '1']",
        "*[d/text()]",
        "*[./d]",
        "*[d[1]]",
        "boolean(b/d)",
        "boolean(@x)",
        "not(/a)",
        "boolean(b, c)",
        "b | c",
        "count(b) and 1",
    ] {
        let nav = Hooks::new(&document, ALL);
        same(&nav, &document, a, expression);
        assert_eq!(nav.exists.get(), 0, "{expression}");
    }
    // Predicates at every node, with and without answers.
    for answer in [ALL, NONE] {
        let nav = Hooks::new(&document, answer);
        for node in nodes(&document) {
            for expression in ["*[d]", "self::*[c or d]", "not(e) and boolean(b)"] {
                same(&nav, &document, node, expression);
            }
        }
    }
}

// A name with an unbound prefix is reported exactly as before; the hooks
// are not asked.
#[test]
fn unbound_prefixes_are_still_errors() {
    let document = Document::parse(FIXTURE).unwrap();
    let nav = Hooks::new(&document, ALL);
    for expression in [
        "count(p:b)",
        "boolean(p:b)",
        "*[p:b]",
        "//p:d",
        "descendant::p:d",
    ] {
        assert_eq!(
            evaluate(&nav, document.root(), expression)
                .unwrap_err()
                .kind,
            ErrorKind::Namespace,
            "{expression}"
        );
    }
    assert_eq!(nav.calls()[1..], [0, 0, 0]);
}

// The depth limit applies as before: a pushed argument still counts as one
// level of nesting.
#[test]
fn depth_limit_is_unchanged() {
    let document = Document::parse(FIXTURE).unwrap();
    let a = evaluate(&document, document.root(), "/a")
        .unwrap()
        .as_nodes()
        .unwrap()[0];
    for expression in ["count(b)", "boolean(b)", "b and c", "*[d]"] {
        for depth in 1..5 {
            let limits = Limits {
                max_depth: depth,
                ..Limits::default()
            };
            let nav = Hooks::new(&document, ALL);
            let compiled = XPath::compile(expression).unwrap();
            let got = compiled.evaluate(&Context::new(&nav, a).with_limits(limits));
            let want = compiled.evaluate(&Context::new(&document, a).with_limits(limits));
            match (got, want) {
                (Ok(got), Ok(want)) => assert_eq!(got, want),
                (Err(got), Err(want)) => assert_eq!(got.kind, want.kind),
                (got, want) => panic!("{expression} depth {depth}: {got:?} vs {want:?}"),
            }
        }
    }
}

/// The smallest `max_operations` at which `expression` evaluates.
fn minimum_budget<N: Navigator>(nav: &N, node: N::Node, expression: &str) -> usize {
    let compiled = XPath::compile(expression).unwrap();
    let fits = |budget| {
        let limits = Limits {
            max_operations: budget,
            ..Limits::default()
        };
        compiled
            .evaluate(&Context::new(nav, node.clone()).with_limits(limits))
            .is_ok()
    };
    assert!(fits(1_000_000), "{expression}");
    let (mut low, mut high) = (0, 1_000_000);
    while low + 1 < high {
        let middle = (low + high) / 2;
        if fits(middle) {
            high = middle;
        } else {
            low = middle;
        }
    }
    high
}

// A declined attempt leaves no trace on the work budget: a navigator that is
// asked and declines needs exactly the budget of one without hooks, and a
// `//name` attempt declined part-way through is refunded.
#[test]
fn declined_hooks_refund_the_budget() {
    let document = Document::parse(FIXTURE).unwrap();
    let root = document.root();
    let a = evaluate(&document, root, "/a").unwrap().as_nodes().unwrap()[0];
    let declining = Hooks::new(&document, NONE);
    for (node, expression) in [
        (a, "count(b)"),
        (a, "boolean(b) and not(q)"),
        (a, "b or c"),
        (a, "*[d]"),
        (root, "//d"),
        (root, "/descendant::d"),
        (root, "/descendant-or-self::b"),
        (a, "count(descendant::d) + count(d)"),
    ] {
        let before = declining.calls();
        assert_eq!(
            minimum_budget(&declining, node, expression),
            minimum_budget(&document, node, expression),
            "{expression}"
        );
        assert_ne!(declining.calls(), before, "{expression}: no hook was asked");
    }
    // `(/a/b | /a/b/b)//d`: the `//` asks each `b` in turn, is answered for
    // the first and declined for the nested one, so both steps are then
    // evaluated normally.
    let mut partial = Hooks::new(
        &document,
        Answer {
            descendants: true,
            ..NONE
        },
    );
    partial.decline_nested = Some("b");
    let nested = evaluate(&document, root, "//b[b]/b").unwrap();
    assert_eq!(nested.as_nodes().unwrap().len(), 1);
    for expression in [
        "(/a/b | /a/b/b)//d",
        "(/a/b | /a/b/b)/descendant-or-self::node()/d",
        "/descendant-or-self::node()[self::b]//d",
    ] {
        let before = partial.descendants.get();
        assert_eq!(
            minimum_budget(&partial, root, expression),
            minimum_budget(&document, root, expression),
            "{expression}"
        );
        assert!(partial.descendants.get() > before);
        same(&partial, &document, root, expression);
    }
    // An answered hook does less work than the scan it replaces.
    let answering = Hooks::new(&document, ALL);
    for (node, expression) in [(a, "count(b)"), (a, "*[d]"), (root, "//d")] {
        assert!(
            minimum_budget(&answering, node, expression)
                < minimum_budget(&document, node, expression),
            "{expression}"
        );
    }
}

// A hook answer is charged: an answer larger than the remaining budget is
// a work-limit error, as the scan would be.
#[test]
fn hook_answers_are_metered() {
    let mut source = String::from("<a>");
    for _ in 0..500 {
        source.push_str("<d/>");
    }
    source.push_str("</a>");
    let document = Document::parse(&source).unwrap();
    let limits = Limits {
        max_operations: 100,
        ..Limits::default()
    };
    let nav = Hooks::new(&document, ALL);
    for expression in ["//d", "/descendant::d"] {
        let error = XPath::compile(expression)
            .unwrap()
            .evaluate(&Context::new(&nav, document.root()).with_limits(limits))
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Limit, "{expression}");
    }
    // count() answers in one unit whatever the count.
    let a = evaluate(&document, document.root(), "/a")
        .unwrap()
        .as_nodes()
        .unwrap()[0];
    assert_eq!(
        XPath::compile("count(d)")
            .unwrap()
            .evaluate(&Context::new(&nav, a).with_limits(limits))
            .unwrap(),
        Value::Number(500.0)
    );
}
