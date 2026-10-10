use crate::model::Metadata;
use crate::{Error, Export, FlatNode, Module, Result, SchemaId, SchemaView, View};
use fjall::{Database, Keyspace, KeyspaceCreateOptions, PersistMode, Readable, Snapshot};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};

/// Owns exactly two application keyspaces. Installed projections are immutable.
pub struct SchemaStore {
    db: Database,
    schema: Keyspace,
    data: Keyspace,
}

fn prefix(tag: u8, id: SchemaId) -> Vec<u8> {
    let mut key = Vec::with_capacity(42);
    key.push(tag);
    key.extend_from_slice(&id.0);
    key
}
fn node_key(tag: u8, id: SchemaId, node: u32) -> Vec<u8> {
    let mut key = prefix(tag, id);
    key.extend_from_slice(&node.to_be_bytes());
    key
}
fn child_prefix(id: SchemaId, view: View, parent: u32) -> Vec<u8> {
    let mut key = prefix(b'c', id);
    key.push(view.code());
    key.extend_from_slice(&parent.to_be_bytes());
    key
}

impl SchemaStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = Database::builder(path).open()?;
        let schema = db.keyspace("schema", KeyspaceCreateOptions::default)?;
        let data = db.keyspace("data", KeyspaceCreateOptions::default)?;
        Ok(Self { db, schema, data })
    }

    pub fn keyspace_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .db
            .list_keyspace_names()
            .iter()
            .map(ToString::to_string)
            .collect();
        names.sort();
        names
    }

    /// Validate completely before publishing a generation and its active pointer
    /// together in one durable atomic batch. Older generations remain readable.
    pub fn install(&self, export: Export) -> Result<SchemaId> {
        let (metadata, rows) = export.prepare()?;
        let id = metadata.id;
        let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));
        if self.schema.get(prefix(b'm', id))?.is_none() {
            for row in &rows {
                batch.insert(
                    &self.schema,
                    node_key(b'r', id, row.node.id),
                    serde_json::to_vec(row)?,
                );
                for view in [View::Compiled, View::DataPaths] {
                    if !view.visible(row.node.kind) {
                        continue;
                    }
                    if let Some(parent) = row.parent(view) {
                        let mut key = child_prefix(id, view, parent);
                        key.extend_from_slice(&row.node.id.to_be_bytes());
                        batch.insert(&self.schema, key, []);
                    }
                }
            }
            batch.insert(
                &self.schema,
                prefix(b'm', id),
                serde_json::to_vec(&metadata)?,
            );
        }
        batch.insert(&self.schema, b"active", id.0);
        batch.commit()?;
        Ok(id)
    }

    pub fn snapshot(&self) -> Result<SchemaSnapshot> {
        let snapshot = self.db.snapshot();
        let active = snapshot
            .get(&self.schema, b"active")?
            .ok_or_else(|| Error::Missing("no installed schema".into()))?;
        let bytes: [u8; 32] = active
            .as_ref()
            .try_into()
            .map_err(|_| Error::Invalid("invalid stored active projection ID".into()))?;
        self.snapshot_at(snapshot, SchemaId(bytes))
    }

    pub fn snapshot_for(&self, id: SchemaId) -> Result<SchemaSnapshot> {
        self.snapshot_at(self.db.snapshot(), id)
    }

    fn snapshot_at(&self, snapshot: Snapshot, id: SchemaId) -> Result<SchemaSnapshot> {
        let bytes = snapshot
            .get(&self.schema, prefix(b'm', id))?
            .ok_or_else(|| Error::Missing(format!("schema projection {id}")))?;
        let metadata: Metadata = serde_json::from_slice(&bytes)?;
        if metadata.id != id || metadata.format_version != 1 || metadata.node_count == 0 {
            return Err(Error::Invalid("invalid stored projection metadata".into()));
        }
        Ok(SchemaSnapshot {
            snapshot,
            schema: self.schema.clone(),
            data: self.data.clone(),
            metadata: Arc::new(metadata),
        })
    }

    /// Store a lexical leaf value. `instance` is an opaque caller-owned instance
    /// identifier, not an XPath or a YANG instance-identifier. No validation is run.
    pub fn put_value(&self, id: SchemaId, node: u32, instance: &str, value: &str) -> Result<()> {
        let snapshot = self.snapshot_for(id)?;
        // Fjall's key length limit is u16::MAX; our prefix occupies 37 bytes.
        if !snapshot.row(node)?.node.kind.scalar() || instance.is_empty() || instance.len() > 65_498
        {
            return Err(Error::Invalid(
                "a value requires a leaf/leaf-list and an instance key of 1..=65498 UTF-8 bytes"
                    .into(),
            ));
        }
        let mut key = node_key(b'v', id, node);
        key.extend_from_slice(instance.as_bytes());
        let value = DataValue {
            schema_node: node,
            instance: instance.into(),
            value: value.into(),
        };
        let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));
        batch.insert(&self.data, key, serde_json::to_vec(&value)?);
        batch.commit()?;
        Ok(())
    }
}

