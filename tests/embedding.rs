#![cfg(feature = "xml")]
use xpath1::{xml::Document, Context, Value, XPath};

#[test]
fn yang_namespace_option_only_changes_unprefixed_element_tests() {
    let doc = Document::parse("<m:root xmlns:m='urn:module'><m:leaf flag='on'/></m:root>").unwrap();
    let ctx = Context::new(&doc, doc.root())
        .with_default_element_namespace("urn:module")
        .with_namespace("alias", "urn:module")
        .with_variable("expected", Value::Number(1.0));
    for expression in [
        "count(/root/leaf/@flag) = $expected",
        "count(/alias:root/leaf/@alias:flag) = 0",
        "count(/root/leaf/namespace::xml) = 1",
        "name(/root) = 'm:root'",
    ] {
        assert_eq!(
            XPath::compile(expression).unwrap().evaluate(&ctx).unwrap(),
            Value::Boolean(true),
            "{expression}"
        );
    }
}

// (f) R1: the compiled AST is readable for install-time static resolution.
#[test]
fn ast_exposes_steps_for_static_resolution() {
    use xpath1::ast::{Expr, NodeTest, Step};
    use xpath1::Axis;

    fn collect(expr: &Expr, out: &mut Vec<(Axis, String)>) {
        match expr {
            Expr::Path { base, steps, .. } => {
                if let Some(base) = base {
                    collect(base, out);
                }
                for Step {
                    axis,
                    test,
                    predicates,
                    ..
                } in steps
                {
                    if let NodeTest::Name(name) = test {
                        out.push((*axis, name.local.clone()));
                    }
                    for predicate in predicates {
                        collect(predicate, out);
                    }
                }
            }
            Expr::Filter { base, predicates } => {
                collect(base, out);
                predicates.iter().for_each(|p| collect(p, out));
            }
            Expr::Binary(_, a, b) => {
                collect(a, out);
                collect(b, out);
            }
            Expr::Unary(a) => collect(a, out),
            Expr::Function(_, args) => args.iter().for_each(|a| collect(a, out)),
            Expr::String(_) | Expr::Number(_) | Expr::Variable(_) => {}
        }
    }

    let xpath = XPath::compile("a/b[@c]").unwrap();
    let mut names = Vec::new();
    collect(xpath.ast(), &mut names);
    assert_eq!(
        names,
        vec![
            (Axis::Child, "a".to_string()),
            (Axis::Child, "b".to_string()),
            (Axis::Attribute, "c".to_string()),
        ]
    );
    let Expr::Path {
        base: None,
        absolute: false,
        steps,
    } = xpath.ast()
    else {
        panic!("expected a relative path: {:?}", xpath.ast());
    };
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1].predicates.len(), 1);
}

// ---------------------------------------------------------------------------
// A counting navigator over an application-owned list, for the D21 hooks.
//
// Tree: 0 root / 1 <a> / entries <b> each with key child <c> and a <d>.
// Entry i (0-based) has handles base = 2 + 3*i: b = base, c = base+1, d = base+2.
// c's value is KEYS[i]; d's value is "d{i}".
mod counting {
    use std::cell::{Cell, RefCell};
    use std::cmp::Ordering;
    use xpath1::{ExpandedName, Navigator, NodeKind, Result, Value};

    pub const KEYS: [&str; 4] = ["w", "x", "y", "z"];

    #[derive(Default)]
    pub struct Calls {
        pub children: Cell<usize>,
        pub name_matches: Cell<usize>,
        pub children_named: Cell<usize>,
        pub child_by_key: Cell<usize>,
    }

    pub struct Rows {
        pub calls: Calls,
        /// Hooks the adapter answers; when false the default (None) is used.
        pub keyed: bool,
        pub named: bool,
        /// The key arguments of the last child_by_key call, verbatim.
        pub seen: RefCell<Vec<(ExpandedName, Value<usize>)>>,
        /// Canonicalization the hook applies to string keys (models a YANG type).
        pub canonical: fn(&str) -> String,
    }

