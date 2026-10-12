#![cfg(feature = "xml")]
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
use xpath1::{xml::Document, Context, ErrorKind, Namespaces, Navigator, XPath};

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

const DOC: &str = r#"<a xmlns="urn:d" xmlns:p="urn:p"><p:b>x</p:b><b>y</b></a>"#;

fn selected_strings(doc: &Document, xpath: &XPath, ctx: &Context<'_, Document>) -> Vec<String> {
    xpath
        .select(ctx)
        .unwrap()
        .iter()
        .map(|n| doc.string_value(n).unwrap())
        .collect()
}

#[test]
fn shared_namespaces_resolve_prefixes_and_default() {
    let doc = Document::parse(DOC).unwrap();
    let ns = Namespaces::new()
        .with_namespace("p", "urn:p")
        .with_default_element_namespace("urn:d");
    assert_eq!(ns.namespace("p"), Some("urn:p"));
    assert_eq!(ns.namespace("q"), None);
    assert_eq!(ns.default_element_namespace(), "urn:d");
    let ctx = Context::new(&doc, doc.root()).with_namespaces(&ns);
    let prefixed = XPath::compile("/a/p:b").unwrap();
    let unprefixed = XPath::compile("/a/b").unwrap();
    assert_eq!(selected_strings(&doc, &prefixed, &ctx), ["x"]);
    assert_eq!(selected_strings(&doc, &unprefixed, &ctx), ["y"]);
    // Warm (cached) evaluations give the same answers.
    assert_eq!(selected_strings(&doc, &prefixed, &ctx), ["x"]);
    assert_eq!(selected_strings(&doc, &unprefixed, &ctx), ["y"]);
    let union = XPath::compile("/a/p:b | /a/b").unwrap();
    assert_eq!(selected_strings(&doc, &union, &ctx), ["x", "y"]);
}

#[test]
fn own_binding_overrides_shared_without_caching() {
    let doc = Document::parse(DOC).unwrap();
    let ns = Namespaces::new()
        .with_namespace("p", "urn:p")
        .with_default_element_namespace("urn:d");
    let xpath = XPath::compile("/a/p:b").unwrap();
    let overriding = Context::new(&doc, doc.root())
        .with_namespace("p", "urn:d")
        .with_namespaces(&ns);
    assert_eq!(selected_strings(&doc, &xpath, &overriding), ["y"]);
    let shared = Context::new(&doc, doc.root()).with_namespaces(&ns);
    assert_eq!(selected_strings(&doc, &xpath, &shared), ["x"]);
    // An own default element namespace also bypasses the cache.
    let own_default = Context::new(&doc, doc.root())
        .with_default_element_namespace("urn:none")
        .with_namespaces(&ns);
    let unprefixed = XPath::compile("/a/b").unwrap();
    assert!(unprefixed.select(&own_default).unwrap().is_empty());
    assert_eq!(selected_strings(&doc, &unprefixed, &shared), ["y"]);
}

#[test]
fn cache_is_per_namespaces() {
    let doc = Document::parse(DOC).unwrap();
    let ns1 = Namespaces::new()
        .with_namespace("p", "urn:p")
        .with_default_element_namespace("urn:d");
    let ns2 = Namespaces::new()
        .with_namespace("p", "urn:d")
        .with_default_element_namespace("urn:d");
    let xpath = XPath::compile("/a/p:b").unwrap();
    for _ in 0..2 {
        let one = Context::new(&doc, doc.root()).with_namespaces(&ns1);
        let two = Context::new(&doc, doc.root()).with_namespaces(&ns2);
        assert_eq!(selected_strings(&doc, &xpath, &one), ["x"]);
        assert_eq!(selected_strings(&doc, &xpath, &two), ["y"]);
    }
}

#[test]
fn unbound_prefix_is_still_a_namespace_error() {
    let doc = Document::parse(DOC).unwrap();
    let ns = Namespaces::new();
    let ctx = Context::new(&doc, doc.root()).with_namespaces(&ns);
    let error = XPath::compile("/a/q:b").unwrap().select(&ctx).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Namespace);
    // The xml prefix stays bound without any declaration.
    XPath::compile("/a/@xml:lang")
        .unwrap()
        .select(&ctx)
        .unwrap();
}

#[test]
fn warm_evaluation_allocates_nothing_for_names() {
    let doc = Document::parse(r#"<a xmlns:p="urn:p"><p:b><p:c><p:d/></p:c></p:b></a>"#).unwrap();
    let xpath = XPath::compile("/a/p:b/p:c/p:d").unwrap();
    let ns = Namespaces::new().with_namespace("p", "urn:p");
    let shared = Context::new(&doc, doc.root()).with_namespaces(&ns);
    let own = Context::new(&doc, doc.root()).with_namespace("p", "urn:p");
    assert_eq!(xpath.select(&shared).unwrap().len(), 1);
    assert_eq!(xpath.select(&own).unwrap().len(), 1);
    let warm_shared = allocations_during(|| {
        xpath.select(&shared).unwrap();
    });
    let warm_own = allocations_during(|| {
        xpath.select(&own).unwrap();
    });
    // Four name tests, each previously costing at least two allocations.
    assert!(
        warm_own >= warm_shared + 8,
        "shared {warm_shared} vs own {warm_own}"
    );

    // `p:d` has no children, so a final `p:*` or `*` step visits no candidate
    // and the two paths differ only in resolving `p:*`'s namespace, which
    // must not allocate either.
    let wildcard = XPath::compile("/a/p:b/p:c/p:d/p:*").unwrap();
    let any = XPath::compile("/a/p:b/p:c/p:d/*").unwrap();
    assert!(wildcard.select(&shared).unwrap().is_empty());
    assert!(any.select(&shared).unwrap().is_empty());
    let warm_wildcard = allocations_during(|| {
        wildcard.select(&shared).unwrap();
    });
    let warm_any = allocations_during(|| {
        any.select(&shared).unwrap();
    });
    assert_eq!(warm_wildcard, warm_any, "p:* vs * on a warm shared set");
}
