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
