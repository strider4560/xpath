use std::{cell::Cell, cmp::Ordering};
use xpath1::{Context, Error, ErrorKind, Limits, Navigator, NodeKind, Result, Value, XPath};

struct Strings<'a> {
    values: Vec<&'a str>,
    reads: Cell<usize>,
    fail_at: Option<usize>,
}

impl Navigator for Strings<'_> {
    type Node = usize;
    fn kind(&self, _: &usize) -> NodeKind {
        NodeKind::Element
    }
    fn parent(&self, _: &usize) -> Result<Option<usize>> {
        Ok(None)
    }
    fn children<'a>(&'a self, _: &'a usize) -> impl Iterator<Item = Result<usize>> + 'a {
        std::iter::empty()
    }
    fn name(&self, _: &usize) -> Result<Option<xpath1::ExpandedName>> {
        Ok(None)
    }
    fn string_value(&self, node: &usize) -> Result<String> {
        self.reads.set(self.reads.get() + 1);
        if self.fail_at == Some(*node) {
            return Err(Error::new(ErrorKind::Navigation, "cannot read leaf"));
        }
        Ok(self.values[*node].into())
    }
    fn compare_document_order(&self, a: &usize, b: &usize) -> Ordering {
        a.cmp(b)
    }
}

fn evaluate(nav: &Strings<'_>, source: &str, split: usize) -> Result<bool> {
    let context = Context::new(nav, 0)
        .with_variable("a", Value::Nodes((0..split).collect()))
        .with_variable("b", Value::Nodes((split..nav.values.len()).collect()));
    Ok(XPath::compile(source)?.evaluate(&context)?.to_boolean())
}

#[test]
fn comparisons_are_existential_for_all_small_string_sets() {
    // Exhaustively compare both operators against their definition, with empty
    // sets, duplicate string values, empty strings, non-ASCII, and lexical
    // numbers (node-set equality compares strings, not numbers).
    let alphabet = ["", "same", "01", "1", "é"];
    let mut sets = vec![Vec::new()];
    for a in alphabet {
        sets.push(vec![a]);
        for b in alphabet {
            sets.push(vec![a, b]);
        }
    }
    for left in &sets {
        for right in &sets {
            let nav = Strings {
                values: left.iter().chain(right).copied().collect(),
                reads: Cell::new(0),
                fail_at: None,
            };
            assert_eq!(
                evaluate(&nav, "$a = $b", left.len()).unwrap(),
                left.iter().any(|a| right.iter().any(|b| a == b))
            );
            assert_eq!(
                evaluate(&nav, "$a != $b", left.len()).unwrap(),
                left.iter().any(|a| right.iter().any(|b| a != b))
            );
        }
    }
}

#[test]
fn each_leaf_string_is_read_at_most_once_per_comparison() {
    for (left, right, expression, expected) in [
        ("left", "right", "$a = $b", false),
        ("same", "same", "$a != $b", false),
    ] {
        let nav = Strings {
            values: [vec![left; 128], vec![right; 128]].concat(),
            reads: Cell::new(0),
            fail_at: None,
        };
        assert_eq!(evaluate(&nav, expression, 128).unwrap(), expected);
        assert!(
            nav.reads.get() <= nav.values.len(),
            "{} string reads",
            nav.reads.get()
        );
    }
}

#[test]
fn comparison_short_circuits_and_propagates_reached_errors() {
    for (source, values) in [
        ("$a = $b", vec!["match", "unread", "match", "unread"]),
        ("$a != $b", vec!["first", "unread", "different", "unread"]),
    ] {
        let nav = Strings {
            values,
            reads: Cell::new(0),
            fail_at: Some(1),
        };
        assert!(evaluate(&nav, source, 2).unwrap());
        assert_eq!(nav.reads.get(), 2);
    }
    for source in ["$a = $b", "$a != $b"] {
        let nav = Strings {
            values: vec!["x", "y"],
            reads: Cell::new(0),
            fail_at: Some(1),
        };
        let error = evaluate(&nav, source, 1).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Navigation);
        assert_eq!(error.message, "cannot read leaf");
    }
    // After exhausting the first comparison row, a later left-hand read can
    // still fail. An index of right-hand strings must not hide that error.
    let nav = Strings {
        values: vec!["x", "x", "y", "y"],
        reads: Cell::new(0),
        fail_at: Some(1),
    };
    assert_eq!(
        evaluate(&nav, "$a = $b", 2).unwrap_err().kind,
        ErrorKind::Navigation
    );
    let nav = Strings {
        values: vec!["x"; 4],
        reads: Cell::new(0),
        fail_at: Some(1),
    };
    assert_eq!(
        evaluate(&nav, "$a != $b", 2).unwrap_err().kind,
        ErrorKind::Navigation
    );
}

#[test]
fn comparison_string_reads_share_the_evaluation_budget() {
    let nav = Strings {
        values: vec!["x"; 16],
        reads: Cell::new(0),
        fail_at: None,
    };
    let context = Context::new(&nav, 0)
        .with_variable("a", Value::Nodes((0..8).collect()))
        .with_variable("b", Value::Nodes((8..16).collect()))
        .with_limits(Limits {
            max_operations: 20,
            max_depth: 16,
            ..Limits::default()
        });
    let error = XPath::compile("$a != $b")
        .unwrap()
        .evaluate(&context)
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    assert!(nav.reads.get() <= 2);
}
