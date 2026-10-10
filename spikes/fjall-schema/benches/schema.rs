use criterion::{black_box, criterion_group, criterion_main, Criterion as Bench};
use fjall_schema_spike::{Criterion, SchemaStore, SelectionPlan, View};
use xpath1::XPath;

fn schema(c: &mut Bench) {
    let dir = tempfile::tempdir().unwrap();
    let store = SchemaStore::open(dir.path()).unwrap();
    store
        .install(serde_json::from_str(include_str!("../fixtures/demo-schema.json")).unwrap())
        .unwrap();
    let snapshot = store.snapshot().unwrap();
    let view = snapshot.view(View::DataPaths);
    let context = view.context().unwrap();
    let path = XPath::compile("/if:interfaces/if:interface/if:static-address").unwrap();
    let metadata = XPath::compile("//*[@config='false']").unwrap();
    let criteria = [
        Criterion::new("keys", "//*[@key='true']").unwrap(),
        Criterion::new("state", "//*[@kind='leaf' and @config='false']").unwrap(),
        Criterion::new("constraints", "//*[@has-when='true' or @has-must='true']").unwrap(),
    ];
    let plan = SelectionPlan::build(&view, &criteria).unwrap();
    assert_eq!(path.select(&context).unwrap().len(), 1);
    assert_eq!(metadata.select(&context).unwrap().len(), 6);
    assert_eq!(snapshot.cursor(View::Compiled).unwrap().count(), 29);

    let mut group = c.benchmark_group("fjall_schema_29_rows");
    group.sample_size(30);
    group.warm_up_time(std::time::Duration::from_millis(200));
    group.measurement_time(std::time::Duration::from_secs(1));
    group.bench_function("point_row", |b| {
        b.iter(|| black_box(snapshot.row(black_box(12)).unwrap()))
    });
    group.bench_function("preorder_decode_all", |b| {
        b.iter(|| {
            for row in snapshot.cursor(View::Compiled).unwrap() {
                black_box(row.unwrap());
            }
        })
    });
    group.bench_function("skip_interface_subtree", |b| {
        b.iter(|| {
            let mut cursor = snapshot.subtree(View::DataPaths, 3).unwrap();
            black_box(cursor.next().unwrap().unwrap());
            cursor.skip_subtree();
            assert!(cursor.next().is_none());
        })
    });
    group.bench_function("xpath_path", |b| {
        b.iter(|| black_box(path.select(&context).unwrap()))
    });
    group.bench_function("xpath_metadata", |b| {
        b.iter(|| black_box(metadata.select(&context).unwrap()))
    });
    group.bench_function("build_three_criterion_plan", |b| {
        b.iter(|| black_box(SelectionPlan::build(&view, &criteria).unwrap()))
    });
    group.bench_function("walk_built_plan", |b| {
        b.iter(|| {
            for row in plan.cursor() {
                black_box(row);
            }
        })
    });
    group.finish();
}
criterion_group!(benches, schema);
criterion_main!(benches);
