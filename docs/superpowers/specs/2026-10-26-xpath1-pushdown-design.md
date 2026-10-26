# xpath1 0.4.0 — storage pushdown completion (WP10)

- **Date:** 2026-10-26
- **Repo:** `github.com/strider4560/xpath`, drop in at `docs/superpowers/specs/2026-10-26-xpath1-pushdown-design.md`
- **Baseline:** 0.3.0 (WP9). Rust 1.85. No new dependencies.
- **Consumer:** the NMDA-over-Fjall store (`DataView: Navigator` over SID-path keys). Nothing here is store-specific.

## Goal

Complete the family of storage hooks that D21 (0.2.0) began, so that a storage-backed `Navigator` can answer every step shape that has a prefix-scan or point-get form without the evaluator materializing intermediate node sets it will immediately reduce. The family is closed and small: a step is storage-answerable iff it is a `child` or `descendant(-or-self)` axis with a name test and (for `child`) an optional leading key run, and the aggregates `count()` and existence over such a step. Everything else continues to materialize under `Limits`, exactly as today.

This release does **not** add streaming evaluation, an optimizer, lazy node sets, or any change to evaluation results. The design spec's statement "results and intermediate node sets are materialized; this version does not promise bounded-memory streaming" remains true. It also does not add YANG functions (`current`, `deref`, `derived-from`, …): those stay with the YANG library, as the README says.

## Design decisions

Every addition follows the D21 contract: a defaulted `Navigator` method whose default declines (`None`), an evaluator path that consults it first and falls back to the existing code on `None`, results identical with and without the hook, and the work budget refunded on a declined attempt so the fallback is indistinguishable. The Jaxen corpus and the libxml2 differential must pass unchanged; a navigator that answers every hook and one that declines every hook must produce byte-identical results on the full test matrix.

Hooks are asked only for `Root` and `Element` context nodes and only for name tests whose expanded name resolved; a navigator receives an `&ExpandedName` and never a prefix.

## R7 — public key-run analysis (`ast`)

Expose the evaluator's R2 recognizer so hosts (planners, access-control path matchers) reuse the exact grammar instead of copying `eval.rs`.

```rust
// src/ast.rs (public module)
impl Step {
    /// Length of the leading run of predicates of the form `k = e` / `e = k`
    /// that `Navigator::child_by_key` can answer: `k` a one-step relative
    /// `child::` name test without predicates, `e` context-independent.
    /// Namespace resolution is not applied here (the evaluator additionally
    /// ends the run at a key whose prefix is unbound).
    pub fn key_run(&self) -> usize;
}
/// `(key name, right-hand side)` for one predicate of that form, else `None`.
pub fn key_pair(predicate: &Expr) -> Option<(&QName, &Expr)>;
/// A string or number literal, a variable, `current()`, or a path rooted at `current()`.
pub fn context_independent(expr: &Expr) -> bool;
```

`eval.rs` calls these instead of its private copies; `key_run` in the evaluator keeps its namespace check on top. No behaviour change.

## R8 — descendant name pushdown

```rust
// src/model.rs
/// The element descendants of `node` whose expanded name is `name`, in
/// document order, without duplicates, or `None` if the adapter cannot
/// answer without a scan. Does not include `node` itself.
fn descendants_named(&self, _node: &Self::Node, _name: &ExpandedName) -> Option<Vec<Self::Node>> { None }
```

Evaluator (`step`): for a step with axis `Descendant` or `DescendantOrSelf` and `ResolvedTest::Name`, ask `descendants_named` first; on `Some(v)`, keep only `NodeKind::Element` results, validate each handle, spend one unit per returned node, and for `DescendantOrSelf` prepend the context node if it matches the test. On `None` fall back to `descendants()` + `matches`. `Some` must contain exactly the element descendants a scan would select, in document order. Vectors returned by the hook are given to the pool after use, as for R6.

Rationale: on a prefix-ordered store, "all `interface` rows under X" is one key-only scan; today it is a full materialization of every descendant node (`eval.rs:758-776`) followed by a name filter.

## R9 — count and existence pushdown

```rust
// src/model.rs
/// Number of element children of `parent` named `name`, or `None` to decline.
/// Must equal `children_named(parent, name).len()` when both answer.
fn count_children_named(&self, _parent: &Self::Node, _name: &ExpandedName) -> Option<usize> { None }
/// Whether `parent` has at least one element child named `name`, or `None`.
fn exists_child_named(&self, _parent: &Self::Node, _name: &ExpandedName) -> Option<bool> { None }
```

Evaluator rewrites, recognized structurally in `expr`:

