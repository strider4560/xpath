# Fjall flat-schema spike

This executable spike imports a **real libyang-compiled schema** into Fjall,
uses the existing `xpath1` crate to select schema declarations, and constructs
an ancestry-preserving flat selection tree. Both the stored schema and the
selection tree can be walked with a cursor and exclusive subtree bounds.

The compiler boundary is a versioned JSON table. The Rust code has no libyang
dependency: a future compiler can produce the same table. This is a canonical
**planning projection**, not a complete YANG AST or schema validator.

## Run

The spike is a separate, unpublished package. Fjall 3.1.10 requires Rust 1.90;
the nested `rust-toolchain.toml` selects 1.90.0. The parent XPath crate still
requires only Rust 1.85 and has no Fjall dependency.

```sh
cd spikes/fjall-schema
cargo run --locked -- demo ./demo-db
cargo run --locked -- query ./demo-db \
  "/if:interfaces/if:interface/*[@key='true']"
cargo run --locked -- walk ./demo-db --compiled
cargo run --locked -- plan ./demo-db \
  "//*[@key='true']" "//*[@config='false']" > selection-plan.json
cargo run --locked -- values ./demo-db 4
```

The demo installs the checked-in 29-row export, constructs a 22-row selection
from six criteria, writes two illustrative leaf values to `data`, then reaches
those values through the selected schema IDs. It also seeks to schema row 3,
skips its subtree `[3, 26)`, and lands on `/demo-if:inventory`. Running it again
reuses the projection and replaces the same two illustrative values; use a
dedicated demo directory.

To import your own compiled projection:

```sh
cargo run --locked -- import ./your-db /path/to/compiled-schema.json
```

See [exporter/README.md](exporter/README.md) for building the pinned C exporter
and [FORMAT.md](FORMAT.md) for the compiler-independent contract. The fixtures
include grouping expansion, keys, choice/case, config/state, constraints,
leafrefs, features, and an augment from a second module. Enabling the fixture's
`diagnostics` feature produces a separate 30-row projection.

## Rust interface

```rust
use fjall_schema_spike::{Criterion, SchemaStore, SelectionPlan, View};
use xpath1::XPath;

let store = SchemaStore::open("./demo-db")?;
let snapshot = store.snapshot()?;
let schema = snapshot.view(View::DataPaths);

// A compiled selector can be reused across schema snapshots.
let keys = XPath::compile("/if:interfaces/if:interface/*[@key='true']")?;
for node in keys.select(&schema.context()?)? {
    println!("{}", node.row().data_path);
}

let plan = SelectionPlan::build(&schema, &[
    Criterion::new("keys", "//*[@key='true']")?,
    Criterion::new("state", "//*[@kind='leaf' and @config='false']")?,
    Criterion::new("conditions", "//*[@has-when='true']")?,
])?;

let mut cursor = plan.cursor();
while let Some(row) = cursor.next() {
    // Empty matched_by means this row was retained as an ancestor.
    if !row.matched_by.is_empty() {
        println!("{}: {:?}", row.node.data_path, row.matched_by);
        for value in snapshot.values(row.node.node.id) {
            println!("{:?}", value?);
        }
    }
    // A caller can decide to prune: cursor.skip_subtree();
}
# Ok::<(), fjall_schema_spike::Error>(())
```

`SchemaStore::install(Export)` validates and atomically publishes a projection.
`SchemaStore::snapshot()` captures the active projection and both keyspaces at
one Fjall sequence number. Use `snapshot_for(id)` to open a retained generation.

To walk the underlying table directly:

```rust
let mut cursor = snapshot.subtree(View::DataPaths, 3)?;
let interface = cursor.next().transpose()?.unwrap();
assert_eq!(interface.subtree_end, 26);
cursor.skip_subtree(); // Range-seeks to 26; does not decode descendants.
assert!(cursor.next().is_none()); // This cursor was bounded to that subtree.
```

## The two trees

| View | What XPath sees | Fixture path to `static-address` |
|---|---|---|
| `Compiled` | Physical compiled declarations, including choice/case | `/if:interfaces/if:interface/if:addressing/if:static/if:static-address` |
| `DataPaths` | Schema declarations with choice/case transparent | `/if:interfaces/if:interface/if:static-address` |

`DataPaths` is still a schema view. A list declaration appears once; it does
not have a row for every list instance. Conditions on runtime values, such as
`if:interface[if:name='eth0']`, belong in a data-tree navigator. Schema elements
have empty string values and no text nodes. Schema metadata is exposed through
unqualified virtual attributes:

| Metadata attributes | Meaning |
|---|---|
| `@kind`, `@name`, `@module`, `@namespace`, `@id` | Compiled declaration identity |
| `@config`, `@key`, `@mandatory`, `@presence`, `@status`, `@type` | Effective flags, status, builtin base type |
| `@has-when`, `@has-must`, `@when-count`, `@must-count` | Discover declarations carrying constraints |
| `@path`, `@compiled-path` | Module-name paths in the chosen view and physical tree |

Boolean attributes contain the strings `true` and `false`. Use
`[@key='true']`; `[@key]` merely tests that the attribute exists. `@config` and
`@type` are absent when inapplicable. The virtual root has no attributes.
Constraint expressions remain available in `node.row().node.when` and `.must`.
They lack the original evaluation contexts and prefix maps, so the spike does
not execute them as YANG `when`/`must` constraints. The parent crate's
`examples/yang.rs` demonstrates Boolean evaluation over an actual data context.

