//! R11: the Navigator conformance kit, run against the XML adapter (no
//! hooks), the scan-backed reference (every hook), and navigators whose hooks
//! deliberately lie.
use std::{cell::Cell, cmp::Ordering};
use xpath1::{
    conformance::{self, check_hooks, ScanHooks, DOCUMENTS},
    xml::{Document, NodeId},
    Context, ExpandedName, Navigator, NodeKind, Result, Value, XPath,
};

fn xml(source: &str) -> Result<(Document, NodeId)> {
    let document = Document::parse(source)?;
    let root = document.root();
    Ok((document, root))
}

/// Counts how often the evaluator consults each hook.
#[derive(Default)]
struct Calls {
    children_named: Cell<usize>,
    descendants_named: Cell<usize>,
    count_children_named: Cell<usize>,
    exists_child_named: Cell<usize>,
    child_by_key: Cell<usize>,
}

fn bump(cell: &Cell<usize>) {
    cell.set(cell.get() + 1);
}

/// Forwards to an inner navigator and counts hook calls.
struct Counting<'c, N> {
    inner: N,
    calls: &'c Calls,
}

impl<N: Navigator> Navigator for Counting<'_, N> {
    type Node = N::Node;
    fn validate_node(&self, node: &Self::Node) -> Result<()> {
        self.inner.validate_node(node)
    }
    fn kind(&self, node: &Self::Node) -> NodeKind {
        self.inner.kind(node)
    }
    fn parent(&self, node: &Self::Node) -> Result<Option<Self::Node>> {
        self.inner.parent(node)
    }
    fn children<'a>(
        &'a self,
        node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        self.inner.children(node)
    }
    fn attributes<'a>(
        &'a self,
        node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        self.inner.attributes(node)
    }
    fn namespaces<'a>(
        &'a self,
        node: &'a Self::Node,
    ) -> impl Iterator<Item = Result<Self::Node>> + 'a {
        self.inner.namespaces(node)
    }
    fn name(&self, node: &Self::Node) -> Result<Option<ExpandedName>> {
        self.inner.name(node)
    }
    fn qualified_name(&self, node: &Self::Node) -> Result<String> {
        self.inner.qualified_name(node)
    }
    fn string_value(&self, node: &Self::Node) -> Result<String> {
        self.inner.string_value(node)
    }
    fn compare_document_order(&self, a: &Self::Node, b: &Self::Node) -> Ordering {
        self.inner.compare_document_order(a, b)
    }
    fn name_matches(&self, node: &Self::Node, name: &ExpandedName) -> bool {
        self.inner.name_matches(node, name)
    }
    fn children_named(&self, parent: &Self::Node, name: &ExpandedName) -> Option<Vec<Self::Node>> {
        bump(&self.calls.children_named);
        self.inner.children_named(parent, name)
    }
    fn descendants_named(&self, node: &Self::Node, name: &ExpandedName) -> Option<Vec<Self::Node>> {
        bump(&self.calls.descendants_named);
        self.inner.descendants_named(node, name)
    }
    fn count_children_named(&self, parent: &Self::Node, name: &ExpandedName) -> Option<usize> {
        bump(&self.calls.count_children_named);
        self.inner.count_children_named(parent, name)
    }
    fn exists_child_named(&self, parent: &Self::Node, name: &ExpandedName) -> Option<bool> {
        bump(&self.calls.exists_child_named);
        self.inner.exists_child_named(parent, name)
    }
    fn child_by_key(
        &self,
        parent: &Self::Node,
        name: &ExpandedName,
        keys: &[(ExpandedName, Value<Self::Node>)],
    ) -> Option<Option<Self::Node>> {
        bump(&self.calls.child_by_key);
        self.inner.child_by_key(parent, name, keys)
    }
    fn element_by_id(&self, context: &Self::Node, id: &str) -> Result<Option<Self::Node>> {
        self.inner.element_by_id(context, id)
    }
}

// The reference implementation (no hooks) passes the kit, with the same
// accounting as tests/jaxen.rs.
#[test]
fn xml_adapter_passes_the_kit() {
    let report = conformance::run(xml);
    eprintln!("{report}");
    report.assert_passed();
    assert_eq!(
        (
            report.contexts,
            report.core,
            report.extensions,
            report.exceptions,
            report.nested
        ),
        (62, 277, 19, 9, 15)
    );
    assert!(report.hook_checks > 100_000, "{}", report.hook_checks);
}

// A navigator answering every hook passes the same corpus, and the corpus
// exercises every hook path of the evaluator.
#[test]
fn every_hook_answered_passes_the_kit() {
    let calls = Calls::default();
    let report = conformance::run(|source| {
        let (document, root) = xml(source)?;
        Ok((
            Counting {
                inner: ScanHooks::new(document),
                calls: &calls,
            },
            root,
        ))
    });
    eprintln!("{report}");
    report.assert_passed();
    assert_eq!(report.definitions(), 296);
    assert!(calls.child_by_key.get() > 0);
}