1. `count(P)` where `P` is `Expr::Path { base: None, absolute: false, steps: [Step { axis: Child, test: Name(q), predicates: [] }] }` and the focus node is Root/Element → `count_children_named(focus, name)`; `Some(n)` → `Value::Number(n as f64)`, spending one unit; `None` → ordinary evaluation.
2. A path of the same shape in **boolean position** — the operand of `and`/`or`, the argument of `boolean()` or `not()`, a predicate whose value is not numeric — → `exists_child_named`; `Some(b)` → `Value::Boolean(b)`. A predicate is boolean position only when the step's expression cannot yield a number, which a bare path cannot, so `[name]` qualifies and `[1]` does not.

Only the predicate-free, single-step, relative `child::name` shape is pushed; anything else (predicates, multi-step, absolute, descendant) evaluates as today. `count` with a hook answer must equal `count` without it; the conformance kit (R11) checks that per fixture.

Rationale: YANG `min-elements`/`max-elements`, `count(../x) < n` constraints and existence tests over large lists are the dominant validation cost on a store; the hook answers them with a counted or short-circuited key-only scan.

## R10 — cooperative cancellation in `Limits`

```rust
// src/context.rs
pub struct Limits {
    pub max_operations: usize,
    pub max_depth: usize,
    /// Checked once per `max_operations / 256` units (at least every 4096);
    /// a set flag ends evaluation with `ErrorKind::Limit`.
    pub cancel: Option<&'static core::sync::atomic::AtomicBool>  // see note
    /// Wall-clock deadline checked on the same cadence.
    pub deadline: Option<std::time::Instant>,
}
```

Note on lifetime: `Limits` is `Copy` and stored in `Context<'a, N>`; use `Option<&'a AtomicBool>` by adding the lifetime to `Limits<'a>` with a `'static` default, **or** keep `Limits` lifetime-free and put `cancel: Option<&'a AtomicBool>` on `Context` via `Context::with_cancel(&'a AtomicBool)`. The second keeps `Limits` `Copy` and is preferred. `deadline` uses `std::time::Instant` (the crate already depends on `std`). Checks happen in `spend` on a counter, not on every unit, so the per-operation cost is a decrement and a compare. Both default to `None`; behaviour without them is unchanged.

Rationale: a host evaluating thousands of constraints per transaction needs one RPC-level budget; `max_operations` is per evaluation.

## R11 — Navigator conformance kit

A `pub mod conformance` behind a `conformance` cargo feature (dev-facing, no runtime cost): the Jaxen-style harness parameterized over `Navigator`. The host supplies a function that builds its navigator from an XML fixture (the kit renders each fixture document and expectations); the kit runs the full corpus plus a **hook-consistency matrix**: for every fixture context and every name present, `children_named`, `descendants_named`, `count_children_named`, `exists_child_named` and `child_by_key` must agree with the scan-based defaults, and `compare_document_order` must be a total order consistent with `children`. The XML adapter is the reference implementation and must pass it.

Rationale: a storage navigator implements seven hooks whose only contract is "agree with the scan"; the kit makes that contract executable.

## R12 — documentation and bounds

- State in the README and `model.rs` that `FunctionTable<N>` is `Send + Sync` when its closures are (add `static_assertions`-free compile tests), and that `Namespaces`, `NodeSetPool` and `Context` are per-thread, per-evaluation-set objects.
- Document the hook family as closed: the table of step shapes with their answering hook, and that every other shape materializes.
- CHANGELOG entry 0.4.0 — WP10 with the R7–R12 labels.

## Explicitly not in scope

- Iterator-returning `children_named` / streaming `step`: without a streaming `filter` (which needs `position()`/`last()`) the evaluator collects immediately; nothing is gained and the materialization model would no longer be the single documented one.
- `evaluate_streaming`: a second evaluator.
- Borrowed string values (R4): deferred again until a measured workload needs it.
- YANG function pack: belongs above the engine.
- Any change to `Value`, coercions, document-order semantics, or the parser.

## Correctness evidence

- Jaxen corpus (296 assertion definitions) and the 10 000-case libxml2 differential pass unchanged with the XML adapter (which implements none of the new hooks).
- A test navigator over the XML arena that implements **all** new hooks, and one that implements none, run the full matrix with identical results; a third implements hooks that deliberately lie (wrong order, duplicates, wrong count) and the conformance kit must report each lie.
- Property: for random fixture trees and random names, `count_children_named == children_named.len()` and `exists_child_named == !children_named.is_empty()` whenever both answer.
- Budget-refund tests for each declined hook (existing pattern for R2).
- Cancellation tests: flag set mid-evaluation → `ErrorKind::Limit` within the check cadence; deadline in the past → `Limit` before any navigation.
- Benchmarks (reported, not asserted): `count(child::x)` over 1 000-entry lists with and without R9; `//x` over the schema-shaped fixture with and without R8.

## Deliverables

Source, tests, the conformance kit, CHANGELOG, README updates, `docs/<task>-report.md` with test accounting. Backwards compatible for navigator implementors (every new method has a default) and for callers (`Limits` fields are additive; `Context` gains one builder method). Version 0.4.0.
