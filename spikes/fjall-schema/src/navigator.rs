use crate::{FlatNode, Result, SchemaId, SchemaSnapshot, View};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    sync::Arc,
};
use xpath1::{Context, ExpandedName, Navigator, NodeKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
enum Part {
    Element,
    Namespace(usize),
    Attribute(u8),
}

/// A checked handle caches one decoded row, so identity, kind and order never
/// perform storage I/O. Handles are scoped to a projection and a tree view.
#[derive(Clone, Debug)]
pub struct NodeHandle {
    schema: SchemaId,
    view: View,
    row: Arc<FlatNode>,
    part: Part,
}
impl NodeHandle {
    pub fn row(&self) -> &FlatNode {
        &self.row
    }
    pub fn schema_id(&self) -> SchemaId {
        self.schema
    }
    pub fn is_schema_node(&self) -> bool {
        self.part == Part::Element
    }
    fn with_part(&self, part: Part) -> Self {
        Self {
            part,
            ..self.clone()
        }
    }
}
impl PartialEq for NodeHandle {
    fn eq(&self, other: &Self) -> bool {
        (self.schema, self.view, self.row.node.id, self.part)
            == (other.schema, other.view, other.row.node.id, other.part)
    }
}
impl Eq for NodeHandle {}
impl Hash for NodeHandle {
    fn hash<H: Hasher>(&self, h: &mut H) {
        (self.schema, self.view, self.row.node.id, self.part).hash(h);
    }
}

/// Adapts a flat compiled-schema snapshot to the existing generic XPath engine.
/// Elements denote schema declarations; unqualified attributes expose metadata.
/// Schema elements have empty string values, not data-instance values.
pub struct SchemaView {
    snapshot: SchemaSnapshot,
    mode: View,
    bindings: Vec<(String, String)>,
    module_aliases: BTreeMap<String, String>,
}
impl SchemaView {
    pub(crate) fn new(snapshot: SchemaSnapshot, mode: View) -> Self {
        let mut used: BTreeSet<_> = snapshot
            .modules()
            .iter()
            .flat_map(|m| [m.name.as_str(), m.prefix.as_str()])
            .collect();
        used.insert("xml");
        used.insert("xmlns");
        let mut bindings = BTreeMap::new();
        let mut module_aliases = BTreeMap::new();
        for module in snapshot.modules() {
            let alias = if matches!(module.name.as_str(), "xml" | "xmlns") {
                (0..)
                    .map(|i| format!("_schema{i}"))
                    .find(|alias| !used.contains(alias.as_str()) && !bindings.contains_key(alias))
                    .unwrap()
            } else {
                module.name.clone()
            };
            module_aliases.insert(module.name.clone(), alias.clone());
            bindings.insert(alias, module.namespace.clone());
        }
        let mut aliases: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for module in snapshot.modules() {
            aliases
                .entry(&module.prefix)
                .or_default()
                .push(&module.namespace);
        }
        for (prefix, namespaces) in aliases {
            if namespaces.len() == 1 && !matches!(prefix, "xml" | "xmlns") {
                bindings
                    .entry(prefix.into())
                    .or_insert_with(|| namespaces[0].into());
            }
        }
        bindings.insert("xml".into(), "http://www.w3.org/XML/1998/namespace".into());
        Self {
            snapshot,
            mode,
            bindings: bindings.into_iter().collect(),
            module_aliases,
        }
    }
    pub fn snapshot(&self) -> &SchemaSnapshot {
        &self.snapshot
    }
    pub fn mode(&self) -> View {
        self.mode
    }
    pub fn node(&self, id: u32) -> Result<NodeHandle> {
        let row = self.snapshot.row(id)?;
        if !self.mode.visible(row.node.kind) {
            return Err(crate::Error::Invalid("node is hidden in this view".into()));
        }
        Ok(self.handle(row))
    }
    fn handle(&self, row: FlatNode) -> NodeHandle {
        NodeHandle {
            schema: self.snapshot.id(),
            view: self.mode,
            row: Arc::new(row),
            part: Part::Element,
        }
    }
    pub fn context(&self) -> Result<Context<'_, Self>> {
        self.context_at(0)
    }
    pub fn context_at(&self, id: u32) -> Result<Context<'_, Self>> {
        let mut context = Context::new(self, self.node(id)?);
        for (prefix, uri) in &self.bindings {
            context = context.with_namespace(prefix, uri);
        }
        Ok(context)
    }
    /// Unambiguous module prefixes plus canonical module-name aliases. Callers
    /// may bind their own aliases on Context when declared prefixes collide.
    pub fn namespace_bindings(&self) -> &[(String, String)] {
        &self.bindings
    }

    fn attribute(&self, node: &NodeHandle, slot: u8) -> Option<String> {
        let row = &node.row;
        let n = &row.node;
        Some(match slot {
            0 => n.id.to_string(),
            1 => n.kind.as_str().into(),
            2 => n.module.clone(),
            3 => n.namespace.clone(),
            4 => n.config?.to_string(),
            5 => n.key.to_string(),
            6 => n.mandatory.to_string(),
            7 => n.presence.to_string(),
            8 => n.status.clone(),
            9 => n.type_name.clone()?,
            10 => (!n.when.is_empty()).to_string(),
            11 => (!n.must.is_empty()).to_string(),
            12 => n.when.len().to_string(),
            13 => n.must.len().to_string(),
            14 => {
                if self.mode == View::Compiled {
                    row.compiled_path.clone()
                } else {
                    row.data_path.clone()
                }
            }
            15 => row.compiled_path.clone(),
            16 => n.name.clone(),
            _ => return None,
        })
    }
}

