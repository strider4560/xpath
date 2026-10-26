# xpath1

XPath 1.0 in safe Rust, evaluated over application-owned trees. Designed as a
dependency for a Rust YANG implementation: node handles can identify records in
a storage snapshot without constructing an XML document or a YANG object tree.

The crate implements all thirteen axes, the four XPath value types, predicates,
union, operators, variables, namespace resolution and all 27 core functions.
Expressions compile once and can be reused with different trees and bindings.
The handwritten parser and evaluator have no mandatory dependencies. The default
`xml` feature adds an owned XML adapter using roxmltree 0.20.

## Use as a dependency

This source distribution is not published to crates.io. Extract it beside your
application and use a path dependency:

```toml
[dependencies]
xpath1 = { path = "../xpath1", default-features = false }
```

Enable `features = ["xml"]` to use the XML adapter. Rust 1.85 or newer is required.

```rust
use xpath1::{xml::Document, Context, Value, XPath};

let document = Document::parse(
    "<interfaces><interface enabled='true'>eth0</interface></interfaces>"
)?;
let expression = XPath::compile("/interfaces/interface[@enabled = $enabled]")?;
let context = Context::new(&document, document.root())
    .with_variable("enabled", Value::String("true".into()));

let selected = expression.select(&context)?;
assert_eq!(selected.len(), 1);
assert_eq!(expression.evaluate(&context)?.to_string(&document)?, "eth0");
# Ok::<(), xpath1::Error>(())
```

`evaluate` returns `Result<Value<Node>>`, where `Value` is `Nodes`, `Boolean`,
`Number(f64)`, or `String`. `select` is a convenience that requires a node-set
result. `Value::to_boolean`, `to_number`, and `to_string` apply XPath coercions.
Returned node sets are deduplicated and in document order. This is an API
ordering guarantee; reverse axes still apply predicates in reverse axis order.

## Bring your own tree

Implement `Navigator` for a borrowed transaction/snapshot or a tree adapter.
The required operations are node kind, parent, child iterator, expanded name,
string value and comparison in document order. Attribute and namespace iterators,
qualified names, ID lookup and handle validation have defaults. Most production
adapters will override the relevant defaults.

Children, attributes and namespaces use fallible iterators, so an implementation
can expose a K/V cursor. Handles need `Clone + Eq + Hash + Debug`; they need not
contain an owned node. Iterator or lookup errors propagate as `ErrorKind::Navigation`
when returned with that category. All methods are synchronous. Use a stable read
snapshot across an evaluation; concurrent mutation of the logical tree is outside
the contract.

The tree must provide these XPath invariants:

- A distinct root node above the document element.
- Stable node identity and total document order. Namespaces and attributes follow
  their owning element and precede its children; order within each group is stable
  but implementation-defined.
- Attributes and namespace nodes have element parents but are not children.
  In-scope namespace nodes have distinct identities for each owning element.
- Element/root string values concatenate descendant text in document order.
- Names are namespace URI plus local part. Preserve a lexical QName separately
  when the embedding needs `name()` to retain prefixes.

See `examples/custom_tree.rs` for a complete dependency-free adapter. See
`src/model.rs` for the exact contract.

Compiled `XPath` values can be shared across threads. A `FunctionTable` built
with `FunctionTable::new_sync()` (a `SyncFunctionTable`, holding only
`Send + Sync` closures) is itself `Send + Sync` and can be shared too; one
built with `FunctionTable::new()` may hold any closure and stays on its thread.
`Namespaces`, `NodeSetPool` and `Context` are per-thread objects for one set of
evaluations: build them for each caller (or each read).

### Storage hooks

A storage-backed navigator can answer the step shapes that have a prefix-scan
or point-get form, instead of letting the evaluator enumerate nodes. The
family is closed:

| Shape (name test resolved; context node a root or element) | Hook |
|---|---|
| any name test | `name_matches` |
| `child::name` | `children_named` |
| `child::name[k = e]...` (leading key run, `ast::Step::key_run`) | `child_by_key` |
| `descendant::name`, `descendant-or-self::name`, `//name` | `descendants_named` |
| `count(name)` | `count_children_named` |
| `name` as an `and`/`or` operand, in `boolean()`/`not()`, or as `[name]` | `exists_child_named` |

Every hook has a default that declines (`None`); the evaluator then scans
exactly as before, with the work budget spent on the attempt refunded, and an
answer must agree exactly with that scan. Every other shape (predicates on the
counted or tested path, several steps, wildcards, other axes) materializes
under `Limits`. Navigators receive expanded names, never prefixes.

The `conformance` feature (`xpath1::conformance`) makes "agree with the scan"
executable: `conformance::run(|xml| ...)` builds your navigator from each
pinned Jaxen fixture, runs the full corpus against it, and checks every hook
and `compare_document_order` against scans of the same tree.
`conformance::check_hooks(&navigator, &root)` runs the hook matrix on any
tree, for example a store's own data, and `conformance::ScanHooks` is a
reference adapter that answers every hook.

## Integration with YANG

XPath 1.0 is the default: unprefixed element names match the null namespace.
Bind expression prefixes using `with_namespace`, independently of data prefixes.
For YANG's module namespace rules, opt into
`with_default_element_namespace(module_namespace)`. This changes only unprefixed
element name tests, not attributes, variables, namespace nodes or functions.
Resolve imported YANG prefixes to namespace URIs in the calling library.

