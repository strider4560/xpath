use crate::{Error, FlatNode, Result, SchemaId, SchemaView, View};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use xpath1::XPath;

/// A reusable compiled selector over schema declarations and their metadata.
pub struct Criterion {
    label: String,
    expression: XPath,
}
impl Criterion {
    pub fn new(label: impl Into<String>, source: &str) -> Result<Self> {
        let label = label.into();
        if label.is_empty() {
            return Err(Error::Invalid("criterion label is empty".into()));
        }
        Ok(Self {
            label,
            expression: XPath::compile(source)?,
        })
    }
}

/// Indices refer to this selection's dense rows, while `node.node.id` continues
/// to refer to the immutable schema projection. Unmatched ancestors are retained.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanRow {
    pub node: FlatNode,
    pub parent: Option<usize>,
    pub subtree_end: usize,
    pub depth: u32,
    pub matched_by: Vec<String>,
}

/// An ancestry-preserving flat selection tree, not an optimizer or the XPath
/// parser's expression AST. Empty selections produce an empty tree.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionPlan {
    pub schema_id: SchemaId,
    pub view: View,
    pub criteria: BTreeMap<String, String>,
    pub rows: Vec<PlanRow>,
}
impl SelectionPlan {
    pub fn build(view: &SchemaView, criteria: &[Criterion]) -> Result<Self> {
        let context = view.context()?;
        let mut sources = BTreeMap::new();
        let mut selected: BTreeMap<u32, (FlatNode, BTreeSet<String>)> = BTreeMap::new();
        for criterion in criteria {
            if sources
                .insert(
                    criterion.label.clone(),
                    criterion.expression.source().into(),
                )
                .is_some()
            {
                return Err(Error::Invalid("criterion labels must be unique".into()));
            }
            for handle in criterion.expression.select(&context)? {
                if !handle.is_schema_node() {
                    return Err(Error::Invalid(
                        "plan criteria must select schema nodes, not attributes or namespaces"
                            .into(),
                    ));
                }
                let row = handle.row();
                selected
                    .entry(row.node.id)
                    .or_insert_with(|| (row.clone(), BTreeSet::new()))
                    .1
                    .insert(criterion.label.clone());
                let mut parent = row.parent(view.mode());
                while let Some(id) = parent {
                    if selected.contains_key(&id) {
                        break;
                    }
                    let row = view.snapshot().row(id)?;
                    parent = row.parent(view.mode());
                    selected.insert(id, (row, BTreeSet::new()));
                }
            }
        }
        let mut indices = BTreeMap::new();
        let mut rows: Vec<PlanRow> = Vec::with_capacity(selected.len());
        for (id, (node, matched_by)) in selected {
            let parent = node.parent(view.mode()).map(|id| indices[&id]);
            let depth = parent.map_or(0, |index: usize| rows[index].depth + 1);
            let index = rows.len();
            indices.insert(id, index);
            rows.push(PlanRow {
                node,
                parent,
                subtree_end: index + 1,
                depth,
                matched_by: matched_by.into_iter().collect(),
            });
        }
        for index in (0..rows.len()).rev() {
            if let Some(parent) = rows[index].parent {
                rows[parent].subtree_end = rows[parent].subtree_end.max(rows[index].subtree_end);
            }
        }
        Ok(Self {
            schema_id: view.snapshot().id(),
            view: view.mode(),
            criteria: sources,
            rows,
        })
    }

    pub fn cursor(&self) -> PlanCursor<'_> {
        PlanCursor {
            rows: &self.rows,
            next: 0,
            last_end: None,
        }
    }
}

pub struct PlanCursor<'a> {
    rows: &'a [PlanRow],
    next: usize,
    last_end: Option<usize>,
}
impl PlanCursor<'_> {
    pub fn skip_subtree(&mut self) {
        if let Some(end) = self.last_end.take() {
            self.next = end;
        }
    }
}
impl<'a> Iterator for PlanCursor<'a> {
    type Item = &'a PlanRow;
    fn next(&mut self) -> Option<Self::Item> {
        let row = self.rows.get(self.next)?;
        self.next += 1;
        self.last_end = Some(row.subtree_end);
        Some(row)
    }
}
