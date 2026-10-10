#![cfg(feature = "xml")]
use xpath1::{xml::Document, Navigator, NodeKind};

#[test]
fn owned_xpath_model_preserves_kinds_text_names_and_order() {
    let doc = Document::parse("<?p before?><!--outside--><p:r xmlns:p='urn:u' xmlns:q='urn:u' q:a='v'>a<![CDATA[b]]>c<!--inside--><?target data?><q:child/>d</p:r>").unwrap();
    let root = doc.root();
    let top: Vec<_> = doc.children(&root).map(Result::unwrap).collect();
    assert_eq!(
        top.iter().map(|n| doc.kind(n)).collect::<Vec<_>>(),
        vec![
            NodeKind::ProcessingInstruction,
            NodeKind::Comment,
            NodeKind::Element
        ]
    );
    let element = top[2];
    assert_eq!(doc.qualified_name(&element).unwrap(), "p:r");
    assert_eq!(doc.string_value(&root).unwrap(), "abcd");
    let children: Vec<_> = doc.children(&element).map(Result::unwrap).collect();
    assert_eq!(children.len(), 5);
    assert_eq!(doc.string_value(&children[0]).unwrap(), "abc");
    assert_eq!(doc.string_value(&children[1]).unwrap(), "inside");
    assert_eq!(doc.qualified_name(&children[2]).unwrap(), "target");
    assert_eq!(doc.qualified_name(&children[3]).unwrap(), "q:child");
    let attrs: Vec<_> = doc.attributes(&element).map(Result::unwrap).collect();
    assert_eq!(attrs.len(), 1);
    assert_eq!(doc.kind(&attrs[0]), NodeKind::Attribute);
    assert_eq!(doc.qualified_name(&attrs[0]).unwrap(), "q:a");
    assert_eq!(doc.parent(&attrs[0]).unwrap(), Some(element));
    assert!(doc.compare_document_order(&element, &attrs[0]).is_lt());
    assert!(doc.compare_document_order(&attrs[0], &children[0]).is_lt());
    assert!(doc.children(&attrs[0]).next().is_none());
}

#[test]
fn namespaces_are_in_scope_per_owner_and_include_xml() {
    let doc =
        Document::parse("<r xmlns='urn:default' xmlns:p='urn:p'><s/><t xmlns=''/></r>").unwrap();
    let root = doc.root();
    let r = doc.children(&root).next().unwrap().unwrap();
    let children: Vec<_> = doc.children(&r).map(Result::unwrap).collect();
    let ns: Vec<_> = doc.namespaces(&r).map(Result::unwrap).collect();
    let inherited: Vec<_> = doc.namespaces(&children[0]).map(Result::unwrap).collect();
    assert_eq!(ns.len(), 3);
    assert_eq!(inherited.len(), 3);
    assert!(ns.iter().all(|n| !inherited.contains(n)));
    for n in &ns {
        assert_eq!(doc.parent(n).unwrap(), Some(r));
    }
    assert!(ns.iter().any(|n| doc.qualified_name(n).unwrap() == "xml"
        && doc.string_value(n).unwrap() == "http://www.w3.org/XML/1998/namespace"));
    assert_eq!(doc.namespaces(&children[1]).count(), 2);
    assert_eq!(doc.namespaces(&root).count(), 0);
}

#[test]
fn document_identity_and_explicit_ids() {
    let mut a = Document::parse("<r id='ordinary'/>").unwrap();
    let b = Document::parse("<r/>").unwrap();
    assert_ne!(a.root(), b.root());
    assert!(a.validate_node(&b.root()).is_err());
    assert!(a.parent(&b.root()).is_err());
    assert!(a.children(&b.root()).next().unwrap().is_err());
    assert!(a.string_value(&b.root()).is_err());
    let element = a.children(&a.root()).next().unwrap().unwrap();
    assert_eq!(a.element_by_id(&element, "ordinary").unwrap(), None);
    a.register_id("ordinary", element).unwrap();
    assert_eq!(
        a.element_by_id(&element, "ordinary").unwrap(),
        Some(element)
    );
    assert!(a.register_id("foreign", b.root()).is_err());
    assert!(a.register_id("root", a.root()).is_err());
}

#[test]
fn internal_entities_coalesce_and_external_entities_do_not_load() {
    let doc = Document::parse("<!DOCTYPE r [<!ENTITY e 'expanded'>]><r>a&e;b</r>").unwrap();
    assert_eq!(doc.string_value(&doc.root()).unwrap(), "aexpandedb");
    let element = doc.children(&doc.root()).next().unwrap().unwrap();
    assert_eq!(doc.children(&element).count(), 1);
    assert!(
        Document::parse("<!DOCTYPE r [<!ENTITY e SYSTEM 'file:///etc/passwd'>]><r>&e;</r>")
            .is_err()
    );
    assert!(Document::parse("<r>").is_err());
}

