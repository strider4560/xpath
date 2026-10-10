mod support;

use criterion::{BenchmarkId, Criterion};
use std::{hint::black_box, time::Duration};
use support::{Counted, Tree, NS, SELECT, WHEN};
use xpath1::{Context, Value, XPath};

const SELECTORS: [(&str, &str); 5] = [
    ("all", "/if:interfaces/if:interface"),
    ("key", SELECT),
    ("descendants", "//if:mtu"),
    ("union", "/if:interfaces/if:interface | //if:interface"),
    (
        "reverse",
        "/if:interfaces/if:interface[last()]/preceding-sibling::if:interface[1]",
    ),
];

fn compilation(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile");
    for (name, source) in [
        ("selector", SELECT),
        ("when", WHEN),
        ("axes", SELECTORS[4].1),
    ] {
        assert!(XPath::compile(source).is_ok());
        group.bench_function(name, |b| {
            b.iter(|| XPath::compile(black_box(source)).unwrap())
        });
    }
    group.finish();
}

fn application_tree(c: &mut Criterion) {
    let schema = Tree::interfaces(1);
    let context = Context::new(&schema, 0).with_namespace("if", NS);
    let selector = XPath::compile(SELECTORS[0].1).unwrap();
    assert_eq!(selector.select(&context).unwrap().len(), 1);
    c.bench_function("schema/select_declaration", |b| {
        b.iter(|| black_box(&selector).select(black_box(&context)).unwrap())
    });

    let mut group = c.benchmark_group("data");
    for count in [10, 100, 1000] {
        let tree = Tree::interfaces(count);
        let context = Context::new(&tree, 0)
            .with_namespace("if", NS)
            .with_variable("wanted", Value::String(format!("eth{}", count / 2)));
        for (name, source) in SELECTORS {
            let expression = XPath::compile(source).unwrap();
            let expected = if matches!(name, "key" | "reverse") {
                1
            } else {
                count
            };
            assert_eq!(
                expression.select(&context).unwrap().len(),
                expected,
                "{name}"
            );
            group.bench_function(BenchmarkId::new(name, count), |b| {
                b.iter(|| black_box(&expression).select(black_box(&context)).unwrap())
            });
        }
    }
    group.finish();

    let tree = Tree::interfaces(1000);
    let expression = XPath::compile(WHEN).unwrap();
    let mut group = c.benchmark_group("when");
    for (name, index, expected) in [("true", 0, true), ("false", 1, false)] {
        let context = Context::new(&tree, tree.mtu[index]).with_default_element_namespace(NS);
        assert_eq!(
            expression.evaluate(&context).unwrap().to_boolean(),
            expected
        );
        group.bench_function(name, |b| {
            b.iter(|| {
                black_box(&expression)
                    .evaluate(black_box(&context))
                    .unwrap()
                    .to_boolean()
            })
        });
    }
    // One reusable expression, with a fresh focus for each data-node instance.
    let contexts: Vec<_> = tree
        .mtu
        .iter()
        .map(|&node| Context::new(&tree, node).with_default_element_namespace(NS))
        .collect();
    let evaluate_all = || {
        contexts
            .iter()
            .filter(|context| {
                expression
                    .evaluate(black_box(context))
                    .unwrap()
                    .to_boolean()
            })
            .count()
    };
    assert_eq!(evaluate_all(), 500);
    group.bench_function("1000_contexts", |b| b.iter(evaluate_all));
    group.finish();

    let context = Context::new(&tree, 0);
    let scalar = XPath::compile("(17 + 3) * 4 div 2 = 40 and not(false())").unwrap();
    assert!(scalar.evaluate(&context).unwrap().to_boolean());
    c.bench_function("scalar/cached", |b| {
        b.iter(|| black_box(&scalar).evaluate(black_box(&context)).unwrap())
    });
}

