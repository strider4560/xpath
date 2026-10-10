use fjall_schema_spike::{Criterion, Export, NodeKind, SchemaStore, SelectionPlan, View};
use xpath1::XPath;

fn demo() -> Export {
    serde_json::from_str(include_str!("../fixtures/demo-schema.json")).unwrap()
}

#[test]
fn real_compiler_export_can_be_selected_and_walked_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let schema_id;
    {
        let store = SchemaStore::open(dir.path()).unwrap();
        schema_id = store.install(demo()).unwrap();
        assert_eq!(store.keyspace_names(), vec!["data", "schema"]);
    }
    let store = SchemaStore::open(dir.path()).unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.id(), schema_id);
    let view = snapshot.view(View::DataPaths);
    let context = view.context().unwrap();
    let selected = XPath::compile("/if:interfaces/if:interface/*[@key='true']")
        .unwrap()
        .select(&context)
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].row().node.name, "name");
    let all: Vec<_> = snapshot
        .cursor(View::Compiled)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(all.len(), demo().nodes.len());
    assert_eq!(all[0].subtree_end as usize, all.len());
    assert!(all.iter().any(|row| row.node.kind == NodeKind::Choice));
    assert!(all.iter().any(|row| row.node.kind == NodeKind::Case));
}

#[test]
fn flat_selection_plan_includes_ancestors_and_remembers_each_criterion() {
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    store.install(demo()).unwrap();
    let snapshot = store.snapshot().unwrap();
    let view = snapshot.view(View::DataPaths);
    let criteria = [
        Criterion::new("keys", "//*[@key='true']").unwrap(),
        Criterion::new("state", "//*[@kind='leaf' and @config='false']").unwrap(),
        Criterion::new("constraints", "//*[@has-when='true' or @has-must='true']").unwrap(),
    ];
    let plan = SelectionPlan::build(&view, &criteria).unwrap();
    assert!(!plan.rows.is_empty());
    assert_eq!(plan.rows[0].node.node.id, 0);
    assert_eq!(plan.rows[0].subtree_end, plan.rows.len());
    assert!(plan
        .rows
        .iter()
        .any(|r| r.matched_by.iter().any(|c| c == "keys")));
    assert!(plan
        .rows
        .iter()
        .any(|r| r.matched_by.iter().any(|c| c == "state")));
    assert!(plan
        .rows
        .iter()
        .any(|r| r.matched_by.iter().any(|c| c == "constraints")));
    for (i, row) in plan.rows.iter().enumerate().skip(1) {
        let parent = row.parent.unwrap();
        assert!(parent < i && i < plan.rows[parent].subtree_end);
        assert!(row.subtree_end > i && row.subtree_end <= plan.rows.len());
    }
    let mut cursor = plan.cursor();
    assert_eq!(cursor.next().unwrap().node.node.id, 0);
    cursor.skip_subtree();
    assert!(cursor.next().is_none());
}

#[test]
fn snapshot_spans_schema_and_data_and_old_projection_remains_walkable() {
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    let old_id = store.install(demo()).unwrap();
    let old = store.snapshot().unwrap();
    let key_id = demo().nodes.iter().find(|n| n.key).unwrap().id;
    store
        .put_value(old_id, key_id, "interface=eth0", "eth0")
        .unwrap();
    assert_eq!(
        old.values(key_id)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .len(),
        0
    );
    let with_data = store.snapshot().unwrap();
    assert_eq!(
        with_data
            .values(key_id)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()[0]
            .value,
        "eth0"
    );
    let mut changed = demo();
    changed.nodes[key_id as usize].status = "deprecated".into();
    let new_id = store.install(changed).unwrap();
    assert_ne!(old_id, new_id);
    assert_eq!(old.id(), old_id);
    assert_eq!(store.snapshot().unwrap().id(), new_id);
    assert_eq!(
        store
            .snapshot_for(old_id)
            .unwrap()
            .values(key_id)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .len(),
        1
    );
    assert!(store.snapshot().unwrap().values(key_id).next().is_none());
}

