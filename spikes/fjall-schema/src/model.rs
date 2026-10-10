use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, str::FromStr};

// Serde normally treats a missing Option field like null. The interchange
// requires all fields, so route nullable fields through a field deserializer.
fn required_option<'de, D, T>(d: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(d)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SchemaId(pub(crate) [u8; 32]);

impl fmt::Display for SchemaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
impl FromStr for SchemaId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid(
                "projection ID must contain 64 hex digits".into(),
            ));
        }
        let mut bytes = [0; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(|_| Error::Invalid("invalid projection ID".into()))?;
        }
        Ok(Self(bytes))
    }
}
impl Serialize for SchemaId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for SchemaId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Root,
    Container,
    List,
    Leaf,
    LeafList,
    Choice,
    Case,
    Anydata,
    Anyxml,
    Rpc,
    Action,
    Input,
    Output,
    Notification,
}
impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Container => "container",
            Self::List => "list",
            Self::Leaf => "leaf",
            Self::LeafList => "leaf-list",
            Self::Choice => "choice",
            Self::Case => "case",
            Self::Anydata => "anydata",
            Self::Anyxml => "anyxml",
            Self::Rpc => "rpc",
            Self::Action => "action",
            Self::Input => "input",
            Self::Output => "output",
            Self::Notification => "notification",
        }
    }
    pub fn transparent(self) -> bool {
        matches!(self, Self::Choice | Self::Case)
    }
    pub fn scalar(self) -> bool {
        matches!(self, Self::Leaf | Self::LeafList)
    }
    fn can_have_children(self) -> bool {
        !matches!(
            self,
            Self::Leaf | Self::LeafList | Self::Anydata | Self::Anyxml
        )
    }
}