    impl Rows {
        pub fn new(keyed: bool, named: bool) -> Self {
            Self {
                calls: Calls::default(),
                keyed,
                named,
                seen: RefCell::new(Vec::new()),
                canonical: |s| s.to_string(),
            }
        }
        fn entries(&self) -> usize {
            KEYS.len()
        }
        fn local(&self, n: usize) -> Option<&'static str> {
            match n {
                0 => None,
                1 => Some("a"),
                n => Some(["b", "c", "d"][(n - 2) % 3]),
            }
        }
    }

    impl Navigator for Rows {
        type Node = usize;
        fn kind(&self, n: &usize) -> NodeKind {
            if *n == 0 {
                NodeKind::Root
            } else {
                NodeKind::Element
            }
        }
        fn parent(&self, n: &usize) -> Result<Option<usize>> {
            Ok(match *n {
                0 => None,
                1 => Some(0),
                n if (n - 2) % 3 == 0 => Some(1),
                n => Some(n - (n - 2) % 3),
            })
        }
        fn children<'a>(&'a self, n: &'a usize) -> impl Iterator<Item = Result<usize>> + 'a {
            self.calls.children.set(self.calls.children.get() + 1);
            let nodes: Vec<usize> = match *n {
                0 => vec![1],
                1 => (0..self.entries()).map(|i| 2 + 3 * i).collect(),
                n if (n - 2) % 3 == 0 => vec![n + 1, n + 2],
                _ => vec![],
            };
            nodes.into_iter().map(Ok)
        }
        fn name(&self, n: &usize) -> Result<Option<ExpandedName>> {
            Ok(self.local(*n).map(ExpandedName::local))
        }
        fn name_matches(&self, n: &usize, name: &ExpandedName) -> bool {
            self.calls
                .name_matches
                .set(self.calls.name_matches.get() + 1);
            name.namespace_uri.is_empty() && self.local(*n) == Some(name.local_name.as_str())
        }
        fn children_named(&self, parent: &usize, name: &ExpandedName) -> Option<Vec<usize>> {
            self.calls
                .children_named
                .set(self.calls.children_named.get() + 1);
            if !self.named {
                return None;
            }
            let wanted = |n: &usize| {
                name.namespace_uri.is_empty() && self.local(*n) == Some(name.local_name.as_str())
            };
            Some(match *parent {
                0 => vec![1].into_iter().filter(wanted).collect(),
                1 => (0..self.entries())
                    .map(|i| 2 + 3 * i)
                    .filter(wanted)
                    .collect(),
                n if (n - 2) % 3 == 0 => vec![n + 1, n + 2].into_iter().filter(wanted).collect(),
                _ => vec![],
            })
        }
        fn child_by_key(
            &self,
            parent: &usize,
            name: &ExpandedName,
            keys: &[(ExpandedName, Value<usize>)],
        ) -> Option<Option<usize>> {
            self.calls
                .child_by_key
                .set(self.calls.child_by_key.get() + 1);
            *self.seen.borrow_mut() = keys.to_vec();
            if !self.keyed || *parent != 1 || name.local_name != "b" {
                return None;
            }
            let [(key, Value::String(wanted))] = keys else {
                return None;
            };
            if key.local_name != "c" {
                return None;
            }
            let wanted = (self.canonical)(wanted);
            Some(KEYS.iter().position(|k| *k == wanted).map(|i| 2 + 3 * i))
        }
        fn string_value(&self, n: &usize) -> Result<String> {
            Ok(match *n {
                0 | 1 => String::new(),
                n => match (n - 2) % 3 {
                    1 => KEYS[(n - 2) / 3].to_string(),
                    2 => format!("d{}", (n - 2) / 3),
                    _ => String::new(),
                },
            })
        }
        fn compare_document_order(&self, a: &usize, b: &usize) -> Ordering {
            a.cmp(b)
        }
    }
}

fn select<N: xpath1::Navigator>(nav: &N, node: N::Node, expr: &str) -> Vec<N::Node> {
    XPath::compile(expr)
        .unwrap()
        .select(&Context::new(nav, node))
        .unwrap()
}

