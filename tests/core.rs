use std::cmp::Ordering;
use xpath1::{
    Context, Error, ErrorKind, ExpandedName, Limits, Navigator, NodeKind, Result, Value, XPath,
};

// An application-owned tree. Handles are integer IDs; no XML library is used.
struct Tree;
impl Navigator for Tree {
    type Node = usize;
    fn kind(&self, n: &usize) -> NodeKind {
        if *n == 0 {
            NodeKind::Root
        } else {
            NodeKind::Element
        }
    }
    fn parent(&self, n: &usize) -> Result<Option<usize>> {
        Ok(match n {
            0 => None,
            1 => Some(0),
            2 | 3 => Some(1),
            4 | 5 => Some(3),
            _ => None,
        })
    }
    fn children<'a>(&'a self, n: &'a usize) -> impl Iterator<Item = Result<usize>> + 'a {
        let nodes: &[usize] = match n {
            0 => &[1],
            1 => &[2, 3],
            3 => &[4, 5],
            _ => &[],
        };
        nodes.iter().copied().map(Ok)
    }
    fn name(&self, n: &usize) -> Result<Option<ExpandedName>> {
        Ok(match n {
            0 => None,
            1 => Some(ExpandedName::local("root")),
            _ => Some(ExpandedName::local("n")),
        })
    }
    fn string_value(&self, n: &usize) -> Result<String> {
        Ok(match n {
            2 => "2",
            3 => "10",
            4 => "3",
            5 => "7",
            _ => "210",
        }
        .into())
    }
    fn compare_document_order(&self, a: &usize, b: &usize) -> Ordering {
        a.cmp(b)
    }
}
fn value(expr: &str, node: usize) -> Value<usize> {
    XPath::compile(expr)
        .unwrap()
        .evaluate(&Context::new(&Tree, node))
        .unwrap()
}
fn nodes(expr: &str, node: usize) -> Vec<usize> {
    match value(expr, node) {
        Value::Nodes(n) => n,
        other => panic!("expected nodes: {other:?}"),
    }
}
fn boolean(expr: &str) -> bool {
    value(expr, 0).to_boolean()
}