/// A borrowed navigator, so one document can back several wrappers.
impl<N: Navigator> Navigator for Borrow<'_, N> {
    type Node = N::Node;
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
    fn element_by_id(&self, context: &Self::Node, id: &str) -> Result<Option<Self::Node>> {
        self.0.element_by_id(context, id)
    }
}
struct Borrow<'d, N>(&'d N);

/// Expressions whose evaluation reaches every hook shape, generated from
/// the element names present under `root`.
fn matrix_expressions(document: &Document, root: NodeId) -> Vec<String> {
    let names: std::collections::BTreeSet<String> = XPath::compile("//*")
        .unwrap()
        .select(&Context::new(document, root))
        .unwrap()
        .iter()
        .filter_map(|n| document.name(n).unwrap())
        .filter(|n| n.namespace_uri.is_empty())
        .map(|n| n.local_name)
        .take(8)
        .collect();
    let mut expressions = Vec::new();
    for a in &names {
        expressions.extend([
            format!("count({a})"),
            format!("count(child::{a})"),
            format!("boolean({a})"),
            format!("not({a})"),
            format!("//{a}"),
            format!("descendant::{a}"),
            format!("descendant-or-self::{a}"),
            format!("descendant::{a}[1]"),
            format!(".//{a}"),
            format!("//{a}[1]"),
            format!("*[{a}]"),
            format!("//*[{a}]"),
            format!("count(//{a})"),
            format!("{a}[1]"),
            format!("{a}[last()]"),
            format!("//{a}/.."),
        ]);
        for b in names.iter().take(4) {
            expressions.extend([
                format!("{a} and {b}"),
                format!("{a} or {b}"),
                format!("//{a}//{b}"),
                format!("//{a}[{b}]"),
                format!("count(//{a}/{b})"),
                format!("//{a}[{b} = 'x']"),
            ]);
        }
    }
    expressions
}

fn evaluate<N: Navigator>(nav: &N, node: N::Node, expression: &str) -> String {
    match XPath::compile(expression)
        .unwrap()
        .evaluate(&Context::new(nav, node))
    {
        Ok(value) => format!("{value:?}"),
        Err(error) => format!("ERR {:?}", error.kind),
    }
}

// A navigator that answers every hook and one that declines every hook give
// byte-identical results over the fixture matrix, at every element context.
#[test]
fn hooks_and_scans_agree_on_the_fixture_matrix() {
    let calls = Calls::default();
    let mut cases = 0;
    for (path, source) in DOCUMENTS {
        let (document, root) = xml(source).unwrap();
        let hooked = Counting {
            inner: ScanHooks::new(Borrow(&document)),
            calls: &calls,
        };
        let expressions = matrix_expressions(&document, root);
        let contexts: Vec<NodeId> = std::iter::once(root)
            .chain(
                XPath::compile("//*")
                    .unwrap()
                    .select(&Context::new(&document, root))
                    .unwrap(),
            )
            .take(12)
            .collect();
        for &node in &contexts {
            for expression in &expressions {
                let plain = evaluate(&document, node, expression);
                let fast = evaluate(&hooked, node, expression);
                assert_eq!(fast, plain, "{path} {node:?} {expression}");
                cases += 1;
            }
        }
    }
    eprintln!("matrix: {cases} cases");
    assert!(cases > 20_000, "{cases}");
    // The evaluator consulted every hook shape.
    for (hook, count) in [
        ("children_named", calls.children_named.get()),
        ("descendants_named", calls.descendants_named.get()),
        ("count_children_named", calls.count_children_named.get()),
        ("exists_child_named", calls.exists_child_named.get()),
    ] {
        assert!(count > 0, "{hook} never consulted");
    }
}

// ---------------------------------------------------------------------------
// Lying hooks: the kit reports each lie.

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lie {
    ChildrenReversed,
    ChildrenDuplicated,
    DescendantsReversed,
    DescendantsWithSelf,
    CountOffByOne,
    ExistsNegated,
    KeyWrongEntry,
    KeyAmbiguous,
    NameMatchesAll,
    OrderAttributesAfterChildren,
}

struct Liar<'d> {
    honest: ScanHooks<Borrow<'d, Document>>,
    lie: Lie,
}

