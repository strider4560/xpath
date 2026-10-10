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
