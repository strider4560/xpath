//! An owned XML snapshot implementing the XPath data model.
//!
//! Internal DTD entities are expanded by roxmltree. External DTDs are not
//! fetched, external entities are not resolved, and DTD ID typing is not
//! inferred. Register application-known IDs explicitly with `register_id`.
use crate::{Error, ErrorKind, ExpandedName, Navigator, NodeKind, Result};
use std::{
    cmp::Ordering,
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
};

static NEXT_DOCUMENT: AtomicU64 = AtomicU64::new(1);
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// Stable handle whose identity includes its owning document.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct NodeId {
    document: u64,
    index: usize,
}

#[derive(Debug)]
struct Node {
    kind: NodeKind,
    parent: Option<usize>,
    children: Vec<usize>,
    attributes: Vec<usize>,
    namespaces: Vec<usize>,
    name: Option<ExpandedName>,
    qualified_name: String,
    value: String,
}

/// An immutable arena, except for its explicit application ID index.
#[derive(Debug)]
pub struct Document {
    identity: u64,
    nodes: Vec<Node>,
    ids: HashMap<String, NodeId>,
}

impl Document {
    /// Parse XML with internal entities and a limit of 256 expanded element
    /// levels. Excessive nesting returns [`ErrorKind::Limit`] before entering
    /// the XML tokenizer. External resources are never fetched.
    pub fn parse(source: &str) -> Result<Self> {
        check_depth(source)?;
        let parsed = roxmltree::Document::parse_with_options(
            source,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .map_err(|e| Error::new(ErrorKind::Navigation, format!("XML parse error: {e}")))?;
        let identity = NEXT_DOCUMENT.fetch_add(1, AtomicOrdering::Relaxed);
        assert_ne!(identity, 0, "XML document identity exhausted");
        let mut result = Self {
            identity,
            nodes: Vec::new(),
            ids: HashMap::new(),
        };
        // Preorder traversal keeps arena indices in document order without
        // growing the Rust call stack with XML nesting depth.
        let mut pending: Vec<(roxmltree::Node<'_, '_>, Option<usize>)> =
            vec![(parsed.root(), None)];
        while let Some((source_node, parent)) = pending.pop() {
            if source_node.is_text() {
                if let Some(parent) = parent {
                    if let Some(&last) = result.nodes[parent].children.last() {
                        if result.nodes[last].kind == NodeKind::Text {
                            result.nodes[last]
                                .value
                                .push_str(source_node.text().unwrap_or(""));
                            continue;
                        }
                    }
                }
            }
            let index = result.import_node(source_node, parent, source);
            if let Some(parent) = parent {
                result.nodes[parent].children.push(index);
            }
            pending.extend(source_node.children().rev().map(|n| (n, Some(index))));
        }
        Ok(result)
    }

    pub fn root(&self) -> NodeId {
        self.handle(0)
    }

    /// Test ownership without dereferencing a handle, useful for composite navigators.
    pub fn contains_node(&self, node: &NodeId) -> bool {
        node.document == self.identity && node.index < self.nodes.len()
    }

    pub fn register_id(&mut self, id: impl Into<String>, node: NodeId) -> Result<()> {
        if self.get(&node)?.kind != NodeKind::Element {
            return Err(Error::new(
                ErrorKind::Navigation,
                "an XML ID must identify an element",
            ));
        }
        self.ids.insert(id.into(), node);
        Ok(())
    }

    fn handle(&self, index: usize) -> NodeId {
        NodeId {
            document: self.identity,
            index,
        }
    }
    fn get(&self, node: &NodeId) -> Result<&Node> {
        if !self.contains_node(node) {
            return Err(Error::new(
                ErrorKind::Navigation,
                "node belongs to a different XML document",
            ));
        }
        Ok(&self.nodes[node.index])
    }
    fn push(
        &mut self,
        kind: NodeKind,
        parent: Option<usize>,
        name: Option<ExpandedName>,
        qualified_name: String,
        value: String,
    ) -> usize {
        let index = self.nodes.len();
        self.nodes.push(Node {
            kind,
            parent,
            name,
            qualified_name,
            value,
            children: Vec::new(),
            attributes: Vec::new(),
            namespaces: Vec::new(),
        });
        index
    }
    fn import_node(
        &mut self,
        source_node: roxmltree::Node<'_, '_>,
        parent: Option<usize>,
        source: &str,
    ) -> usize {
        let (kind, name, qualified_name, value) = match source_node.node_type() {
            roxmltree::NodeType::Root => (NodeKind::Root, None, String::new(), String::new()),
            roxmltree::NodeType::Element => {
                let tag = source_node.tag_name();
                (
                    NodeKind::Element,
                    Some(ExpandedName::new(tag.namespace().unwrap_or(""), tag.name())),
                    lexical_name(&source[source_node.range().start..], true),
                    String::new(),
                )
            }
            roxmltree::NodeType::Text => (
                NodeKind::Text,
                None,
                String::new(),
                source_node.text().unwrap_or("").to_owned(),
            ),
            roxmltree::NodeType::Comment => (
                NodeKind::Comment,
                None,
                String::new(),
                source_node.text().unwrap_or("").to_owned(),
            ),
            roxmltree::NodeType::PI => {
                let pi = source_node.pi().expect("PI node has PI data");
                (
                    NodeKind::ProcessingInstruction,
                    Some(ExpandedName::local(pi.target)),
                    pi.target.to_owned(),
                    pi.value.unwrap_or("").to_owned(),
                )
            }
        };
        let index = self.push(kind, parent, name, qualified_name, value);
        if kind == NodeKind::Element {
            // roxmltree includes inherited namespaces. Undeclared default namespaces
            // have no XPath namespace node; xml is implicit on every element.
            let mut namespaces: Vec<(String, String)> = source_node
                .namespaces()
                .map(|ns| (ns.name().unwrap_or("").to_owned(), ns.uri().to_owned()))
                .filter(|(_, uri)| !uri.is_empty())
                .collect();
            if !namespaces.iter().any(|(prefix, _)| prefix == "xml") {
                namespaces.push(("xml".into(), XML_NAMESPACE.into()));
            }
            namespaces.sort();
            for (prefix, uri) in namespaces {
                let ns = self.push(
                    NodeKind::Namespace,
                    Some(index),
                    Some(ExpandedName::local(&prefix)),
                    prefix,
                    uri,
                );
                self.nodes[index].namespaces.push(ns);
            }
            for attr in source_node.attributes() {
                let qname = lexical_name(&source[attr.range().start..], false);
                let a = self.push(
                    NodeKind::Attribute,
                    Some(index),
                    Some(ExpandedName::new(
                        attr.namespace().unwrap_or(""),
                        attr.name(),
                    )),
                    qname,
                    attr.value().to_owned(),
                );
                self.nodes[index].attributes.push(a);
            }
        }
        index
    }

    fn axis<'a>(
        &'a self,
        node: &'a NodeId,
        select: fn(&Node) -> &[usize],
    ) -> impl Iterator<Item = Result<NodeId>> + 'a {
        let (indices, error) = match self.get(node) {
            Ok(n) => (select(n), None),
            Err(e) => (&[][..], Some(e)),
        };
        error
            .into_iter()
            .map(Err)
            .chain(indices.iter().map(|&i| Ok(self.handle(i))))
    }
}