impl Navigator for Liar<'_> {
    type Node = NodeId;
    fn kind(&self, node: &NodeId) -> NodeKind {
        self.honest.kind(node)
    }
    fn parent(&self, node: &NodeId) -> Result<Option<NodeId>> {
        self.honest.parent(node)
    }
    fn children<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.honest.children(node)
    }
    fn attributes<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.honest.attributes(node)
    }
    fn namespaces<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.honest.namespaces(node)
    }
    fn name(&self, node: &NodeId) -> Result<Option<ExpandedName>> {
        self.honest.name(node)
    }
    fn string_value(&self, node: &NodeId) -> Result<String> {
        self.honest.string_value(node)
    }
    fn compare_document_order(&self, a: &NodeId, b: &NodeId) -> Ordering {
        let honest = self.honest.compare_document_order(a, b);
        if self.lie != Lie::OrderAttributesAfterChildren {
            return honest;
        }
        // Put an element's attributes after its children.
        let attribute = |n: &NodeId| self.kind(n) == NodeKind::Attribute;
        let owner = |n: &NodeId| self.parent(n).unwrap().unwrap();
        let descends = |n: &NodeId, ancestor: &NodeId| {
            let mut current = self.parent(n).unwrap();
            while let Some(p) = current {
                if p == *ancestor {
                    return true;
                }
                current = self.parent(&p).unwrap();
            }
            false
        };
        match (attribute(a), attribute(b)) {
            (true, false) if !attribute(b) && descends(b, &owner(a)) => Ordering::Greater,
            (false, true) if descends(a, &owner(b)) => Ordering::Less,
            _ => honest,
        }
    }
    fn name_matches(&self, node: &NodeId, name: &ExpandedName) -> bool {
        self.lie == Lie::NameMatchesAll || self.honest.name_matches(node, name)
    }
    fn children_named(&self, parent: &NodeId, name: &ExpandedName) -> Option<Vec<NodeId>> {
        let mut found = self.honest.children_named(parent, name)?;
        match self.lie {
            Lie::ChildrenReversed => found.reverse(),
            Lie::ChildrenDuplicated => {
                if let Some(first) = found.first().copied() {
                    found.push(first);
                }
            }
            _ => {}
        }
        Some(found)
    }
    fn descendants_named(&self, node: &NodeId, name: &ExpandedName) -> Option<Vec<NodeId>> {
        let mut found = self.honest.descendants_named(node, name)?;
        match self.lie {
            Lie::DescendantsReversed => found.reverse(),
            Lie::DescendantsWithSelf if self.honest.name_matches(node, name) => {
                found.insert(0, *node)
            }
            _ => {}
        }
        Some(found)
    }
    fn count_children_named(&self, parent: &NodeId, name: &ExpandedName) -> Option<usize> {
        let count = self.honest.count_children_named(parent, name)?;
        Some(if self.lie == Lie::CountOffByOne {
            count + 1
        } else {
            count
        })
    }
    fn exists_child_named(&self, parent: &NodeId, name: &ExpandedName) -> Option<bool> {
        let exists = self.honest.exists_child_named(parent, name)?;
        Some(exists != (self.lie == Lie::ExistsNegated))
    }
    fn child_by_key(
        &self,
        parent: &NodeId,
        name: &ExpandedName,
        keys: &[(ExpandedName, Value<NodeId>)],
    ) -> Option<Option<NodeId>> {
        let entries = self.honest.children_named(parent, name)?;
        match self.lie {
            // Answer with the last entry whenever there is more than one.
            Lie::KeyWrongEntry if entries.len() > 1 => Some(entries.last().copied()),
            // Answer with the first match even when several match.
            Lie::KeyAmbiguous => {
                let first = entries.iter().find(|entry| {
                    keys.iter().all(|(key, value)| {
                        let Value::String(value) = value else {
                            return false;
                        };
                        self.honest
                            .children_named(entry, key)
                            .unwrap()
                            .iter()
                            .any(|leaf| self.string_value(leaf).unwrap() == *value)
                    })
                });
                Some(first.copied())
            }
            _ => self.honest.child_by_key(parent, name, keys),
        }
    }
}

const LIAR_FIXTURE: &str = "<r xmlns:p='urn:p'>\
    <list><e><v>a</v><k>1</k></e><e><v>a</v><k>2</k></e><e><v>b</v><k>3</k></e></list>\
    <x a='1' b='2'><x a='3'><y/></x><y/><z/></x>\
    <p:x/><x><y/><y/></x></r>";

