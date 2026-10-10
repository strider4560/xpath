//! The same interface can read records from a stable K/V transaction.
use std::cmp::Ordering;
use xpath1::{Context, ExpandedName, Navigator, NodeKind, Result, Value, XPath};

struct Record {
    parent: Option<usize>,
    children: &'static [usize],
    name: &'static str,
    text: &'static str,
}
struct Snapshot(Vec<Record>);
impl Navigator for Snapshot {
    type Node = usize;
    fn validate_node(&self, node: &usize) -> Result<()> {
        if *node < self.0.len() {
            Ok(())
        } else {
            Err(xpath1::Error::new(
                xpath1::ErrorKind::Navigation,
                "unknown record",
            ))
        }
    }
    fn kind(&self, node: &usize) -> NodeKind {
        match *node {
            0 => NodeKind::Root,
            3 | 5 => NodeKind::Text,
            _ => NodeKind::Element,
        }
    }
    fn parent(&self, node: &usize) -> Result<Option<usize>> {
        Ok(self.0[*node].parent)
    }
    fn children<'a>(&'a self, node: &'a usize) -> impl Iterator<Item = Result<usize>> + 'a {
        self.0[*node].children.iter().copied().map(Ok)
    }
    fn name(&self, node: &usize) -> Result<Option<ExpandedName>> {
        Ok((self.kind(node) == NodeKind::Element).then(|| ExpandedName::local(self.0[*node].name)))
    }
    fn string_value(&self, node: &usize) -> Result<String> {
        Ok(self.0[*node].text.into())
    }
    fn compare_document_order(&self, a: &usize, b: &usize) -> Ordering {
        a.cmp(b)
    }
}
fn main() -> Result<()> {
    // IDs are assigned in document order in this tiny example. A K/V adapter can
    // instead compare hierarchical keys encoded in its stable node handles.
    let snapshot = Snapshot(vec![
        Record {
            parent: None,
            children: &[1],
            name: "",
            text: "9100",
        },
        Record {
            parent: Some(0),
            children: &[2, 4],
            name: "inventory",
            text: "9100",
        },
        Record {
            parent: Some(1),
            children: &[3],
            name: "stock",
            text: "9",
        },
        Record {
            parent: Some(2),
            children: &[],
            name: "",
            text: "9",
        },
        Record {
            parent: Some(1),
            children: &[5],
            name: "stock",
            text: "100",
        },
        Record {
            parent: Some(4),
            children: &[],
            name: "",
            text: "100",
        },
    ]);
    let context = Context::new(&snapshot, 0).with_variable("minimum", Value::Number(20.0));
    let expression = XPath::compile("/inventory/stock[. >= $minimum]")?;
    assert_eq!(expression.select(&context)?, vec![4]);
    println!("{}", expression.evaluate(&context)?.to_string(&snapshot)?);
    Ok(())
}