// roxmltree 0.20 tokenizes nested elements recursively. Inspect the expanded
// markup first with an explicit stack, so excessive XML nesting is a Result
// error rather than a process-aborting stack overflow. This is a depth guard,
// not an XML validator; malformed syntax remains roxmltree's responsibility.
const MAX_XML_DEPTH: usize = 256;
fn check_depth(source: &str) -> Result<()> {
    let mut entities: HashMap<&str, &str> = HashMap::new();
    let mut pending = vec![(source, 0, 0)];
    let mut depth = 0usize;
    let mut references = 0usize;
    while let Some((fragment, mut pos, entity_depth)) = pending.pop() {
        let bytes = fragment.as_bytes();
        while pos < bytes.len() {
            if bytes[pos] == b'<' {
                let rest = &fragment[pos..];
                let marker = if rest.starts_with("<!--") {
                    Some(("-->", 4))
                } else if rest.starts_with("<![CDATA[") {
                    Some(("]]>", 9))
                } else if rest.starts_with("<?") {
                    Some(("?>", 2))
                } else {
                    None
                };
                if let Some((end, start)) = marker {
                    pos = rest[start..]
                        .find(end)
                        .map(|i| pos + start + i + end.len())
                        .unwrap_or(bytes.len());
                    continue;
                }
                if rest.starts_with("<!") {
                    let end = declaration_end(fragment, pos);
                    if rest.starts_with("<!DOCTYPE") {
                        collect_entities(&fragment[pos..end], &mut entities);
                    }
                    pos = end;
                    continue;
                }
                let end = tag_end(fragment, pos);
                if rest.starts_with("</") {
                    depth = depth.saturating_sub(1);
                } else {
                    let tag = &fragment[pos..end];
                    // Count empty elements too: the parser creates them inside
                    // their parent call before returning immediately.
                    if depth >= MAX_XML_DEPTH {
                        return Err(Error::new(
                            ErrorKind::Limit,
                            "XML nesting exceeds 256 elements",
                        ));
                    }
                    if !tag.trim_end_matches('>').trim_end().ends_with('/') {
                        depth += 1;
                    }
                }
                pos = end;
            } else if bytes[pos] == b'&' {
                let rest = &fragment[pos + 1..];
                if let Some(end) = rest.find(';') {
                    let name = &rest[..end];
                    pos += end + 2;
                    if let Some(&replacement) = entities.get(name) {
                        if entity_depth == 0 {
                            references = 0;
                        } else {
                            references += 1;
                            if references > 255 {
                                return Err(Error::new(
                                    ErrorKind::Navigation,
                                    "XML entity expansion exceeds parser limit",
                                ));
                            }
                        }
                        // Match roxmltree's entity recursion bound, while the
                        // expanded markup depth is checked independently.
                        if entity_depth >= 10 {
                            return Err(Error::new(
                                ErrorKind::Navigation,
                                "XML entity nesting exceeds parser limit",
                            ));
                        }
                        pending.push((fragment, pos, entity_depth));
                        pending.push((replacement, 0, entity_depth + 1));
                        break;
                    }
                } else {
                    pos += 1;
                }
            } else {
                pos += 1;
            }
        }
    }
    Ok(())
}