#[test]
fn the_kit_reports_each_lie() {
    let document = Document::parse(LIAR_FIXTURE).unwrap();
    let honest = check_hooks(&ScanHooks::new(Borrow(&document)), &document.root());
    assert!(honest.passed(), "{honest}");
    for (lie, hook) in [
        (Lie::ChildrenReversed, "children_named"),
        (Lie::ChildrenDuplicated, "children_named"),
        (Lie::DescendantsReversed, "descendants_named"),
        (Lie::DescendantsWithSelf, "descendants_named"),
        (Lie::CountOffByOne, "count_children_named"),
        (Lie::ExistsNegated, "exists_child_named"),
        (Lie::KeyWrongEntry, "child_by_key"),
        (Lie::KeyAmbiguous, "child_by_key"),
        (Lie::NameMatchesAll, "name_matches"),
        (Lie::OrderAttributesAfterChildren, "compare_document_order"),
    ] {
        let liar = Liar {
            honest: ScanHooks::new(Borrow(&document)),
            lie,
        };
        let report = check_hooks(&liar, &document.root());
        assert!(!report.passed(), "{lie:?} was not reported");
        assert!(
            report.violations.iter().all(|v| v.starts_with(hook)),
            "{lie:?}: {report}"
        );
    }
    // Lies the corpus exercises also fail corpus assertions, besides the
    // matrix. (Order and duplicates in a step's result are normalized away;
    // a reversed hook answer shows through positional predicates. The corpus
    // has no `count(name)` or bare `[name]` at an element focus, so only the
    // matrix sees count and existence lies.)
    for lie in [Lie::ChildrenReversed, Lie::NameMatchesAll] {
        let report = conformance::run(|source| {
            let document: &'static Document = Box::leak(Box::new(Document::parse(source)?));
            Ok((
                Liar {
                    honest: ScanHooks::new(Borrow(document)),
                    lie,
                },
                document.root(),
            ))
        });
        assert!(!report.violations.is_empty(), "{lie:?}");
        assert!(
            !report.failures.is_empty(),
            "{lie:?}: the corpus notices too"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: random trees and names.

struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

fn random_tree(rng: &mut Rng) -> String {
    const NAMES: [&str; 5] = ["a", "b", "c", "d", "e"];
    fn element(rng: &mut Rng, depth: usize, out: &mut String) {
        let name = NAMES[rng.next(NAMES.len())];
        out.push('<');
        out.push_str(name);
        if rng.next(3) == 0 {
            out.push_str(&format!(" k='{}'", rng.next(3)));
        }
        out.push('>');
        if rng.next(2) == 0 {
            out.push_str(&format!("<k>{}</k>", rng.next(3)));
        }
        let children = if depth >= 5 { 0 } else { rng.next(5) };
        for _ in 0..children {
            match rng.next(6) {
                0 => out.push_str(&format!("t{}", rng.next(3))),
                1 => out.push_str("<!--c-->"),
                _ => element(rng, depth + 1, out),
            }
        }
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    }
    let mut out = String::new();
    element(rng, 0, &mut out);
    out
}

#[test]
fn random_trees_hooks_agree_with_scans() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for round in 0..200 {
        let source = random_tree(&mut rng);
        let document = Document::parse(&source).unwrap();
        let root = document.root();
        let hooked = ScanHooks::new(Borrow(&document));
        let report = check_hooks(&hooked, &root);
        assert!(report.passed(), "{source}\n{report}");
        let all = XPath::compile("/descendant-or-self::node()")
            .unwrap()
            .select(&Context::new(&document, root))
            .unwrap();
        for _ in 0..20 {
            let node = all[rng.next(all.len())];
            let name = ExpandedName::local(["a", "b", "c", "d", "e", "q"][rng.next(6)]);
            let children = hooked.children_named(&node, &name).unwrap();
            assert_eq!(
                hooked.count_children_named(&node, &name),
                Some(children.len())
            );
            assert_eq!(
                hooked.exists_child_named(&node, &name),
                Some(!children.is_empty())
            );
            let local = &name.local_name;
            for expression in [
                format!("count({local})"),
                format!("boolean({local})"),
                format!("not({local}) or {local}"),
                format!("//{local}"),
                format!("descendant-or-self::{local}"),
                format!("*[{local}]"),
                format!("{local}[k = '1']"),
                format!("//{local}[k = 1]/{local}"),
            ] {
                assert_eq!(
                    evaluate(&hooked, node, &expression),
                    evaluate(&document, node, &expression),
                    "round {round}: {source} at {node:?}: {expression}"
                );
            }
        }
    }
}

#[test]
#[ignore = "diagnostic: which lies the Jaxen corpus alone detects"]
fn corpus_sensitivity_to_lies() {
    for lie in [
        Lie::ChildrenReversed,
        Lie::ChildrenDuplicated,
        Lie::DescendantsReversed,
        Lie::DescendantsWithSelf,
        Lie::CountOffByOne,
        Lie::ExistsNegated,
        Lie::KeyWrongEntry,
        Lie::KeyAmbiguous,
        Lie::NameMatchesAll,
        Lie::OrderAttributesAfterChildren,
    ] {
        let report = conformance::run(|source| {
            let document: &'static Document = Box::leak(Box::new(Document::parse(source)?));
            Ok((
                Liar {
                    honest: ScanHooks::new(Borrow(document)),
                    lie,
                },
                document.root(),
            ))
        });
        eprintln!(
            "{lie:?}: {} corpus failures, {} matrix violations",
            report.failures.len(),
            report.violations.len()
        );
    }
}
