use fjall_schema_spike::{Criterion, Export, SchemaStore, SelectionPlan, View};
use std::{error::Error, path::Path};
use xpath1::XPath;

const DEMO: &str = include_str!("../fixtures/demo-schema.json");
type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    let mode = if let Some(index) = args.iter().position(|s| s == "--compiled") {
        args.remove(index);
        View::Compiled
    } else {
        View::DataPaths
    };
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["--help"] | ["help"] => {
            println!(
                "fjall-schema-spike demo DB\n\
                      fjall-schema-spike import DB EXPORT.json\n\
                      fjall-schema-spike query DB XPATH [--compiled]\n\
                      fjall-schema-spike walk DB [--compiled]\n\
                      fjall-schema-spike plan DB XPATH... [--compiled]\n\
                      fjall-schema-spike values DB NODE_ID"
            );
        }
        ["demo", path] => demo(Path::new(path))?,
        ["import", path, file] => {
            let export: Export = serde_json::from_slice(&std::fs::read(file)?)?;
            println!("{}", SchemaStore::open(path)?.install(export)?);
        }
        ["query", path, source] => {
            let store = SchemaStore::open(path)?;
            let view = store.snapshot()?.view(mode);
            let expression = XPath::compile(source)?;
            for node in expression.select(&view.context()?)? {
                use xpath1::Navigator;
                if node.is_schema_node() {
                    let row = node.row();
                    println!(
                        "{}\t{}\t{}",
                        row.node.id,
                        row.node.kind.as_str(),
                        if mode == View::Compiled {
                            &row.compiled_path
                        } else {
                            &row.data_path
                        }
                    );
                } else {
                    println!(
                        "{}\t{}\t{}",
                        node.row().node.id,
                        view.qualified_name(&node)?,
                        view.string_value(&node)?
                    );
                }
            }
        }
        ["walk", path] => {
            let store = SchemaStore::open(path)?;
            for row in store.snapshot()?.cursor(mode)? {
                let row = row?;
                println!(
                    "{}\tparent={:?}\tend={}\t{}\t{}",
                    row.node.id,
                    row.parent(mode),
                    row.subtree_end,
                    row.node.kind.as_str(),
                    if mode == View::Compiled {
                        &row.compiled_path
                    } else {
                        &row.data_path
                    }
                );
            }
        }
        ["plan", path, sources @ ..] if !sources.is_empty() => {
            let store = SchemaStore::open(path)?;
            let view = store.snapshot()?.view(mode);
            let criteria: Vec<_> = sources
                .iter()
                .enumerate()
                .map(|(i, source)| Criterion::new(format!("selector-{}", i + 1), source))
                .collect::<std::result::Result<_, _>>()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&SelectionPlan::build(&view, &criteria)?)?
            );
        }
        ["values", path, node] => {
            let store = SchemaStore::open(path)?;
            for value in store.snapshot()?.values(node.parse()?) {
                println!("{}", serde_json::to_string(&value?)?);
            }
        }
        _ => return Err("invalid arguments; run with --help".into()),
    }
    Ok(())
}

fn demo(path: &Path) -> Result<()> {
    let store = SchemaStore::open(path)?;
    let id = store.install(serde_json::from_str(DEMO)?)?;
    println!("Projection: {id}");
    let snapshot = store.snapshot()?;
    println!(
        "Compiler: {}; {} compiled rows",
        snapshot.compiler(),
        snapshot.node_count()
    );
    println!("Keyspaces: {}", store.keyspace_names().join(", "));
    let view = snapshot.view(View::DataPaths);
    let criteria = [
        Criterion::new("keys", "//*[@key='true']")?,
        Criterion::new(
            "config-leaves",
            "/if:interfaces/if:interface/*[@kind='leaf' and @config='true']",
        )?,
        Criterion::new("state-leaves", "//*[@kind='leaf' and @config='false']")?,
        Criterion::new("constraints", "//*[@has-when='true' or @has-must='true']")?,
        Criterion::new("augment", "//aug:*")?,
        Criterion::new("leafrefs", "//*[@type='leafref']")?,
    ];
    let plan = SelectionPlan::build(&view, &criteria)?;
    println!(
        "\nFlat selection: {} rows including ancestors",
        plan.rows.len()
    );
    println!("index  schema-id  parent  end  kind         matches  path");
    for (index, row) in plan.cursor().enumerate() {
        println!(
            "{index:>5}  {:>9}  {:>6}  {:>3}  {:<12} {:<32} {}",
            row.node.node.id,
            row.parent.map_or_else(|| "-".into(), |p| p.to_string()),
            row.subtree_end,
            row.node.node.kind.as_str(),
            row.matched_by.join(","),
            row.node.data_path
        );
    }
    // Resolve the schema IDs with XPath; data storage never hardcodes them.
    let key = XPath::compile("//*[@key='true']")?
        .select(&view.context()?)?
        .remove(0);
    let status = XPath::compile("//if:oper-status")?
        .select(&view.context()?)?
        .remove(0);
    store.put_value(id, key.row().node.id, "interface=eth0", "eth0")?;
    store.put_value(id, status.row().node.id, "interface=eth0", "up")?;

    let with_data = store.snapshot()?;
    println!("\nData rows reached through selected schema IDs (new snapshot):");
    for row in plan
        .cursor()
        .filter(|r| !r.matched_by.is_empty() && r.node.node.kind.scalar())
    {
        for value in with_data.values(row.node.node.id) {
            let value = value?;
            println!(
                "{} [{}] = {}",
                row.node.data_path, value.instance, value.value
            );
        }
    }
    let mut cursor = with_data.cursor(View::DataPaths)?;
    let interface = XPath::compile("/if:interfaces/if:interface")?
        .select(&view.context()?)?
        .remove(0);
    cursor.seek(interface.row().node.id)?;
    let row = cursor.next().ok_or("missing interface row")??;
    cursor.skip_subtree();
    let next = cursor.next().transpose()?.map(|r| r.data_path);
    println!(
        "\nCursor: skipped schema IDs {}..{}; next = {next:?}",
        row.node.id, row.subtree_end
    );
    Ok(())
}
