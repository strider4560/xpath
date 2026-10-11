# Changelog

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
  context's initial node. Keys must identify at most one entry.
- R4 is deferred; see `docs/PERFORMANCE.md`.

## 0.1.0

Initial release.