#[test]
fn invalid_table_does_not_replace_an_installed_schema() {
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    let id = store.install(demo()).unwrap();
    let mut broken = demo();
    broken.nodes[1].parent = Some(1);
    assert!(store.install(broken).is_err());
    let mut broken = demo();
    broken.format_version = 9000;
    assert!(store.install(broken).is_err());
    assert_eq!(store.snapshot().unwrap().id(), id);
}

#[test]
fn xpath_matches_independent_yang_fixture_expectations() {
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/expected-selections.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    store.install(demo()).unwrap();
    let snapshot = store.snapshot().unwrap();
    let cases = [
        (View::Compiled, "/*", "top_level"),
        (
            View::Compiled,
            "/if:interfaces/if:interface/*",
            "interface_physical_children",
        ),
        (
            View::DataPaths,
            "/if:interfaces/if:interface/*",
            "interface_logical_children_with_choice_and_case_transparent",
        ),
        (View::Compiled, "//*[@kind='leaf']", "all_leaves"),
        (View::Compiled, "//*[@kind='leaf-list']", "all_leaf_lists"),
        (View::Compiled, "//*[@kind='list']", "all_lists"),
        (
            View::Compiled,
            "//*[@kind='choice' or @kind='case']",
            "physical_choice_and_case_nodes",
        ),
        (View::DataPaths, "//*[@key='true']", "key_leaves"),
        (View::Compiled, "//*[@mandatory='true']", "mandatory_nodes"),
        (
            View::DataPaths,
            "//*[@presence='true']",
            "presence_containers",
        ),
        (
            View::DataPaths,
            "//*[@config='false']",
            "config_false_nodes",
        ),
        (View::Compiled, "//aug:*", "demo_augment_namespace_nodes"),
        (View::DataPaths, "//*[@has-when='true']", "nodes_with_when"),
        (View::DataPaths, "//*[@has-must='true']", "nodes_with_must"),
        (
            View::DataPaths,
            "//*[@status='deprecated']",
            "deprecated_nodes",
        ),
        (View::DataPaths, "//*[@status='obsolete']", "obsolete_nodes"),
        (View::DataPaths, "//*[@type='leafref']", "leafref_nodes"),
    ];
    for (mode, expression, oracle) in cases {
        let view = snapshot.view(mode);
        let selected = XPath::compile(expression)
            .unwrap()
            .select(&view.context().unwrap())
            .unwrap();
        let ids: Vec<_> = selected
            .iter()
            .map(|n| u64::from(n.row().node.id))
            .collect();
        let want: Vec<_> = expected[oracle]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .collect();
        assert_eq!(ids, want, "{mode:?} {expression}");
    }
}

#[test]
fn transparent_parents_reverse_axes_and_union_preserve_xpath_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    store.install(demo()).unwrap();
    let snapshot = store.snapshot().unwrap();
    let logical = snapshot.view(View::DataPaths);
    let physical = snapshot.view(View::Compiled);
    let ids = |view: &fjall_schema_spike::SchemaView, expr: &str| {
        XPath::compile(expr)
            .unwrap()
            .select(&view.context().unwrap())
            .unwrap()
            .iter()
            .map(|n| n.row().node.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&logical, "/if:interfaces/if:interface/if:static-address/.."),
        [3]
    );
    assert_eq!(ids(&physical, "//if:static-address/.."), [11]);
    assert_eq!(ids(&logical, "//if:dhcp/preceding-sibling::*[1]"), [13]);
    assert!(ids(&physical, "//if:dhcp/preceding-sibling::*[1]").is_empty());
    assert_eq!(
        ids(&logical, "//if:mtu | //*[@key='true'] | //if:mtu"),
        [4, 7]
    );
    assert_eq!(ids(&physical, "//if:static-address/ancestor::*[1]"), [11]);
    assert_eq!(ids(&logical, "//if:static-address/ancestor::*[1]"), [3]);
    assert!(logical.node(10).is_err());
    assert!(XPath::compile(".")
        .unwrap()
        .select(&xpath1::Context::new(&logical, physical.node(3).unwrap()))
        .is_err());
}