// (a) R2: `a/b[c = 'x']` is one keyed lookup, with the same node set as a scan.
#[test]
fn keyed_lookup_answers_list_key_predicate() {
    use counting::Rows;
    let keyed = Rows::new(true, false);
    let plain = Rows::new(false, false);
    for expr in [
        "a/b[c = 'x']",
        "a/b['x' = c]",
        "a/child::b[child::c = 'x']",
        "a/b[c = 'x']/d",
        "a/b[c = 'x'][d = 'd1']",
        "a/b[c = 'x'][1]",
        "a/b[c = 'x'][2]",
        "a/b[c = 'nope']",
    ] {
        let before = keyed.calls.child_by_key.get();
        let fast = select(&keyed, 0, expr);
        let slow = select(&plain, 0, expr);
        assert_eq!(fast, slow, "{expr}");
        assert_eq!(keyed.calls.child_by_key.get() - before, 1, "{expr}");
    }
    assert_eq!(select(&keyed, 0, "a/b[c = 'x']"), vec![5]);
    assert_eq!(select(&keyed, 0, "a/b[c = 'x']/d"), vec![7]);
    assert!(select(&keyed, 0, "a/b[c = 'nope']").is_empty());

    // The keyed lookup does not enumerate the list: only `a` is scanned for
    // (root's children), never a's children.
    let fresh = Rows::new(true, false);
    assert_eq!(select(&fresh, 0, "a/b[c = 'x']"), vec![5]);
    assert_eq!(fresh.calls.children.get(), 1);
    assert_eq!(fresh.calls.child_by_key.get(), 1);
    assert_eq!(fresh.calls.name_matches.get(), 1);

    // Shapes outside the leading-equality run never reach the hook.
    for expr in [
        "a/b[1][c = 'x']",
        "a/b[position() = 2]",
        "a/b[c != 'x']",
        "a/b[c/text() = 'x']",
        "a/b[c = d]",
        "a/b[c = string(.)]",
        "a/b[./c = 'x']",
        "a/b[c[1] = 'x']",
        "a/b[@c = 'x']",
    ] {
        let before = keyed.calls.child_by_key.get();
        assert_eq!(select(&keyed, 0, expr), select(&plain, 0, expr), "{expr}");
        assert_eq!(keyed.calls.child_by_key.get(), before, "{expr}");
    }
}

// (a, cont.) Variables and current()-rooted paths are evaluated once against
// the step's context node and reach the hook as values.
#[test]
fn keyed_lookup_accepts_variables_and_current() {
    use counting::Rows;
    use xpath1::{ExpandedName, FunctionContext};
    let current =
        |f: &FunctionContext<'_, Rows>, _: &[Value<usize>]| Ok(Value::Nodes(vec![*f.initial_node]));
    for keyed in [true, false] {
        let nav = Rows::new(keyed, false);
        let ctx = Context::new(&nav, 0).with_variable("k", Value::String("y".into()));
        let found = XPath::compile("a/b[c = $k]").unwrap().select(&ctx).unwrap();
        assert_eq!(found, vec![8]);
        // From entry y's <c>, `current()` is that <c>: look the entry up by it.
        let ctx = Context::new(&nav, 9).with_function(ExpandedName::local("current"), current);
        let found = XPath::compile("/a/b[c = current()]/d")
            .unwrap()
            .select(&ctx)
            .unwrap();
        assert_eq!(found, vec![10]);
        let found = XPath::compile("/a/b[c = current()/../c]")
            .unwrap()
            .select(&ctx)
            .unwrap();
        assert_eq!(found, vec![8]);
        assert_eq!(nav.calls.child_by_key.get(), 3);
        let seen = nav.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, ExpandedName::local("c"));
        assert_eq!(seen[0].1, Value::Nodes(vec![9]));
    }
}

// (b) The right-hand side reaches the hook verbatim; the navigator, not the
// evaluator, decides canonical equality.
#[test]
fn keyed_lookup_passes_rhs_verbatim() {
    use counting::Rows;
    use xpath1::ExpandedName;
    let mut nav = Rows::new(true, false);
    nav.canonical = |s| s.trim().to_lowercase();
    assert_eq!(select(&nav, 0, "a/b[c = '  X ']"), vec![5]);
    assert_eq!(
        *nav.seen.borrow(),
        vec![(ExpandedName::local("c"), Value::String("  X ".into()))]
    );
    // A number is passed as a number, not converted to a string.
    assert!(select(&nav, 0, "a/b[c = 007]").is_empty());
    assert_eq!(
        *nav.seen.borrow(),
        vec![(ExpandedName::local("c"), Value::Number(7.0))]
    );
    // A scan applies XPath string equality instead: no match.
    assert!(select(&Rows::new(false, false), 0, "a/b[c = '  X ']").is_empty());
}