#[test]
fn internal_entity_markup_preserves_lexical_prefixes() {
    let doc = Document::parse("<!DOCTYPE r [<!ENTITY e '<q:child p:a=\"v\"/>'>]><r xmlns:p='urn:u' xmlns:q='urn:u'>&e;</r>").unwrap();
    let root = doc.root();
    let r = doc.children(&root).next().unwrap().unwrap();
    let child = doc.children(&r).next().unwrap().unwrap();
    assert_eq!(doc.qualified_name(&child).unwrap(), "q:child");
    let attr = doc.attributes(&child).next().unwrap().unwrap();
    assert_eq!(doc.qualified_name(&attr).unwrap(), "p:a");
}

#[test]
fn deeply_nested_xml_import_uses_bounded_call_stack() {
    let depth = 10_000;
    let source = format!("{}text{}", "<r>".repeat(depth), "</r>".repeat(depth));
    assert_eq!(
        Document::parse(&source).unwrap_err().kind,
        xpath1::ErrorKind::Limit
    );
    let source = format!("{}text{}", "<r>".repeat(256), "</r>".repeat(256));
    let doc = Document::parse(&source).unwrap();
    assert_eq!(doc.string_value(&doc.root()).unwrap(), "text");
}

#[test]
fn expanded_entity_markup_counts_toward_xml_depth_limit() {
    let mut declarations = String::new();
    for index in 0..8 {
        let content = if index == 0 {
            "text".to_owned()
        } else {
            format!("&e{};", index - 1)
        };
        declarations.push_str(&format!(
            "<!ENTITY e{index} '{}{content}{}'>",
            "<r>".repeat(40),
            "</r>".repeat(40)
        ));
    }
    let source = format!("<!DOCTYPE r [{declarations}]><r>&e7;</r>");
    assert_eq!(
        Document::parse(&source).unwrap_err().kind,
        xpath1::ErrorKind::Limit
    );
    let source = format!("<!DOCTYPE r [{declarations}]><r>&e1;</r>");
    let doc = Document::parse(&source).unwrap();
    assert_eq!(doc.string_value(&doc.root()).unwrap(), "text");
}

#[test]
fn depth_guard_distinguishes_markup_from_comments_cdata_and_unicode() {
    let apparent_tags = "<r>".repeat(10_000);
    let source =
        format!("<r><!--{apparent_tags}--><?p {apparent_tags}?><![CDATA[{apparent_tags}]]></r>");
    let doc = Document::parse(&source).unwrap();
    assert_eq!(doc.string_value(&doc.root()).unwrap(), apparent_tags);
    let doc = Document::parse("<!DOCTYPE r [<!ENTITY é 'v'>]><r>&é;</r>").unwrap();
    assert_eq!(doc.string_value(&doc.root()).unwrap(), "v");
}

#[test]
fn evaluation_rejects_foreign_context_variable_and_extension_handles() {
    use xpath1::{Context, ErrorKind, ExpandedName, Value, XPath};
    let a = Document::parse("<r/>").unwrap();
    let b = Document::parse("<r/>").unwrap();
    let expr = XPath::compile(".").unwrap();
    assert_eq!(
        expr.evaluate(&Context::new(&a, b.root())).unwrap_err().kind,
        ErrorKind::Navigation
    );
    let ctx = Context::new(&a, a.root()).with_variable("foreign", Value::Nodes(vec![b.root()]));
    assert_eq!(
        XPath::compile("$foreign")
            .unwrap()
            .evaluate(&ctx)
            .unwrap_err()
            .kind,
        ErrorKind::Navigation
    );
    let foreign = b.root();
    let ctx = Context::new(&a, a.root())
        .with_function(ExpandedName::local("foreign"), move |_, _| {
            Ok(Value::Nodes(vec![foreign]))
        });
    assert_eq!(
        XPath::compile("foreign()")
            .unwrap()
            .evaluate(&ctx)
            .unwrap_err()
            .kind,
        ErrorKind::Navigation
    );
}

#[test]
fn dtd_processing_instructions_cannot_shadow_depth_guard_entities() {
    let replacement = format!("{}text{}", "<r>".repeat(256), "</r>".repeat(256));
    for prefix in [
        "",
        "<?pi <!ENTITY deep ''> ?>",
        "<?pi ]> ?>",
        "<!-- <!ENTITY deep ''> -->",
    ] {
        let source = format!("<!DOCTYPE r [{prefix}<!ENTITY deep '{replacement}'>]><r>&deep;</r>");
        assert_eq!(
            Document::parse(&source).err().map(|e| e.kind),
            Some(xpath1::ErrorKind::Limit),
            "DTD prefix: {prefix}"
        );
    }
    let replacement = format!("{}text{}", "<r>".repeat(255), "</r>".repeat(255));
    let source = format!(
        "<!DOCTYPE r [<?pi <!ENTITY deep ''> ?><!ENTITY deep '{replacement}'>]><r>&deep;</r>"
    );
    let doc = Document::parse(&source).unwrap();
    assert_eq!(doc.string_value(&doc.root()).unwrap(), "text");
}