const ATTRIBUTES: &[&str] = &[
    "id",
    "kind",
    "module",
    "namespace",
    "config",
    "key",
    "mandatory",
    "presence",
    "status",
    "type",
    "has-when",
    "has-must",
    "when-count",
    "must-count",
    "path",
    "compiled-path",
    "name",
];
fn navigation(error: impl std::fmt::Display) -> xpath1::Error {
    xpath1::Error::new(xpath1::ErrorKind::Navigation, error.to_string())
}
type Axis<'a> = Box<dyn Iterator<Item = xpath1::Result<NodeHandle>> + 'a>;

impl Navigator for SchemaView {
    type Node = NodeHandle;
    fn validate_node(&self, node: &NodeHandle) -> xpath1::Result<()> {
        if node.schema != self.snapshot.id()
            || node.view != self.mode
            || node.row.node.id >= self.snapshot.node_count()
            || !self.mode.visible(node.row.node.kind)
        {
            return Err(navigation(
                "handle belongs to a different projection or view",
            ));
        }
        Ok(())
    }
    fn kind(&self, node: &NodeHandle) -> NodeKind {
        match node.part {
            Part::Element if node.row.node.id == 0 => NodeKind::Root,
            Part::Element => NodeKind::Element,
            Part::Attribute(_) => NodeKind::Attribute,
            Part::Namespace(_) => NodeKind::Namespace,
        }
    }
    fn parent(&self, node: &NodeHandle) -> xpath1::Result<Option<NodeHandle>> {
        self.validate_node(node)?;
        if node.part != Part::Element {
            return Ok(Some(node.with_part(Part::Element)));
        }
        node.row
            .parent(self.mode)
            .map(|id| self.node(id).map_err(navigation))
            .transpose()
    }
    fn children<'a>(
        &'a self,
        node: &'a NodeHandle,
    ) -> impl Iterator<Item = xpath1::Result<NodeHandle>> + 'a {
        let iter: Axis<'a> = match self.validate_node(node) {
            Err(e) => Box::new(std::iter::once(Err(e))),
            Ok(()) if node.part == Part::Element => Box::new(
                self.snapshot
                    .children(self.mode, node.row.node.id)
                    .map(|row| row.map(|r| self.handle(r)).map_err(navigation)),
            ),
            _ => Box::new(std::iter::empty()),
        };
        iter
    }
    fn attributes<'a>(
        &'a self,
        node: &'a NodeHandle,
    ) -> impl Iterator<Item = xpath1::Result<NodeHandle>> + 'a {
        let iter: Axis<'a> = match self.validate_node(node) {
            Err(e) => Box::new(std::iter::once(Err(e))),
            Ok(()) if self.kind(node) == NodeKind::Element => Box::new(
                (0..ATTRIBUTES.len() as u8)
                    // Presence checks must not allocate every attribute's string
                    // value. Only two metadata attributes are optional.
                    .filter(|&slot| match slot {
                        4 => node.row.node.config.is_some(),
                        9 => node.row.node.type_name.is_some(),
                        _ => true,
                    })
                    .map(|slot| Ok(node.with_part(Part::Attribute(slot)))),
            ),
            _ => Box::new(std::iter::empty()),
        };
        iter
    }
    fn namespaces<'a>(
        &'a self,
        node: &'a NodeHandle,
    ) -> impl Iterator<Item = xpath1::Result<NodeHandle>> + 'a {
        let iter: Axis<'a> = match self.validate_node(node) {
            Err(e) => Box::new(std::iter::once(Err(e))),
            Ok(()) if self.kind(node) == NodeKind::Element => Box::new(
                (0..self.bindings.len()).map(|slot| Ok(node.with_part(Part::Namespace(slot)))),
            ),
            _ => Box::new(std::iter::empty()),
        };
        iter
    }
    fn name(&self, node: &NodeHandle) -> xpath1::Result<Option<ExpandedName>> {
        self.validate_node(node)?;
        Ok(match node.part {
            Part::Element if node.row.node.id == 0 => None,
            Part::Element => Some(ExpandedName::new(
                &node.row.node.namespace,
                &node.row.node.name,
            )),
            Part::Attribute(slot) => Some(ExpandedName::local(ATTRIBUTES[slot as usize])),
            Part::Namespace(slot) => Some(ExpandedName::local(&self.bindings[slot].0)),
        })
    }
    fn qualified_name(&self, node: &NodeHandle) -> xpath1::Result<String> {
        self.validate_node(node)?;
        Ok(match node.part {
            Part::Element if node.row.node.id == 0 => String::new(),
            Part::Element => format!(
                "{}:{}",
                self.module_aliases[&node.row.node.module], node.row.node.name
            ),
            Part::Attribute(slot) => ATTRIBUTES[slot as usize].into(),
            Part::Namespace(slot) => self.bindings[slot].0.clone(),
        })
    }
    fn string_value(&self, node: &NodeHandle) -> xpath1::Result<String> {
        self.validate_node(node)?;
        Ok(match node.part {
            Part::Element => String::new(),
            Part::Attribute(slot) => self
                .attribute(node, slot)
                .expect("only present attributes get handles"),
            Part::Namespace(slot) => self.bindings[slot].1.clone(),
        })
    }
    fn compare_document_order(&self, a: &NodeHandle, b: &NodeHandle) -> Ordering {
        (a.row.node.id, a.part).cmp(&(b.row.node.id, b.part))
    }
}