// (c) A hook returning None falls back to ordinary evaluation, identically,
// also through the children_named (R6) path.
#[test]
fn keyed_lookup_fallback_and_children_named() {
    use counting::{Rows, KEYS};
    let exprs = [
        "a/b[c = 'x']",
        "a/b[c = 'x']/d",
        "a/b[c = 'x'][d = 'd1']",
        "a/b[c = 'q']",
        "a/b",
        "a/b/c",
        "a/b[2]/d",
        "a/*",
        "a/b[c = $missing]/d",
    ];
    let reference = Rows::new(false, false);
    for (keyed, named) in [(false, false), (false, true), (true, true)] {
        let nav = Rows::new(keyed, named);
        for expr in exprs {
            let compiled = XPath::compile(expr).unwrap();
            let got = compiled.evaluate(&Context::new(&nav, 0));
            let want = compiled.evaluate(&Context::new(&reference, 0));
            match (got, want) {
                (Ok(got), Ok(want)) => assert_eq!(got, want, "{expr} {keyed} {named}"),
                (Err(got), Err(want)) => assert_eq!(got.kind, want.kind, "{expr}"),
                (got, want) => panic!("{expr}: {got:?} vs {want:?}"),
            }
        }
    }
    // The fallback consulted the hook, then scanned b's children.
    let nav = Rows::new(false, false);
    assert_eq!(select(&nav, 0, "a/b[c = 'x']"), vec![5]);
    assert_eq!(nav.calls.child_by_key.get(), 1);
    assert_eq!(nav.calls.children.get(), 2 + KEYS.len());
    // With children_named, a `parent/leaf` step reads no children at all.
    let nav = Rows::new(false, true);
    assert_eq!(select(&nav, 0, "a/b/d"), vec![4, 7, 10, 13]);
    assert_eq!(nav.calls.children.get(), 0);
    assert_eq!(nav.calls.name_matches.get(), 0);
    assert_eq!(nav.calls.children_named.get(), 2 + KEYS.len());
}

// (d) R5: one context re-focused with set_node serves many evaluations, with
// results equal to fresh contexts, keeping variables, namespaces and functions.
#[test]
fn set_node_reuses_one_context() {
    use counting::Rows;
    use xpath1::{ExpandedName, FunctionTable};
    let nav = Rows::new(true, true);
    let mut table = FunctionTable::new();
    table.insert(ExpandedName::new("urn:f", "twice"), |_, args| {
        Ok(Value::Number(
            2.0 * args[0].to_number(&Rows::new(false, false))?,
        ))
    });
    let expr = XPath::compile(
        "concat(string(.), '|', count(../b[c = $k]), '|', f:twice(count(d)), '|', position(), '/', last())",
    )
    .unwrap();
    let build = |node| {
        Context::new(&nav, node)
            .with_namespace("f", "urn:f")
            .with_variable("k", Value::String("y".into()))
            .with_functions(&table)
    };
    // Start from a context with a non-default focus: set_node resets it.
    let mut reused = build(2).with_focus(2, 3).unwrap();
    let nodes: Vec<usize> = (0..14).collect();
    for i in 0..1000 {
        let node = nodes[i % nodes.len()];
        reused.set_node(node);
        let got = expr.evaluate(&reused).unwrap();
        let want = expr.evaluate(&build(node)).unwrap();
        assert_eq!(got, want, "node {node}");
    }
    reused.set_node(8);
    assert_eq!(
        expr.evaluate(&reused).unwrap(),
        Value::String("|1|2|1/1".into())
    );
}