/// One Fjall sequence number covers both keyspaces and the active generation.
#[derive(Clone)]
pub struct SchemaSnapshot {
    snapshot: Snapshot,
    schema: Keyspace,
    data: Keyspace,
    metadata: Arc<Metadata>,
}
impl SchemaSnapshot {
    pub fn id(&self) -> SchemaId {
        self.metadata.id
    }
    pub fn node_count(&self) -> u32 {
        self.metadata.node_count
    }
    pub fn modules(&self) -> &[Module] {
        &self.metadata.modules
    }
    pub fn compiler(&self) -> &str {
        &self.metadata.compiler
    }
    pub fn view(&self, mode: View) -> SchemaView {
        SchemaView::new(self.clone(), mode)
    }

    pub fn row(&self, node: u32) -> Result<FlatNode> {
        if node >= self.node_count() {
            return Err(Error::Missing(format!("schema node {node}")));
        }
        let bytes = self
            .snapshot
            .get(&self.schema, node_key(b'r', self.id(), node))?
            .ok_or_else(|| Error::Missing(format!("schema row {node}")))?;
        self.decode_row(node, &bytes)
    }

    fn decode_row(&self, id: u32, bytes: &[u8]) -> Result<FlatNode> {
        let row: FlatNode = serde_json::from_slice(bytes)?;
        if row.node.id != id
            || row.subtree_end <= id
            || row.subtree_end > self.node_count()
            || row.node.parent.is_some_and(|p| p >= id)
            || row.data_parent.is_some_and(|p| p >= id)
        {
            return Err(Error::Invalid("corrupt stored schema row".into()));
        }
        Ok(row)
    }

    pub fn cursor(&self, view: View) -> Result<FlatCursor> {
        self.subtree(view, 0)
    }

    pub fn subtree(&self, view: View, node: u32) -> Result<FlatCursor> {
        let row = self.row(node)?;
        if !view.visible(row.node.kind) {
            return Err(Error::Invalid("subtree root is hidden in this view".into()));
        }
        Ok(FlatCursor {
            snapshot: self.clone(),
            view,
            start: node,
            end: row.subtree_end,
            iter: self.row_range(node, row.subtree_end),
            last_end: None,
        })
    }

    fn row_range(&self, start: u32, end: u32) -> fjall::Iter {
        self.snapshot.range(
            &self.schema,
            node_key(b'r', self.id(), start)..node_key(b'r', self.id(), end),
        )
    }

    pub(crate) fn children(
        &self,
        view: View,
        parent: u32,
    ) -> impl Iterator<Item = Result<FlatNode>> + '_ {
        let prefix = child_prefix(self.id(), view, parent);
        self.snapshot
            .prefix(&self.schema, prefix)
            .map(move |guard| {
                let (key, _) = guard.into_inner()?;
                let bytes: [u8; 4] = key
                    .get(38..)
                    .and_then(|b| b.try_into().ok())
                    .ok_or_else(|| Error::Invalid("corrupt child index".into()))?;
                let row = self.row(u32::from_be_bytes(bytes))?;
                if row.parent(view) != Some(parent) || !view.visible(row.node.kind) {
                    return Err(Error::Invalid("child index does not match row".into()));
                }
                Ok(row)
            })
    }

    /// Values ordered by the UTF-8 bytes of the caller's instance key.
    pub fn values(&self, node: u32) -> impl Iterator<Item = Result<DataValue>> + '_ {
        self.snapshot
            .prefix(&self.data, node_key(b'v', self.id(), node))
            .map(move |guard| {
                let (key, value) = guard.into_inner()?;
                let value: DataValue = serde_json::from_slice(&value)?;
                if value.schema_node != node || key.get(37..) != Some(value.instance.as_bytes()) {
                    return Err(Error::Invalid(
                        "data key does not match stored value".into(),
                    ));
                }
                Ok(value)
            })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataValue {
    pub schema_node: u32,
    pub instance: String,
    pub value: String,
}

/// A storage cursor: only the current row is decoded. Seeking past a subtree
/// creates a new bounded LSM range; descendants are not decoded or visited.
pub struct FlatCursor {
    snapshot: SchemaSnapshot,
    view: View,
    start: u32,
    end: u32,
    iter: fjall::Iter,
    last_end: Option<u32>,
}
impl FlatCursor {
    /// Seek within this cursor's original subtree, including its exclusive end.
    pub fn seek(&mut self, node: u32) -> Result<()> {
        if node < self.start || node > self.end {
            return Err(Error::Invalid("seek is outside cursor bounds".into()));
        }
        self.iter = self.snapshot.row_range(node, self.end);
        self.last_end = None;
        Ok(())
    }
    pub fn skip_subtree(&mut self) {
        if let Some(end) = self.last_end.take() {
            self.iter = self.snapshot.row_range(end, self.end);
        }
    }
}
impl Iterator for FlatCursor {
    type Item = Result<FlatNode>;
    fn next(&mut self) -> Option<Self::Item> {
        self.last_end = None;
        for guard in self.iter.by_ref() {
            let decoded = (|| {
                let (key, bytes) = guard.into_inner()?;
                let id: [u8; 4] = key
                    .get(33..)
                    .and_then(|b| b.try_into().ok())
                    .ok_or_else(|| Error::Invalid("corrupt row key".into()))?;
                self.snapshot.decode_row(u32::from_be_bytes(id), &bytes)
            })();
            match decoded {
                Ok(row) if !self.view.visible(row.node.kind) => continue,
                Ok(row) => {
                    self.last_end = Some(row.subtree_end);
                    return Some(Ok(row));
                }
                Err(e) => return Some(Err(e)),
            }
        }
        None
    }
}
