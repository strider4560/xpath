#![cfg(feature = "xml")]
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    rc::Rc,
};
use xpath1::{
    xml::Document, Context, ExpandedName, FunctionTable, Namespaces, Navigator, NodeSetPool, Value,
    XPath,
};

/// Counts allocations made by the current thread only, so tests running in
/// parallel do not disturb each other's measurements.
struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations_during(f: impl FnOnce()) -> usize {
    let before = ALLOCATIONS.with(Cell::get);
    f();
    ALLOCATIONS.with(Cell::get) - before
}

fn strings(doc: &Document, value: &Value<<Document as Navigator>::Node>) -> Vec<String> {
    match value {
        Value::Nodes(nodes) => nodes.iter().map(|n| doc.string_value(n).unwrap()).collect(),
        other => panic!("expected nodes: {other:?}"),
    }
}

#[test]
fn pool_buffers_are_returned_empty_and_result_is_owned() {
    let doc = Document::parse("<a><b>1</b><b>2</b></a>").unwrap();
    let xpath = XPath::compile("/a/b").unwrap();
    let pool = NodeSetPool::new();
    let result = {
        let ctx = Context::new(&doc, doc.root()).with_pool(&pool);
        xpath.evaluate(&ctx).unwrap()
    };
    assert_eq!(strings(&doc, &result), ["1", "2"]);
    assert!(
        !pool.is_empty(),
        "intermediate buffers went back to the pool"
    );
    let pooled: Vec<_> = (0..pool.len()).map(|_| pool.take()).collect();
    assert!(pool.is_empty());
    for buffer in pooled {
        assert!(buffer.is_empty());
        pool.give(buffer);
    }
    drop(pool);
    // The result belongs to the caller and outlives the pool.
    assert_eq!(strings(&doc, &result), ["1", "2"]);
}

#[test]
fn warm_path_expression_allocates_nothing() {
    let doc = Document::parse(
        r#"<a xmlns:p="urn:p"><p:b><p:c><p:d/><p:e>1</p:e></p:c><p:c><p:d/><p:e>2</p:e></p:c></p:b></a>"#,
    )
    .unwrap();
    // `current()` builds its result in a buffer from the same pool, as a host
    // owning the pool would; with `vec![..]` instead, each call allocates and
    // the capacity-1 vectors it hands to the pool are later regrown.
    let pool = Rc::new(NodeSetPool::new());
    let mut functions = FunctionTable::new();
    let function_pool = Rc::clone(&pool);
    functions.insert(ExpandedName::local("current"), move |fc, _| {
        let mut nodes = function_pool.take();
        nodes.push(*fc.initial_node);
        Ok(Value::Nodes(nodes))
    });
    let ns = Namespaces::new().with_namespace("p", "urn:p");
    let xpath = XPath::compile("/a/p:b/p:c[p:d = current()]/p:e").unwrap();
    // The focus is an empty `p:d`, so every string-value compared is empty
    // and reading it does not allocate (`Navigator::string_value` returns an
    // owned `String`, a residual source outside the pool's reach).
    let focus = XPath::compile("/a/p:b/p:c/p:d")
        .unwrap()
        .select(&Context::new(&doc, doc.root()).with_namespaces(&ns))
        .unwrap()[0];
    let ctx = Context::new(&doc, focus)
        .with_namespaces(&ns)
        .with_functions(&functions)
        .with_pool(&pool);
    assert_eq!(strings(&doc, &xpath.evaluate(&ctx).unwrap()), ["1", "2"]);
    let warm = || {
        if let Value::Nodes(nodes) = xpath.evaluate(&ctx).unwrap() {
            assert_eq!(nodes.len(), 2);
            // The caller gives the owned result back, as a host reusing it would.
            pool.give(nodes);
        }
    };
    warm();
    // Node-set buffers allocate nothing when warm. The exact residual is the
    // R2 keyed-lookup attempt for `p:c[p:d = ...]` (the XML adapter declines
    // `child_by_key`, so the step then scans), once per evaluation, all from
    // the R2 owned-key API:
    //   1  the values vector, the `(ExpandedName, Value)` slice passed to
    //      `child_by_key`;
    //   2  `ExpandedName::clone(key)` for `p:d` (namespace URI + local name)
    //      owned by that slice.
    const KEYED_LOOKUP: usize = 3;
    for _ in 0..3 {
        assert_eq!(allocations_during(warm), KEYED_LOOKUP);
    }
    // Without the key predicate nothing at all is allocated.
    let unkeyed = XPath::compile("/a/p:b/p:c[true()][p:d = current()]/p:e").unwrap();
    let warm = || {
        if let Value::Nodes(nodes) = unkeyed.evaluate(&ctx).unwrap() {
            assert_eq!(nodes.len(), 2);
            pool.give(nodes);
        }
    };
    warm();
    assert_eq!(allocations_during(warm), 0);
}

#[test]
fn pool_is_bounded() {
    let pool = NodeSetPool::<u32>::new();
    for value in 0..100 {
        pool.give(vec![value]);
    }
    assert_eq!(pool.len(), 64);
    assert!(pool.take().capacity() >= 1);
}

const DOC: &str = r#"<r>
  <s id="1"><t>a</t><t>b</t><u><t>c</t></u></s>
  <s id="2"><t>d</t><u><t>e</t><t>f</t></u></s>
  <s id="3"/>
  <v>10</v><v>3</v><v>7</v>
</r>"#;

#[test]
fn results_match_without_pool() {
    let doc = Document::parse(DOC).unwrap();
    let pool = NodeSetPool::new();
    let plain = Context::new(&doc, doc.root());
    let pooled = Context::new(&doc, doc.root()).with_pool(&pool);
    for source in [
        "/r/s",
        "/r/s/t",
        "//t",
        "/r/s[2]/u/t",
        "/r/s[@id = '2']//t",
        "/r/s[t]/@id",
        "//t[2]",
        "(//t)[2]",
        "/r/s/t | /r/s/u/t",
        "//u/t | //s[1]/t",
        "/r/descendant::t[last()]",
        "/r/s[position() > 1]",
        "//t[. = 'e']/ancestor::s/@id",
        "//t/following-sibling::t",
        "//t/preceding-sibling::*",
        "//u/preceding::t",
        "/r/s[1]/following::t",
        "/r/v[. > 5]",
        "count(//t) = 6 and sum(/r/v) = 20",
        "//t[not(../../u)] | /r/s[3]",
    ] {
        let xpath = XPath::compile(source).unwrap();
        // Twice with the pool, so the second run reuses buffers.
        for _ in 0..2 {
            let expected = xpath.evaluate(&plain).unwrap();
            let actual = xpath.evaluate(&pooled).unwrap();
            match (&expected, &actual) {
                (Value::Nodes(_), Value::Nodes(_)) => {
                    assert_eq!(strings(&doc, &expected), strings(&doc, &actual), "{source}")
                }
                _ => assert_eq!(expected, actual, "{source}"),
            }
        }
    }
}