fn comparisons(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison");
    for count in [16, 128, 512] {
        for (name, uniform, late_match, source, expected) in [
            ("disjoint", false, false, "/root/a = /root/b", false),
            ("late_match", false, true, "/root/a = /root/b", true),
            ("uniform_ne", true, false, "/root/a != /root/b", false),
        ] {
            let tree = Tree::comparisons(count, uniform, late_match);
            let context = Context::new(&tree, 0).with_default_element_namespace(NS);
            let expression = XPath::compile(source).unwrap();
            assert_eq!(
                expression.evaluate(&context).unwrap().to_boolean(),
                expected
            );
            group.bench_function(BenchmarkId::new(name, count), |b| {
                b.iter(|| {
                    black_box(&expression)
                        .evaluate(black_box(&context))
                        .unwrap()
                        .to_boolean()
                })
            });
        }
    }
    group.finish();
}

#[cfg(feature = "xml")]
fn xml(c: &mut Criterion) {
    use xpath1::xml::Document;
    let mut source = format!("<interfaces xmlns='{NS}'>");
    for index in 0..1000 {
        source.push_str(&format!("<interface><name>eth{index}</name><enabled>true</enabled><kind>ethernet</kind><mtu>1500</mtu></interface>"));
    }
    source.push_str("</interfaces>");
    let document = Document::parse(&source).unwrap();
    let context = Context::new(&document, document.root())
        .with_namespace("if", NS)
        .with_variable("wanted", Value::String("eth500".into()));
    let selector = XPath::compile(SELECT).unwrap();
    let selected = selector.select(&context).unwrap();
    assert_eq!(selected.len(), 1);
    c.bench_function("xml/key/1000", |b| {
        b.iter(|| black_box(&selector).select(black_box(&context)).unwrap())
    });
    let mtu = XPath::compile("if:mtu")
        .unwrap()
        .select(&Context::new(&document, selected[0]).with_namespace("if", NS))
        .unwrap()[0];
    let context = Context::new(&document, mtu).with_default_element_namespace(NS);
    let when = XPath::compile(WHEN).unwrap();
    assert!(when.evaluate(&context).unwrap().to_boolean());
    c.bench_function("xml/when", |b| {
        b.iter(|| {
            black_box(&when)
                .evaluate(black_box(&context))
                .unwrap()
                .to_boolean()
        })
    });
    c.bench_function("xml/parse/1000", |b| {
        b.iter(|| Document::parse(black_box(&source)).unwrap())
    });
}

fn counts() {
    println!("case,name_calls,string_calls,order_calls,child_nodes");
    let tree = Tree::interfaces(1000);
    for (name, source) in SELECTORS.into_iter().chain([("when", WHEN)]) {
        let nav = Counted::new(&tree);
        let context = Context::new(&nav, if name == "when" { tree.mtu[0] } else { 0 })
            .with_namespace("if", NS)
            .with_default_element_namespace(NS)
            .with_variable("wanted", Value::String("eth500".into()));
        XPath::compile(source).unwrap().evaluate(&context).unwrap();
        println!(
            "{name},{},{},{},{}",
            nav.names.get(),
            nav.strings.get(),
            nav.order.get(),
            nav.children.get()
        );
    }
    for (name, uniform, late_match, source) in [
        ("disjoint_512", false, false, "/root/a = /root/b"),
        ("late_match_512", false, true, "/root/a = /root/b"),
        ("uniform_ne_512", true, false, "/root/a != /root/b"),
    ] {
        let tree = Tree::comparisons(512, uniform, late_match);
        let nav = Counted::new(&tree);
        let context = Context::new(&nav, 0).with_default_element_namespace(NS);
        XPath::compile(source).unwrap().evaluate(&context).unwrap();
        println!(
            "{name},{},{},{},{}",
            nav.names.get(),
            nav.strings.get(),
            nav.order.get(),
            nav.children.get()
        );
    }
}

fn main() {
    if std::env::args().any(|arg| arg == "--counts") {
        counts();
        return;
    }
    let mut criterion = Criterion::default()
        .warm_up_time(Duration::from_millis(300))
        .measurement_time(Duration::from_secs(1))
        .sample_size(40)
        .configure_from_args();
    compilation(&mut criterion);
    application_tree(&mut criterion);
    comparisons(&mut criterion);
    #[cfg(feature = "xml")]
    xml(&mut criterion);
    criterion.final_summary();
}