fn tag_end(source: &str, start: usize) -> usize {
    let mut quote = None;
    for (offset, byte) in source.as_bytes()[start..].iter().copied().enumerate() {
        match (quote, byte) {
            (Some(q), b) if q == b => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'>') => return start + offset + 1,
            _ => {}
        }
    }
    source.len()
}

fn declaration_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut pos = start;
    let mut quote = None;
    let mut brackets = 0usize;
    while pos < bytes.len() {
        if quote.is_none() && bytes[pos..].starts_with(b"<!--") {
            pos = source[pos + 4..]
                .find("-->")
                .map(|i| pos + i + 7)
                .unwrap_or(bytes.len());
            continue;
        }
        if quote.is_none() && bytes[pos..].starts_with(b"<?") {
            pos = source[pos + 2..]
                .find("?>")
                .map(|i| pos + i + 4)
                .unwrap_or(bytes.len());
            continue;
        }
        match (quote, bytes[pos]) {
            (Some(q), b) if q == b => quote = None,
            (None, b'\'' | b'"') => quote = Some(bytes[pos]),
            (None, b'[') => brackets += 1,
            (None, b']') => brackets = brackets.saturating_sub(1),
            (None, b'>') if brackets == 0 => return pos + 1,
            _ => {}
        }
        pos += 1;
    }
    source.len()
}