Register extension functions with `with_function(ExpandedName, closure)`.
Callbacks receive the navigator, current node, position, size, and original
context node. This supports a host implementation of YANG's `current()` and
other functions without adding them to the XPath 1.0 builtin library.
Core functions cannot be overridden. `examples/yang.rs` demonstrates module
namespaces and `current()` in a predicate.

To reuse registrations, build a `FunctionTable` once and attach it with
`with_functions(&table)` (per-context `with_function` entries shadow it), and
re-focus one context across many evaluations with `set_node`. Storage-backed
navigators can override the storage hooks above to answer name tests,
`parent/leaf`, `list[key = value]`, `//node`, `count(list)` and existence
tests without scanning; `XPath::ast()` exposes the compiled `ast::Expr` for
static analysis, and `ast::Step::key_run`, `ast::key_pair` and
`ast::context_independent` are the evaluator's own key-predicate recognizer.
See `CHANGELOG.md` for the exact contracts.

The YANG library remains responsible for the accessible tree, `when` dummy-node
rules, schema/default handling, identity and leafref resolution, and the YANG
extension-function semantics. Evaluate a `must` or `when` result with
`Value::to_boolean()` after constructing that context. This crate does not link
libyang, CEL, Java, or Fjall; storage integration is through `Navigator`.

An executable [Fjall flat-schema spike](spikes/fjall-schema/README.md) stores a
real libyang-compiled schema in `schema` and illustrative values in `data`,
adapts the flat index to this crate, and builds cursor-walkable selection trees.
It is a separate experimental package requiring Rust 1.90; the core crate's
dependencies and Rust 1.85 requirement are unchanged.

## Correctness and limits

`cargo test --test jaxen -- --nocapture` reads the original pinned Jaxen XML
catalog and expectations directly. It executes 296 assertion definitions in 62
contexts: 277 core and 19 extension assertions, including nine expected errors
and fifteen nested assertions. Extensions are implemented only in the test
harness. Six count-less selectors are treated exactly as upstream: they supply
the first selected node to their nested assertions without asserting a count.
There are no skipped fixture assertions or edited expectations.

The fixture data and BSD license are in `vendor/jaxen`, pinned to commit
`860cfd1899a1d662e4cb187ffd10fef2fbeb01bc`. The Rust implementation is MIT licensed.
Focused regression tests additionally cover parsing errors, Unicode, IEEE edge
cases, integer formatting, reverse-axis predicates, existential inequality,
namespaces, custom trees, extension focus, handle errors and work limits.
This is corpus-based verification, not a proof of exhaustive conformance or a
claim to execute every Jaxen Java unit test.

Compilation limits are 1 MiB of source, 65,536 tokens, 64 nested expressions, and
AST height 128. Exceeding a limit returns `ErrorKind::Limit`. Syntax errors include
byte ranges into the original expression. Evaluation defaults to 1,000,000 work
units and depth 128; configure `Limits` per context. Work includes AST evaluation,
node processing, axis traversal and core-function navigation. These are work
bounds, not memory quotas: host callbacks and individual bytes in strings are not
metered. Extension functions must enforce their own budgets.

For an RPC-level budget across many evaluations, set `Limits::deadline` (a
`std::time::Instant`) and/or `Context::with_cancel(&AtomicBool)`. Both are
checked when an evaluation starts and then every `max_operations / 256` work
units (at most 4096 apart); either ends the evaluation with `ErrorKind::Limit`.
Cancellation is cooperative: a long-running navigator or extension call is not
interrupted.

The XML adapter has an expanded element depth limit of 256 to protect its parser
stack, including internal entity markup. It does not fetch external DTDs/entities,
validate DTDs, synthesize DTD defaults, or infer DTD ID typing. Register known IDs
with `Document::register_id`; an attribute spelled `id` is not automatically typed
as an XML ID. A custom navigator can implement the full host ID policy.

Node sets and intermediate results are materialized. Cursor input does not imply
constant-memory evaluation; this version does not promise bounded-memory
streaming. The storage hooks push the shapes listed above to the navigator;
everything else is evaluated over materialized node sets. The evaluator reuses
node buffers, merges sorted unions, and indexes string values for large node-set
equality comparisons. It does not include a general query optimizer or storage
indexes of its own.

## Performance

Compile selectors once and reuse them across contexts. The Criterion suite
covers compilation, schema-shaped selection, data lists of 10–1,000 entries,
`when` evaluation, axes, unions, node-set comparisons, and XML parsing:

```sh
cargo bench --locked --bench xpath
cargo bench --locked --no-default-features --bench xpath
cargo bench --locked --bench xpath -- --counts
```

Fixture creation and expression compilation are outside cached-evaluation timing
loops. Expected answers are checked before timing. `--counts` collects navigator
calls separately, without instrumenting the timed runs. Criterion is a development
dependency only. See [the performance report](docs/PERFORMANCE.md) for before/after
results, raw samples, reproduction commands, and remaining bottlenecks.

## Build and verify

```sh
cargo test --locked --all-features      # includes the conformance kit tests
cargo test --locked --no-default-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo run --example xml
cargo run --example custom_tree --no-default-features
cargo run --example yang
```

See `docs/CONFORMANCE.md` for release verification and `tools/differential` for
optional differential checks. Normal tests need neither Java nor network access
after Cargo dependencies are cached.
