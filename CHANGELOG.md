# Changelog

## 0.4.0 — 2026-10-26 — WP10

Completes the storage-hook family begun in 0.2.0 (D21): every step shape with
a prefix-scan or point-get form now has a hook, plus cooperative cancellation
and a Navigator conformance kit. Every new `Navigator` method has a default
that declines, so navigator implementations compile unchanged. Evaluation
results are unchanged: the Jaxen corpus (296 assertion definitions) and the
10,000-case libxml2 differential pass unchanged with the XML adapter (which
implements none of the new hooks), and a navigator answering every hook gives
byte-identical output. Node sets are still materialized; there is no streaming
evaluation or optimizer.

### Added

- **R7, public key-run analysis** (`ast`). `Step::key_run(&self) -> usize`,
  `ast::key_pair(&Expr) -> Option<(&QName, &Expr)>` and
  `ast::context_independent(&Expr) -> bool` expose the evaluator's R2
  recognizer, so planners and access-control matchers reuse its exact
  grammar. The evaluator now calls them; its own `key_run` only adds the
  unbound-prefix check on top.
- **R8, `Navigator::descendants_named(&self, node, name) -> Option<Vec<Node>>`.**
  Asked for `descendant::name` and `descendant-or-self::name` steps, and for
  `//name` (the parser's `descendant-or-self::node()/child::name` when neither
  step has predicates). Element descendants only, in document order, without
  duplicates, not including `node`; for `descendant-or-self` the evaluator
  adds the context node itself when it matches. Non-element results are
  dropped, each returned handle is validated and charged one work unit, and
  the vectors are given to the pool after use. For `//name` over several
  input nodes, one declined input makes the evaluator refund the attempt and
  evaluate both steps normally.
- **R9, `Navigator::count_children_named(&self, parent, name) -> Option<usize>`
  and `Navigator::exists_child_named(&self, parent, name) -> Option<bool>`.**
  Only for a relative, single `child::name` step without predicates at a root
  or element focus: `count(name)` asks `count_children_named` (one work unit
  when answered); the same path in boolean position (an operand of `and` /
  `or`, the argument of `boolean()` or `not()`, or a predicate `[name]`) asks
  `exists_child_named`. Every other shape (predicates, several steps, absolute,
  descendant, a filter base) evaluates as before.