#[test]
fn raw_cursor_can_seek_and_skip_subtrees_without_losing_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    store.install(demo()).unwrap();
    let snapshot = store.snapshot().unwrap();
    let mut cursor = snapshot.cursor(View::Compiled).unwrap();
    cursor.seek(3).unwrap();
    assert_eq!(cursor.next().unwrap().unwrap().node.id, 3);
    cursor.skip_subtree();
    assert_eq!(cursor.next().unwrap().unwrap().node.id, 26);
    cursor.skip_subtree();
    assert!(cursor.next().is_none());
    let mut cursor = snapshot.subtree(View::DataPaths, 3).unwrap();
    assert!(cursor.seek(2).is_err());
    assert!(cursor.seek(27).is_err());
    cursor.seek(10).unwrap(); // Hidden choice/case rows are skipped.
    assert_eq!(cursor.next().unwrap().unwrap().node.id, 12);
    cursor.seek(26).unwrap(); // Exclusive end is a valid empty range.
    assert!(cursor.next().is_none());
}

#[test]
fn plan_retains_multiple_matches_and_handles_empty_and_invalid_criteria() {
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    store.install(demo()).unwrap();
    let view = store.snapshot().unwrap().view(View::DataPaths);
    let plan = SelectionPlan::build(
        &view,
        &[
            Criterion::new("keys", "//*[@key='true']").unwrap(),
            Criterion::new("mandatory", "//*[@mandatory='true']").unwrap(),
        ],
    )
    .unwrap();
    assert_eq!(
        plan.rows
            .iter()
            .find(|r| r.node.node.id == 4)
            .unwrap()
            .matched_by,
        ["keys", "mandatory"]
    );
    let json = serde_json::to_string(&plan).unwrap();
    let decoded: SelectionPlan = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.schema_id, plan.schema_id);
    assert_eq!(decoded.cursor().count(), plan.rows.len());
    let empty =
        SelectionPlan::build(&view, &[Criterion::new("absent", "//if:absent").unwrap()]).unwrap();
    assert!(empty.cursor().next().is_none());
    assert!(
        SelectionPlan::build(&view, &[Criterion::new("attribute", "//@key").unwrap()]).is_err()
    );
    assert!(SelectionPlan::build(
        &view,
        &[
            Criterion::new("same", "/*").unwrap(),
            Criterion::new("same", "/*").unwrap(),
        ]
    )
    .is_err());
}

#[test]
fn module_identity_features_and_compiler_provenance_have_explicit_boundaries() {
    use xpath1::Navigator;
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    let original = store.install(demo()).unwrap();
    let mut equivalent = demo();
    equivalent.compiler = "future-rust-compiler 0.1".into();
    assert_eq!(store.install(equivalent).unwrap(), original);
    let old = store.snapshot().unwrap().view(View::DataPaths);
    let tag = old.node(22).unwrap();
    assert_eq!(
        old.name(&tag).unwrap().unwrap().namespace_uri,
        "urn:demo:augment"
    );
    assert_eq!(old.string_value(&tag).unwrap(), "");
    assert_eq!(
        XPath::compile("name(//aug:tag)")
            .unwrap()
            .evaluate(&old.context().unwrap())
            .unwrap()
            .to_string(&old)
            .unwrap(),
        "demo-augment:tag"
    );
    let xml_ns = XPath::compile("//aug:tag/namespace::xml")
        .unwrap()
        .select(&old.context().unwrap())
        .unwrap();
    assert_eq!(xml_ns.len(), 1);
    assert_eq!(old.parent(&xml_ns[0]).unwrap(), Some(tag));
    let enabled: Export =
        serde_json::from_str(include_str!("../fixtures/demo-schema-feature.json")).unwrap();
    let new_id = store.install(enabled).unwrap();
    assert_ne!(original, new_id);
    let new = store.snapshot().unwrap().view(View::DataPaths);
    assert_eq!(
        XPath::compile("//if:diagnostic-mode")
            .unwrap()
            .select(&new.context().unwrap())
            .unwrap()
            .len(),
        1
    );
    assert!(XPath::compile(".")
        .unwrap()
        .select(&xpath1::Context::new(&new, old.node(3).unwrap()))
        .is_err());
    assert_eq!(old.snapshot().node_count(), 29);
}