/// Compiled exposes choice/case rows. DataPaths hides those rows and rewires
/// their children to the nearest visible parent; it is not a data-instance view.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum View {
    Compiled,
    DataPaths,
}
impl View {
    pub(crate) fn code(self) -> u8 {
        if self == Self::Compiled {
            0
        } else {
            1
        }
    }
    pub(crate) fn visible(self, kind: NodeKind) -> bool {
        self == Self::Compiled || !kind.transparent()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub name: String,
    pub namespace: String,
    pub prefix: String,
    #[serde(deserialize_with = "required_option")]
    pub revision: Option<String>,
    pub enabled_features: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeRecord {
    pub id: u32,
    #[serde(deserialize_with = "required_option")]
    pub parent: Option<u32>,
    pub kind: NodeKind,
    pub module: String,
    pub namespace: String,
    pub name: String,
    #[serde(deserialize_with = "required_option")]
    pub config: Option<bool>,
    pub key: bool,
    pub mandatory: bool,
    pub presence: bool,
    pub status: String,
    #[serde(deserialize_with = "required_option")]
    pub type_name: Option<String>,
    pub when: Vec<String>,
    pub must: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Export {
    pub format_version: u32,
    pub compiler: String,
    pub modules: Vec<Module>,
    pub nodes: Vec<NodeRecord>,
}

/// A flat table row. End is an exclusive preorder boundary. The original
/// compiled parent is retained alongside the transparent-choice/case parent.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlatNode {
    pub node: NodeRecord,
    pub subtree_end: u32,
    pub data_parent: Option<u32>,
    pub compiled_path: String,
    pub data_path: String,
}
impl FlatNode {
    pub fn parent(&self, view: View) -> Option<u32> {
        match view {
            View::Compiled => self.node.parent,
            View::DataPaths => self.data_parent,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    pub format_version: u32,
    pub id: SchemaId,
    pub compiler: String,
    pub modules: Vec<Module>,
    pub node_count: u32,
}

fn identifier(s: &str) -> bool {
    let mut bytes = s.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

impl Export {
    pub(crate) fn prepare(&self) -> Result<(Metadata, Vec<FlatNode>)> {
        let invalid = |message: &str| Error::Invalid(message.into());
        if self.format_version != 1 {
            return Err(invalid("unsupported format_version"));
        }
        if self.nodes.is_empty() || self.nodes.len() > 1_000_000 {
            return Err(invalid("expected 1..=1,000,000 schema rows"));
        }
        let mut modules = BTreeMap::new();
        for module in &self.modules {
            if !identifier(&module.name)
                || !identifier(&module.prefix)
                || module.namespace.is_empty()
            {
                return Err(invalid("invalid module identity"));
            }
            if modules.insert(module.name.as_str(), module).is_some() {
                return Err(invalid("duplicate module name"));
            }
            if module.enabled_features.windows(2).any(|w| w[0] >= w[1]) {
                return Err(invalid("enabled_features must be sorted and unique"));
            }
        }
        if self.modules.windows(2).any(|w| w[0].name >= w[1].name) {
            return Err(invalid("modules must be sorted and unique"));
        }
        let root = &self.nodes[0];
        if root.id != 0
            || root.kind != NodeKind::Root
            || root.parent.is_some()
            || !root.module.is_empty()
            || !root.namespace.is_empty()
            || !root.name.is_empty()
            || root.config.is_some()
            || root.key
            || root.mandatory
            || root.presence
            || root.status != "current"
            || root.type_name.is_some()
            || !root.when.is_empty()
            || !root.must.is_empty()
        {
            return Err(invalid("row 0 must be the unnamed virtual root"));
        }

        let count = self.nodes.len() as u32;
        let mut rows: Vec<FlatNode> = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<u32> = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if node.id as usize != index {
                return Err(invalid("IDs must be dense preorder indices"));
            }
            if !matches!(node.status.as_str(), "current" | "deprecated" | "obsolete") {
                return Err(invalid("invalid status"));
            }
            if index != 0 {
                if node.kind == NodeKind::Root || !identifier(&node.name) {
                    return Err(invalid("invalid non-root node name or kind"));
                }
                let module = modules
                    .get(node.module.as_str())
                    .ok_or_else(|| invalid("node module is absent"))?;
                if module.namespace != node.namespace {
                    return Err(invalid("node namespace differs from module"));
                }
                let parent = node
                    .parent
                    .filter(|p| *p < node.id)
                    .ok_or_else(|| invalid("parent must precede node"))?;
                while stack.last() != Some(&parent) {
                    let closed = stack
                        .pop()
                        .ok_or_else(|| invalid("subtree is not contiguous"))?;
                    rows[closed as usize].subtree_end = node.id;
                }
                if !rows[parent as usize].node.kind.can_have_children() {
                    return Err(invalid("terminal schema node has children"));
                }
                if node.key
                    && (node.kind != NodeKind::Leaf
                        || rows[parent as usize].node.kind != NodeKind::List)
                {
                    return Err(invalid("key must be a leaf directly under a list"));
                }
            }
            if node.presence && node.kind != NodeKind::Container {
                return Err(invalid("presence on non-container"));
            }
            if node.kind.scalar() != node.type_name.is_some() {
                return Err(invalid("base type is required exactly for scalar leaves"));
            }
            if node.type_name.as_deref().is_some_and(|name| {
                !matches!(
                    name,
                    "binary"
                        | "bits"
                        | "boolean"
                        | "decimal64"
                        | "empty"
                        | "enumeration"
                        | "identityref"
                        | "instance-identifier"
                        | "int8"
                        | "int16"
                        | "int32"
                        | "int64"
                        | "leafref"
                        | "string"
                        | "uint8"
                        | "uint16"
                        | "uint32"
                        | "uint64"
                        | "union"
                )
            }) {
                return Err(invalid("unknown builtin YANG base type"));
            }
            if stack.len() >= 256 {
                return Err(invalid("schema depth exceeds 256"));
            }
            let data_parent = node.parent.and_then(|p| {
                if rows[p as usize].node.kind.transparent() {
                    rows[p as usize].data_parent
                } else {
                    Some(p)
                }
            });
            let path = |parent: Option<u32>, data: bool| -> String {
                match parent {
                    None => "/".into(),
                    Some(p) => {
                        let parent_path = if data {
                            &rows[p as usize].data_path
                        } else {
                            &rows[p as usize].compiled_path
                        };
                        format!(
                            "{}/{}:{}",
                            parent_path.trim_end_matches('/'),
                            node.module,
                            node.name
                        )
                    }
                }
            };
            rows.push(FlatNode {
                node: node.clone(),
                subtree_end: count,
                data_parent,
                compiled_path: path(node.parent, false),
                data_path: path(data_parent, true),
            });
            stack.push(node.id);
        }
        // Compiler name/version are provenance, not structural identity.
        let mut canonical = self.clone();
        canonical.compiler.clear();
        let id = SchemaId(Sha256::digest(serde_json::to_vec(&canonical)?).into());
        Ok((
            Metadata {
                format_version: 1,
                id,
                compiler: self.compiler.clone(),
                modules: self.modules.clone(),
                node_count: count,
            },
            rows,
        ))
    }
}