Contexts bind module names as XPath prefixes, plus declared prefixes when
unambiguous and not shadowing a module-name binding. The reserved `xml` binding
always retains its XML namespace; a module named `xml` or `xmlns` receives a
collision-free `_schemaN` alias. `namespace_bindings()` exposes these bindings;
callers can add expression-specific aliases with `Context::with_namespace`.
`name()` uses the selected canonical alias. Stored paths always use actual
module names and are identifiers, not guaranteed standalone XPath expressions.
Unprefixed element names follow XPath 1.0 and match the empty namespace unless
the caller explicitly sets a default element namespace on the context.

## Flat selection rows

Each `PlanRow` contains the original schema row, a dense plan `parent` index,
an exclusive plan `subtree_end`, a depth, and all matching criterion labels.
Rows are in schema preorder. Root and other necessary ancestors are retained;
their `matched_by` is empty unless a criterion selected them directly.

Selecting a container does not automatically select its descendants. Add
`/descendant-or-self::*` to a selector when that is wanted, or open the selected
schema node with `snapshot.subtree(...)`. Empty results produce an empty plan.
Labels must be unique. Criteria must return schema nodes, not attributes,
namespaces, numbers, strings, or Booleans. Plans serialize to JSON.

This selection tree is the spike's “flat AST.” It is not the XPath expression
AST, a data execution plan, or a predicate-pushdown optimizer. It is a useful
schema-level intermediate representation for those future layers.

## Storage layout

There are two application keyspaces, `schema` and `data`. IDs are 32-byte
SHA-256 projection fingerprints; node IDs are big-endian u32s so byte order is
preorder. Tags and fields have fixed widths except the final instance key.

| Keyspace | Key | Value |
|---|---|---|
| schema | `active` | Active 32-byte projection ID |
| schema | `m + projection` | Version, compiler provenance, modules, row count |
| schema | `r + projection + node` | JSON flat row, two parents, paths, subtree end |
| schema | `c + projection + view + parent + child` | Empty; direct-child index |
| data | `v + projection + schema-node + instance` | JSON lexical value and instance key |

Rows and both child indexes publish with the active pointer in one atomic batch
using `SyncAll`. A failed import leaves the previous active generation intact.
Child-axis iteration scans only that parent's child index. A preorder cursor
range-decodes one row at a time; skipping a subtree changes the range start.
Node handles cache one decoded row, keeping identity, node-kind and ordering
comparisons free of storage I/O. Parent navigation performs a point lookup.

Schema generations are immutable and retained indefinitely in this spike.
`install` can reactivate an existing fingerprint and retain its recorded
compiler provenance. Concurrent first installs of the same fingerprint may
record the last writer's provenance; their structural rows are identical.
Plain Fjall atomic batches do not provide a
multi-step serializable transaction API. Concurrent installs use last-writer
wins for the active pointer. An old snapshot continues seeing its original
data values as well as its original schema. Release long-lived snapshots when
finished because they retain old storage versions.

Data values are only a minimal demonstration of schema-to-data lookup. An
instance key is an opaque caller-owned UTF-8 string of 1..=65,498 bytes, ordered
by its bytes; it is not parsed as a YANG instance-identifier. Reusing a key
replaces its lexical value. There is no instance hierarchy, list-key codec,
type validation, automatic defaults, migration, or data XPath navigator here.

## Performance and next steps

`cargo bench --locked --bench schema` measures point reads, preorder decoding,
subtree skipping, XPath path/metadata selection, plan building, and walking a
built plan. Setup, schema import, and XPath compilation occur outside timings.
See [BENCHMARKS.md](BENCHMARKS.md) for the measured baseline and limits.

The schema cursor streams storage rows; the XPath evaluator still materializes
intermediate node sets, and the selection plan materializes matching rows and
ancestors. Each criterion currently runs independently. There are no indexes
for names or metadata predicates, so broad metadata selectors scan the tree.
JSON favors inspection over compactness. Any further changes should be measured
against representative large schemas: candidate improvements are compact row
encoding, a snapshot-local row cache, name/metadata indexes, and sharing scans
between criteria. These require planning support beyond the generic Navigator.

The projection omits full type restrictions, defaults, deviations, identities,
extension semantics, and constraint contexts. Its fingerprint identifies only
what is represented here; do not use it as proof of full schema compatibility
or reuse persisted data across different compiler outputs without a separate
schema-set identity/migration policy. Garbage collection, storage migrations,
multi-writer data transactions, operations/notifications, and complete YANG
validation remain outside this spike.

## Verification

```sh
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
# Optional, after building the exporter:
python3 exporter/test_exporter.py /path/to/schema-export
```

Rust integration tests compare 17 selectors against independently specified
YANG fixture expectations and cover persistence, shared snapshots, generation
isolation, both tree views, reverse axes, unions, cursor bounds, malformed
imports, namespace collisions, multi-criterion plans, and Fjall key limits.
The eight exporter tests run actual libyang compilation, including feature
selection, deterministic output, physical case boundaries, presence refinements,
depth boundaries, and errors. The parent crate retains its Jaxen-based oracle
suite; run that separately from the repository root with `cargo test`.