#[test]
fn path_focus_and_reverse_axes() {
    assert_eq!(nodes("//n[1]", 0), vec![2, 4]);
    assert_eq!(nodes("(//n)[1]", 0), vec![2]);
    assert_eq!(nodes("ancestor::*[1]", 5), vec![3]);
    assert_eq!(nodes("(ancestor::*)[1]", 5), vec![1]);
    assert_eq!(nodes("preceding::n[1]", 5), vec![4]);
    assert_eq!(nodes("(preceding::n)[1]", 5), vec![2]);
    assert_eq!(nodes("following::n", 2), vec![3, 4, 5]);
    assert_eq!(nodes("following::n", 3), Vec::<usize>::new());
    assert_eq!(nodes("//n[last()]", 0), vec![3, 5]);
}
#[test]
fn union_sorting_dedup_and_step_predicates() {
    assert_eq!(nodes("//n | /root/n | /root", 0), vec![1, 2, 3, 4, 5]);
    assert_eq!(nodes("//n[. > 2][1]", 0), vec![3, 4]);
    assert_eq!(nodes("//n/..", 0), vec![1, 3]);
}
#[test]
fn existential_comparisons_and_scalar_coercions() {
    assert!(boolean("/root/n = 2"));
    assert!(boolean("/root/n != 2"));
    assert!(boolean("/root/n = /root/n"));
    assert!(boolean("/root/n != /root/n"));
    assert!(!boolean("/missing != /root/n"));
    assert!(boolean("/missing = false()"));
    assert!(boolean("/root/n > true()"));
    assert!(!boolean("number('NaN') < 1"));
    assert!(boolean("'0' = true()"));
    assert!(boolean("3 > 2 > 1 = false()"));
}
#[test]
fn short_circuit_and_errors() {
    assert!(!boolean("false() and $missing"));
    assert!(boolean("true() or missing()"));
    for (s, k) in [
        ("$missing", ErrorKind::Variable),
        ("1/child::*", ErrorKind::Type),
        ("1 | /root", ErrorKind::Type),
        ("unknown()", ErrorKind::Function),
        ("/missing/p:thing", ErrorKind::Namespace),
    ] {
        assert_eq!(
            XPath::compile(s)
                .unwrap()
                .evaluate(&Context::new(&Tree, 0))
                .unwrap_err()
                .kind,
            k,
            "{s}"
        );
    }
}
#[test]
fn external_variables_and_function_focus() {
    let context = Context::new(&Tree, 1)
        .with_namespace("host", "urn:host")
        .with_variable("nodes", Value::Nodes(vec![5, 2, 2]))
        .with_variable_ns(ExpandedName::new("urn:host", "cutoff"), Value::Number(2.0))
        .with_function(ExpandedName::local("current"), |c, args| {
            assert!(args.is_empty());
            Ok(Value::Nodes(vec![*c.initial_node]))
        });
    assert_eq!(
        XPath::compile("$nodes[1]")
            .unwrap()
            .select(&context)
            .unwrap(),
        vec![2]
    );
    assert_eq!(
        XPath::compile("//n[. > $host:cutoff][current() = /root]")
            .unwrap()
            .select(&context)
            .unwrap(),
        vec![3, 4, 5]
    );
}
#[test]
fn limits_and_context_validation() {
    assert!(Context::new(&Tree, 0).with_focus(0, 1).is_err());
    let ctx = Context::new(&Tree, 0).with_limits(Limits {
        max_operations: 2,
        max_depth: 10,
    });
    assert_eq!(
        XPath::compile("//n")
            .unwrap()
            .evaluate(&ctx)
            .unwrap_err()
            .kind,
        ErrorKind::Limit
    );
    // Two AST evaluations exhaust this budget; builtin navigation must share it.
    let ctx = Context::new(&Tree, 5).with_limits(Limits {
        max_operations: 2,
        max_depth: 10,
    });
    assert_eq!(
        XPath::compile("lang('en')")
            .unwrap()
            .evaluate(&ctx)
            .unwrap_err()
            .kind,
        ErrorKind::Limit
    );
}
struct Broken;
impl Navigator for Broken {
    type Node = usize;
    fn kind(&self, _: &usize) -> NodeKind {
        NodeKind::Root
    }
    fn parent(&self, _: &usize) -> Result<Option<usize>> {
        Ok(None)
    }
    fn children<'a>(&'a self, _: &'a usize) -> impl Iterator<Item = Result<usize>> + 'a {
        std::iter::once(Err(Error::new(
            ErrorKind::Navigation,
            "storage unavailable",
        )))
    }
    fn name(&self, _: &usize) -> Result<Option<ExpandedName>> {
        Ok(None)
    }
    fn string_value(&self, _: &usize) -> Result<String> {
        Ok(String::new())
    }
    fn compare_document_order(&self, a: &usize, b: &usize) -> Ordering {
        a.cmp(b)
    }
}
#[test]
fn storage_errors_propagate() {
    let e = XPath::compile("*")
        .unwrap()
        .evaluate(&Context::new(&Broken, 0))
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Navigation);
    assert_eq!(e.message, "storage unavailable");
}

struct Namespaced;
impl Navigator for Namespaced {
    type Node = usize;
    fn kind(&self, n: &usize) -> NodeKind {
        Tree.kind(n)
    }
    fn parent(&self, n: &usize) -> Result<Option<usize>> {
        Tree.parent(n)
    }
    fn children<'a>(&'a self, n: &'a usize) -> impl Iterator<Item = Result<usize>> + 'a {
        Tree.children(n)
    }
    fn name(&self, n: &usize) -> Result<Option<ExpandedName>> {
        Ok(Tree
            .name(n)?
            .map(|name| ExpandedName::new("urn:module", name.local_name)))
    }
    fn string_value(&self, n: &usize) -> Result<String> {
        Tree.string_value(n)
    }
    fn compare_document_order(&self, a: &usize, b: &usize) -> Ordering {
        a.cmp(b)
    }
}
#[test]
fn opt_in_embedding_namespace_preserves_xpath_default() {
    let strict = Context::new(&Namespaced, 0);
    assert!(XPath::compile("/root/n")
        .unwrap()
        .select(&strict)
        .unwrap()
        .is_empty());
    let yang = strict
        .with_default_element_namespace("urn:module")
        .with_namespace("m", "urn:module")
        .with_variable("count", Value::Number(2.0));
    assert_eq!(
        XPath::compile("/root/n").unwrap().select(&yang).unwrap(),
        vec![2, 3]
    );
    assert_eq!(
        XPath::compile("count(/m:root/n) = $count")
            .unwrap()
            .evaluate(&yang)
            .unwrap(),
        Value::Boolean(true)
    );
}
