use std::{cell::Cell, cmp::Ordering};
use xpath1::{ExpandedName, Navigator, NodeKind, Result};

pub const NS: &str = "urn:example:interfaces";
pub const SELECT: &str = "/if:interfaces/if:interface[if:name = $wanted]";
pub const WHEN: &str = "../enabled = 'true' and ../kind = 'ethernet'";

struct Entry {
    parent: Option<usize>,
    children: Vec<usize>,
    name: &'static str,
    value: String,
}

/// An application-owned, typed-leaf tree, independent of an XML DOM. Integer
/// handles and preorder insertion are representative of a YANG arena adapter.
pub struct Tree {
    nodes: Vec<Entry>,
    pub mtu: Vec<usize>,
}

impl Tree {
    pub fn interfaces(count: usize) -> Self {
        let mut tree = Self {
            nodes: Vec::new(),
            mtu: Vec::with_capacity(count),
        };
        tree.add(None, "", String::new());
        tree.add(Some(0), "interfaces", String::new());
        for index in 0..count {
            let interface = tree.add(Some(1), "interface", String::new());
            tree.add(Some(interface), "name", format!("eth{index}"));
            tree.add(
                Some(interface),
                "enabled",
                if index % 2 == 0 { "true" } else { "false" }.into(),
            );
            tree.add(Some(interface), "kind", "ethernet".into());
            let mtu = tree.add(Some(interface), "mtu", "1500".into());
            tree.mtu.push(mtu);
        }
        tree
    }

    pub fn comparisons(count: usize, uniform: bool, late_match: bool) -> Self {
        let mut tree = Self {
            nodes: Vec::new(),
            mtu: Vec::new(),
        };
        tree.add(None, "", String::new());
        tree.add(Some(0), "root", String::new());
        for index in 0..count {
            tree.add(
                Some(1),
                "a",
                if uniform {
                    "same".into()
                } else {
                    format!("a{index}")
                },
            );
        }
        for index in 0..count {
            let value = if uniform {
                "same".into()
            } else if late_match && index + 1 == count {
                format!("a{index}")
            } else {
                format!("b{index}")
            };
            tree.add(Some(1), "b", value);
        }
        tree
    }

    fn add(&mut self, parent: Option<usize>, name: &'static str, value: String) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Entry {
            parent,
            children: Vec::new(),
            name,
            value,
        });
        if let Some(parent) = parent {
            self.nodes[parent].children.push(id);
        }
        id
    }
}

impl Navigator for Tree {
    type Node = usize;

    fn kind(&self, node: &usize) -> NodeKind {
        if *node == 0 {
            NodeKind::Root
        } else {
            NodeKind::Element
        }
    }
    fn parent(&self, node: &usize) -> Result<Option<usize>> {
        Ok(self.nodes[*node].parent)
    }
    fn children<'a>(&'a self, node: &'a usize) -> impl Iterator<Item = Result<usize>> + 'a {
        self.nodes[*node].children.iter().copied().map(Ok)
    }
    fn name(&self, node: &usize) -> Result<Option<ExpandedName>> {
        Ok((*node != 0).then(|| ExpandedName::new(NS, self.nodes[*node].name)))
    }
    fn string_value(&self, node: &usize) -> Result<String> {
        let mut value = String::new();
        let mut pending = vec![*node];
        while let Some(node) = pending.pop() {
            value.push_str(&self.nodes[node].value);
            pending.extend(self.nodes[node].children.iter().rev());
        }
        Ok(value)
    }
    fn compare_document_order(&self, a: &usize, b: &usize) -> Ordering {
        a.cmp(b)
    }
}

/// Diagnostic call counts are collected separately from timing runs, so Cell
/// updates do not change the measured hot paths.
pub struct Counted<'a> {
    pub tree: &'a Tree,
    pub names: Cell<usize>,
    pub strings: Cell<usize>,
    pub order: Cell<usize>,
    pub children: Cell<usize>,
}

impl<'a> Counted<'a> {
    pub fn new(tree: &'a Tree) -> Self {
        Self {
            tree,
            names: Cell::new(0),
            strings: Cell::new(0),
            order: Cell::new(0),
            children: Cell::new(0),
        }
    }
}

impl Navigator for Counted<'_> {
    type Node = usize;
    fn kind(&self, node: &usize) -> NodeKind {
        self.tree.kind(node)
    }
    fn parent(&self, node: &usize) -> Result<Option<usize>> {
        self.tree.parent(node)
    }
    fn children<'a>(&'a self, node: &'a usize) -> impl Iterator<Item = Result<usize>> + 'a {
        self.tree
            .children(node)
            .inspect(|_| self.children.set(self.children.get() + 1))
    }
    fn name(&self, node: &usize) -> Result<Option<ExpandedName>> {
        self.names.set(self.names.get() + 1);
        self.tree.name(node)
    }
    fn string_value(&self, node: &usize) -> Result<String> {
        self.strings.set(self.strings.get() + 1);
        self.tree.string_value(node)
    }
    fn compare_document_order(&self, a: &usize, b: &usize) -> Ordering {
        self.order.set(self.order.get() + 1);
        self.tree.compare_document_order(a, b)
    }
}