// (e) R5: a shared FunctionTable is callable from many contexts, and a
// per-context with_function shadows it. Works for a navigator that borrows.
#[test]
fn function_table_is_shared_and_shadowed() {
    use xpath1::{ExpandedName, FunctionTable, Navigator};
    struct Borrowed<'s>(&'s Document);
    impl Navigator for Borrowed<'_> {
        type Node = <Document as Navigator>::Node;
        fn kind(&self, n: &Self::Node) -> xpath1::NodeKind {
            self.0.kind(n)
        }
        fn parent(&self, n: &Self::Node) -> xpath1::Result<Option<Self::Node>> {
            self.0.parent(n)
        }
        fn children<'a>(
            &'a self,
            n: &'a Self::Node,
        ) -> impl Iterator<Item = xpath1::Result<Self::Node>> + 'a {
            self.0.children(n)
        }
        fn name(&self, n: &Self::Node) -> xpath1::Result<Option<ExpandedName>> {
            self.0.name(n)
        }
        fn string_value(&self, n: &Self::Node) -> xpath1::Result<String> {
            self.0.string_value(n)
        }
        fn compare_document_order(&self, a: &Self::Node, b: &Self::Node) -> std::cmp::Ordering {
            self.0.compare_document_order(a, b)
        }
    }

    let doc = Document::parse("<r><x>1</x><x>2</x></r>").unwrap();
    let nav = Borrowed(&doc);
    let mut table: FunctionTable<Borrowed<'_>> = FunctionTable::new();
    table
        .insert(ExpandedName::local("current"), |f, _| {
            Ok(Value::Nodes(vec![*f.initial_node]))
        })
        .insert(ExpandedName::local("which"), |_, _| {
            Ok(Value::String("table".into()))
        });
    let which = XPath::compile("which()").unwrap();
    let current = XPath::compile("count(current()/x)").unwrap();
    let root = XPath::compile("/r")
        .unwrap()
        .select(&Context::new(&nav, doc.root()))
        .unwrap();

    {
        let shared_a = Context::new(&nav, root[0]).with_functions(&table);
        let shared_b = Context::new(&nav, doc.root()).with_functions(&table);
        assert_eq!(
            which.evaluate(&shared_a).unwrap(),
            Value::String("table".into())
        );
        assert_eq!(current.evaluate(&shared_a).unwrap(), Value::Number(2.0));
        assert_eq!(current.evaluate(&shared_b).unwrap(), Value::Number(0.0));

        // with_function shadows the table, in either builder order.
        let local = |_: &xpath1::FunctionContext<'_, Borrowed<'_>>, _: &[Value<_>]| {
            Ok(Value::String("local".into()))
        };
        let after = Context::new(&nav, root[0])
            .with_functions(&table)
            .with_function(ExpandedName::local("which"), local);
        let before = Context::new(&nav, root[0])
            .with_function(ExpandedName::local("which"), local)
            .with_functions(&table);
        for ctx in [&after, &before] {
            assert_eq!(which.evaluate(ctx).unwrap(), Value::String("local".into()));
            // Unshadowed table entries remain callable.
            assert_eq!(current.evaluate(ctx).unwrap(), Value::Number(2.0));
        }
    }
    // Core functions are not overridable by either.
    table.insert(ExpandedName::local("count"), |_, _| Ok(Value::Number(-1.0)));
    let ctx = Context::new(&nav, root[0]).with_functions(&table);
    assert_eq!(
        XPath::compile("count(x)").unwrap().evaluate(&ctx).unwrap(),
        Value::Number(2.0)
    );
    // Without the table the function is unknown.
    let bare = Context::new(&nav, root[0]);
    assert_eq!(
        which.evaluate(&bare).unwrap_err().kind,
        xpath1::ErrorKind::Function
    );
}

// R7: the public key-run analysis is the evaluator's: a step reaches
// child_by_key exactly when `Step::key_run` is non-zero.
#[test]
fn key_run_analysis_matches_the_evaluator() {
    use counting::Rows;
    use xpath1::ast::{context_independent, key_pair, Expr, NodeTest};
    use xpath1::Axis;
    for (expr, run) in [
        ("a/b[c = 'x']", 1),
        ("a/b['x' = c]", 1),
        ("a/child::b[child::c = 'x']", 1),
        ("a/b[c = 'x'][d = 'd1']", 2),
        ("a/b[c = 'x'][1]", 1),
        ("a/b[c = $k][d = current()]", 2),
        ("a/b[c = current()/../c]", 1),
        ("a/b[1][c = 'x']", 0),
        ("a/b[position() = 2]", 0),
        ("a/b[c != 'x']", 0),
        ("a/b[c/text() = 'x']", 0),
        ("a/b[c = d]", 0),
        ("a/b[c = string(.)]", 0),
        ("a/b[./c = 'x']", 0),
        ("a/b[c[1] = 'x']", 0),
        ("a/b[@c = 'x']", 0),
        ("a/descendant::b[c = 'x']", 0),
        ("a/*[c = 'x']", 0),
    ] {
        let xpath = XPath::compile(expr).unwrap();
        let Expr::Path { steps, .. } = xpath.ast() else {
            panic!("{expr}");
        };
        let step = steps.last().unwrap();
        assert_eq!(step.key_run(), run, "{expr}");
        let nav = Rows::new(true, false);
        let current = |f: &xpath1::FunctionContext<'_, Rows>, _: &[Value<usize>]| {
            Ok(Value::Nodes(vec![*f.initial_node]))
        };
        let ctx = Context::new(&nav, 0)
            .with_variable("k", Value::String("x".into()))
            .with_function(xpath1::ExpandedName::local("current"), current);
        xpath.evaluate(&ctx).unwrap();
        assert_eq!(nav.calls.child_by_key.get() > 0, run > 0, "{expr}");
        // The run is the leading key pairs, ended by the first other predicate.
        assert!(step.predicates[..run].iter().all(|p| key_pair(p).is_some()));
        let keyable = step.axis == Axis::Child && matches!(step.test, NodeTest::Name(_));
        if let Some(next) = step.predicates.get(run).filter(|_| keyable) {
            assert!(key_pair(next).is_none(), "{expr}");
        }
    }
    let rhs = |source: &str| XPath::compile(source).unwrap().ast().clone();
    for (source, independent) in [
        ("'x'", true),
        ("7", true),
        ("$v", true),
        ("current()", true),
        ("current()/../c", true),
        ("c", false),
        ("string(.)", false),
        ("current(1)", false),
        ("p:current()", false),
        ("concat('a', 'b')", false),
    ] {
        assert_eq!(context_independent(&rhs(source)), independent, "{source}");
    }
    let pair = rhs("k = 'v'");
    let (key, value) = key_pair(&pair).unwrap();
    assert_eq!(key.local, "k");
    assert_eq!(*value, Expr::String("v".into()));
}