- **R10, cooperative cancellation.** `Limits::deadline: Option<Instant>` and
  `Context::with_cancel(&'a AtomicBool)`. Both are read when an evaluation
  starts (so a passed deadline or a set flag ends it before any navigation)
  and then once per `max_operations / 256` work units, clamped to 1..=4096;
  either ends evaluation with `ErrorKind::Limit` ("XPath evaluation
  cancelled" / "XPath evaluation deadline exceeded"). The check is a
  decrement and a compare per charge; without either, behavior is unchanged.
  Work charged inside core functions (`lang`, `id`, `sum`) counts toward the
  budget but not toward the check cadence.
- **R11, Navigator conformance kit** (`conformance` feature, implies `xml`;
  module `xpath1::conformance`). `conformance::run(build)` builds the host's
  navigator from each of the 22 pinned Jaxen fixture documents
  (`conformance::DOCUMENTS`), runs the full corpus against it (through a
  forwarding multi-document navigator, so the host's hooks are exercised),
  and runs `conformance::check_hooks(&nav, &root)` on each fixture: for every
  root or element node and every element name present (plus an absent one),
  `children_named`, `descendants_named`, `count_children_named`,
  `exists_child_named` and `child_by_key` (probed with single-occurrence
  child elements as string keys) must agree with a scan of `children` and
  `name`; `name_matches` with `name`; `parent` with the axes; and
  `compare_document_order` must be a total order consistent with the tree
  (checked on consecutive nodes, within namespace and attribute groups, and
  on a deterministic sample of pairs). `conformance::Report` carries the
  corpus accounting, failures and hook violations. `conformance::ScanHooks<N>`
  wraps any navigator and answers every hook from scans: a reference
  "all hooks" adapter. The XML adapter passes the kit.
- **R12, thread-safety bounds.** `FunctionTable<N, F = LocalFunction<N>>` is
  `Send + Sync` exactly when its closures are: `FunctionTable::new()` builds a
  per-thread table (any `'static` closure, as before);
  `FunctionTable::new_sync()` builds a `SyncFunctionTable<N>` that accepts
  only `Send + Sync` closures and is itself `Send + Sync` for every navigator
  type. `Context::with_functions` accepts either. New exports:
  `LocalFunction`, `SyncFunction`, `SyncFunctionTable`, and the sealed
  `TableFunction`. Compile tests (doctests, including a `compile_fail` one)
  pin both bounds. `Namespaces`, `NodeSetPool` and `Context` are documented
  as per-thread objects for one set of evaluations.
- Documentation of the hook family as closed: the shape-to-hook table is in
  the `Navigator` docs and the README; every other shape materializes.
- Benchmarks: `pushdown/count` (`count(child::x)` over 1,000 entries) and
  `pushdown/descendants`, `pushdown/select_descendants` (`//x` over the
  schema-shaped fixture), each with and without the hooks, over a store-shaped
  indexed adapter.
- `tools/differential` client: `XPATH1_HOOKS=1` wraps the fixture in
  `ScanHooks`, so the libxml2 comparison also covers every hook path.

### Changed

- `Limits` has a third public field, `deadline`. Code that builds `Limits`
  with a struct literal must add `..Limits::default()` (or `deadline: None`);
  code using `Limits::default()` and field assignment is unaffected.
- `FunctionTable` has a second, defaulted type parameter. `FunctionTable<N>`
  and `FunctionTable::new()` mean what they meant before.

## 0.3.0 — 2026-10-12

Shared per-read namespace bindings and an opt-in node-set buffer pool (WP9).
Backwards compatible; no `Navigator` changes. Evaluation results are unchanged; the Jaxen corpus (296 assertion
definitions) and the libxml2 differential pass unchanged.

### Added

- **`Namespaces`** (`new`, `with_namespace`, `with_default_element_namespace`,
  `namespace`, `default_element_namespace`, `Default`): an immutable binding set
  shared by many contexts through `Context::with_namespaces(&'a Namespaces)`.
  Resolution order: `xml` is always the XML namespace; other prefixes resolve
  through the context's own `with_namespace` bindings, then the shared set. The
  default element namespace is the context's own if non-empty, else the shared
  set's. The set caches every resolved name (`Rc<ExpandedName>`), so a warm
  evaluation resolves name tests, variables and function names without
  allocating. **Cache rule:** the cache is read and written only when the
  context's own namespace map and own default element namespace are both empty
  (every lookup went through the shared set), so a context layering its own
  bindings never poisons it. `Namespaces` is not `Send`/`Sync`; use one per
  thread (typically one per read). Contexts that do not use `with_namespaces`
  resolve names as before but now pay one extra `Rc` allocation per resolved
  name per evaluation. Such contexts also resolve R2 key names once per input
  node instead of once per step (an `Rc` plus an `ExpandedName` per key per
  node). Rebinding a `Namespaces` (`with_namespace`,
  `with_default_element_namespace`) clears its cache. Namespace-wildcard (`p:*`) and processing-instruction
  target tests borrow their URI and target instead of allocating.
- `ast::QName` derives `Hash`.
- **`NodeSetPool<T>`** (`new`, `take`, `give`, `len`, `is_empty`, `Default`,
  `Debug`) and **`Context::with_pool(&'a NodeSetPool<N::Node>)`**: an opt-in
  pool of empty node-set buffers. With a pool, the evaluator takes every
  intermediate node-set vector (step results, axis scans, predicate and
  operand node-sets, variable copies, union merges) from it and gives it back
  when done, so warm evaluations reuse capacity instead of allocating. The
  node-set an evaluation returns is always owned by the caller; a host may
  `give` it back once finished. Vectors returned by `Navigator::children_named`
  and node-sets returned by extension functions are given to the pool after
  use, so a navigator or function that builds them from the same pool has them
  recycled. `give` clears the vector and keeps its capacity; it drops vectors
  without capacity and any vector given while the pool holds 64. Not `Sync`;
  use one per thread. Without a pool, behavior and allocations
  are as before.

### Changed

- `xml::Document` overrides `Navigator::name_matches` to compare the stored
  name in place instead of cloning an `ExpandedName` per candidate.

## 0.2.0 — 2026-10-11

Embedding hooks for application-backed (row/cursor) navigators (D21). All
additions are backwards compatible for navigator implementors: every new
`Navigator` method has a default. Evaluation results are unchanged; the Jaxen
corpus (296 assertion definitions) and the 10,000-case libxml2 differential
pass unchanged.

### Added

- **R1, AST read access.** `xpath1::ast` is public (`Expr`, `Step`, `NodeTest`,
  `QName`, `BinaryOp`), and `XPath::ast(&self) -> &ast::Expr` exposes the compiled
  tree read-only for static analysis such as install-time name resolution and
  result-kind inference. `XPath` values are still produced only by the parser.
  `QName` and `Step` are `#[non_exhaustive]`.
- **R3, `Navigator::name_matches(&self, node, name) -> bool`.** Every evaluator
  name test now calls it. The default compares `name(node)` to `name`, so an
  adapter can override it to compare its stored name without allocating an
  `ExpandedName` per candidate.
- **R6, `Navigator::children_named(&self, parent, name) -> Option<Vec<Node>>`.**
  A `child::name` step asks it first (document order required). The default
  `None` falls back to `children()` plus `name_matches`.
- **R2, `Navigator::child_by_key(&self, parent, name, keys) -> Option<Option<Node>>`**
  with `keys: &[(ExpandedName, Value<Node>)]`. A `child::name` step whose leading
  predicates are `[k = e]` (either operand order), with `k` a one-step child name
  test and `e` a string/number literal, a variable, `current()` or a
  `current()`-rooted path, evaluates each `e` once against the step's context node
  and passes the values verbatim. The navigator decides equality.
  `Some(Some(n))` continues with `[n]` and the remaining predicates; `Some(None)`
  selects nothing; `None` (the default) evaluates normally.
- **R5, reusable contexts.** `FunctionTable<N>` (`new`, `insert`) holds extension
  functions registered once; `Context::with_functions(&'a FunctionTable<N>)`
  shares it, and `Context::with_function` entries shadow table entries.
  `Context::set_node(&mut self, node)` re-focuses a context at position 1 of 1
  and keeps its namespace, variable and function bindings and limits.

### Notes

- `name_matches` returns `bool`, so with the default implementation a name that
  fails to read (`name()` returns `Err`) is now a non-match during a name test
  rather than an evaluation error. Navigators whose `name()` is infallible (the
  XML adapter, all bundled tests) are unaffected; `name()`, `local-name()` and
  `namespace-uri()` still propagate errors.
- The R2 fast path assumes `current()` is YANG's function returning the
  context's initial node. `child_by_key` must decline (`None`) unless the
  keys cover every key leaf exactly once.
- R4 (borrowed string values) is deferred: `string_value()` returns
  `String`; a `Cow<str>` or caller-supplied buffer would let leaf values come
  straight from the row.

## 0.1.0

Initial release.