fn collect_entities<'a>(declaration: &'a str, entities: &mut HashMap<&'a str, &'a str>) {
    // Only the internal subset contains declarations. The external identifier
    // in the DOCTYPE header can contain arbitrary quoted text.
    let mut quote = None;
    let mut start = None;
    for (pos, byte) in declaration.bytes().enumerate() {
        match (quote, byte) {
            (Some(q), b) if q == b => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'[') => {
                start = Some(pos + 1);
                break;
            }
            _ => {}
        }
    }
    let Some(mut pos) = start else {
        return;
    };
    while pos < declaration.len() {
        let rest = &declaration[pos..];
        if rest.starts_with(']') {
            break;
        }
        if let Some(comment) = rest.strip_prefix("<!--") {
            pos = comment
                .find("-->")
                .map(|i| pos + i + 7)
                .unwrap_or(declaration.len());
        } else if let Some(pi) = rest.strip_prefix("<?") {
            pos = pi
                .find("?>")
                .map(|i| pos + i + 4)
                .unwrap_or(declaration.len());
        } else if let Some(definition) = rest.strip_prefix("<!ENTITY") {
            let end = tag_end(declaration, pos);
            let definition = definition[..end - pos - 8].trim_start();
            let name_end = definition
                .find(char::is_whitespace)
                .unwrap_or(definition.len());
            let name = &definition[..name_end];
            let value = definition[name_end..].trim_start();
            if let Some(quote @ ('\'' | '"')) = value.chars().next() {
                if let Some(end) = value[1..].find(quote) {
                    entities.entry(name).or_insert(&value[1..end + 1]);
                }
            }
            pos = end;
        } else if rest.starts_with("<!") {
            // Ignore ELEMENT, ATTLIST and NOTATION declarations as complete
            // lexical units, including their quoted contents.
            pos = tag_end(declaration, pos);
        } else if let Some(quote @ ('\'' | '"')) = rest.chars().next() {
            pos = rest[1..]
                .find(quote)
                .map(|i| pos + i + 2)
                .unwrap_or(declaration.len());
        } else {
            // Advance by a Unicode scalar to preserve slice boundaries.
            pos += rest.chars().next().unwrap().len_utf8();
        }
    }
}

// URI-to-prefix lookup cannot preserve the original QName when two prefixes
// bind the same URI. roxmltree source ranges retain that lexical spelling.
fn lexical_name(source: &str, element: bool) -> String {
    let source = if element {
        source.strip_prefix('<').unwrap_or(source)
    } else {
        source
    };
    source
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '=' | '/' | '>'))
        .collect()
}

impl Navigator for Document {
    type Node = NodeId;
    fn validate_node(&self, node: &NodeId) -> Result<()> {
        self.get(node).map(|_| ())
    }
    fn kind(&self, node: &NodeId) -> NodeKind {
        self.get(node).expect("invalid XML node handle").kind
    }
    fn parent(&self, node: &NodeId) -> Result<Option<NodeId>> {
        Ok(self.get(node)?.parent.map(|i| self.handle(i)))
    }
    fn children<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.axis(node, |n| &n.children)
    }
    fn attributes<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.axis(node, |n| &n.attributes)
    }
    fn namespaces<'a>(&'a self, node: &'a NodeId) -> impl Iterator<Item = Result<NodeId>> + 'a {
        self.axis(node, |n| &n.namespaces)
    }
    fn name(&self, node: &NodeId) -> Result<Option<ExpandedName>> {
        Ok(self.get(node)?.name.clone())
    }
    fn name_matches(&self, node: &NodeId, name: &ExpandedName) -> bool {
        matches!(self.get(node), Ok(Node { name: Some(actual), .. }) if actual == name)
    }
    fn qualified_name(&self, node: &NodeId) -> Result<String> {
        Ok(self.get(node)?.qualified_name.clone())
    }
    fn string_value(&self, node: &NodeId) -> Result<String> {
        let n = self.get(node)?;
        if !matches!(n.kind, NodeKind::Root | NodeKind::Element) {
            return Ok(n.value.clone());
        }
        let mut value = String::new();
        let mut pending: Vec<_> = n.children.iter().rev().copied().collect();
        while let Some(index) = pending.pop() {
            let descendant = &self.nodes[index];
            if descendant.kind == NodeKind::Text {
                value.push_str(&descendant.value);
            }
            pending.extend(descendant.children.iter().rev().copied());
        }
        Ok(value)
    }
    fn compare_document_order(&self, a: &NodeId, b: &NodeId) -> Ordering {
        self.get(a).expect("invalid XML node handle");
        self.get(b).expect("invalid XML node handle");
        a.index.cmp(&b.index)
    }
    fn element_by_id(&self, context: &NodeId, id: &str) -> Result<Option<NodeId>> {
        self.get(context)?;
        Ok(self.ids.get(id).copied())
    }
}