// R12: a `new_sync` table is Send + Sync whatever the navigator, and can be
// shared by threads that each evaluate with their own context.
#[test]
fn sync_function_table_is_shared_across_threads() {
    use std::rc::Rc;
    use xpath1::{ExpandedName, FunctionTable, Navigator, SyncFunctionTable};
    fn send_sync<T: Send + Sync>() {}
    send_sync::<SyncFunctionTable<Document>>();
    // Even for a navigator whose handles are not Send.
    struct RcTree;
    impl Navigator for RcTree {
        type Node = Rc<usize>;
        fn kind(&self, _: &Rc<usize>) -> xpath1::NodeKind {
            xpath1::NodeKind::Root
        }
        fn parent(&self, _: &Rc<usize>) -> xpath1::Result<Option<Rc<usize>>> {
            Ok(None)
        }
        fn children<'a>(
            &'a self,
            _: &'a Rc<usize>,
        ) -> impl Iterator<Item = xpath1::Result<Rc<usize>>> + 'a {
            std::iter::empty()
        }
        fn name(&self, _: &Rc<usize>) -> xpath1::Result<Option<ExpandedName>> {
            Ok(None)
        }
        fn string_value(&self, _: &Rc<usize>) -> xpath1::Result<String> {
            Ok(String::new())
        }
        fn compare_document_order(&self, a: &Rc<usize>, b: &Rc<usize>) -> std::cmp::Ordering {
            a.cmp(b)
        }
    }
    send_sync::<SyncFunctionTable<RcTree>>();

    let mut table = FunctionTable::new_sync();
    let offset = std::sync::Arc::new(10.0);
    table.insert(ExpandedName::local("plus"), move |_, args| {
        Ok(Value::Number(
            *offset + args[0].to_number(&Document::parse("<a/>")?)?,
        ))
    });
    let table = &table;
    let results: Vec<Value<_>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|i| {
                scope.spawn(move || {
                    let doc = Document::parse(&format!("<r>{}</r>", "<x/>".repeat(i))).unwrap();
                    let ctx = Context::new(&doc, doc.root()).with_functions(table);
                    let value = XPath::compile("plus(count(/r/x))")
                        .unwrap()
                        .evaluate(&ctx)
                        .unwrap();
                    match value {
                        Value::Number(n) => Value::<()>::Number(n),
                        _ => unreachable!(),
                    }
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(
        results,
        (0..4)
            .map(|i| Value::Number(10.0 + i as f64))
            .collect::<Vec<_>>()
    );
    // A per-thread table still accepts closures that are not Send.
    let mut local: FunctionTable<Document> = FunctionTable::new();
    let shared = Rc::new(1.0);
    local.insert(ExpandedName::local("one"), move |_, _| {
        Ok(Value::Number(*shared))
    });
    let doc = Document::parse("<r/>").unwrap();
    let ctx = Context::new(&doc, doc.root()).with_functions(&local);
    assert_eq!(
        XPath::compile("one()").unwrap().evaluate(&ctx).unwrap(),
        Value::Number(1.0)
    );
}