#[test]
fn rejects_noncontiguous_trees_and_inconsistent_module_namespaces() {
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    let original = store.install(demo()).unwrap();
    let mut bad = demo();
    bad.nodes[6].parent = Some(2); // Closes list 3; row 7 tries to return to it.
    assert!(store.install(bad).is_err());
    let mut bad = demo();
    bad.nodes[4].namespace = "urn:incorrect".into();
    assert!(store.install(bad).is_err());
    for type_name in ["", "future-unknown-base"] {
        let mut bad = demo();
        bad.nodes[4].type_name = Some(type_name.into());
        assert!(store.install(bad).is_err());
    }
    let mut bad = demo();
    bad.nodes[0].key = true;
    assert!(store.install(bad).is_err());
    assert!(store
        .put_value(original, 3, "instance", "not-a-leaf")
        .is_err());
    assert_eq!(store.snapshot().unwrap().id(), original);
}

#[test]
fn module_named_xml_gets_a_nonreserved_xpath_alias() {
    use xpath1::Navigator;
    let mut export = demo();
    export.modules.truncate(1);
    export.modules[0].name = "xml".into();
    export.modules[0].namespace = "urn:xml".into();
    export.modules[0].prefix = "z".into();
    export.nodes.truncate(2);
    export.nodes[1].module = "xml".into();
    export.nodes[1].namespace = "urn:xml".into();
    export.nodes[1].name = "v".into();
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    store.install(export).unwrap();
    let view = store.snapshot().unwrap().view(View::DataPaths);
    let selected = XPath::compile("/z:v")
        .unwrap()
        .select(&view.context().unwrap())
        .unwrap();
    let qname = view.qualified_name(&selected[0]).unwrap();
    assert_ne!(qname, "xml:v");
    assert_eq!(
        XPath::compile(&format!("/{qname}"))
            .unwrap()
            .select(&view.context().unwrap())
            .unwrap(),
        selected
    );
    let namespace = XPath::compile("/z:v/namespace::xml")
        .unwrap()
        .select(&view.context().unwrap())
        .unwrap();
    assert_eq!(
        view.string_value(&namespace[0]).unwrap(),
        "http://www.w3.org/XML/1998/namespace"
    );
}

#[test]
fn oversized_instance_key_returns_error_instead_of_panicking_in_fjall() {
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    let id = store.install(demo()).unwrap();
    assert!(store.put_value(id, 4, &"x".repeat(65_499), "eth0").is_err());
    assert!(store.put_value(id, 4, &"é".repeat(32_750), "eth0").is_err());
    store.put_value(id, 4, &"x".repeat(65_498), "eth0").unwrap();
    assert_eq!(
        store
            .snapshot()
            .unwrap()
            .values(4)
            .next()
            .unwrap()
            .unwrap()
            .value,
        "eth0"
    );
}

#[test]
fn nullable_export_fields_are_required_and_unknown_fields_are_rejected() {
    let original = serde_json::to_value(demo()).unwrap();
    for field in ["parent", "config", "type_name"] {
        let mut value = original.clone();
        value["nodes"][0].as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<Export>(value).is_err(),
            "missing {field}"
        );
    }
    let mut value = original.clone();
    value["modules"][0]
        .as_object_mut()
        .unwrap()
        .remove("revision");
    assert!(serde_json::from_value::<Export>(value).is_err());
    let mut value = original;
    value["unknown"] = true.into();
    assert!(serde_json::from_value::<Export>(value).is_err());
}
