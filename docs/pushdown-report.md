# WP10 report: storage pushdown completion (0.4.0)

Implements `docs/superpowers/specs/2026-10-26-xpath1-pushdown-design.md`
(R7–R12). Rust 1.85 MSRV kept; no new dependencies (the `conformance`
feature reuses the existing optional `roxmltree`).

## What changed

| Item | Where | Summary |
|---|---|---|
| R7 | `src/ast.rs` | `Step::key_run`, `ast::key_pair`, `ast::context_independent` are public; `eval.rs` uses them and only adds the unbound-prefix check. |
| R8 | `src/model.rs`, `src/eval.rs` | `Navigator::descendants_named`; asked for `descendant::name`, `descendant-or-self::name` and `//name`. |
| R9 | `src/model.rs`, `src/eval.rs` | `Navigator::count_children_named` for `count(name)`; `Navigator::exists_child_named` for `name` in boolean position. |
| R10 | `src/context.rs`, `src/eval.rs` | `Limits::deadline`, `Context::with_cancel`; checked at start and every `max_operations / 256` units (1..=4096). |
| R11 | `src/conformance.rs` (feature `conformance`) | `run`, `check_hooks`, `Report`, `DOCUMENTS`, `ScanHooks`. |
| R12 | `src/context.rs`, `src/model.rs`, README | `FunctionTable::new_sync` / `SyncFunctionTable` (`Send + Sync`); per-thread objects documented; closed hook table documented. |

## Decisions and deviations from the spec

- **`//name` is part of R8.** The parser spells `//x` as
  `descendant-or-self::node()/child::x`, so a hook keyed only on a
  `descendant` axis would never see `//x`. The evaluator recognizes that
  two-step pair when *neither* step has predicates (then it is exactly
  `descendant::x`) and asks `descendants_named` for each root/element input.
  If any input is declined, the attempt is refunded and both steps run as
  before. `//x[pred]` is not pushed: its positions are per parent, not per
  descendant list.
- **R9 budget.** `count(name)` answered costs the call's own unit plus one;
  an answered existence test costs one unit in place of the path's
  evaluation. Nothing is charged before a hook declines, so no refund is
  needed there.
- **R10 placement.** As the spec prefers: `deadline` on `Limits` (still
  `Copy`), the flag on `Context::with_cancel(&'a AtomicBool)`. The deadline
  is also checked before the first navigation call. Charges made inside core
  functions (`lang`, `id`, `sum`) count toward the budget but do not advance
  the check countdown.
- **`Limits` struct literals.** Adding `deadline` is additive for code that
  uses `Limits::default()`, but a struct literal without
  `..Limits::default()` no longer compiles. Three test literals in this repo
  were updated. This is noted in the CHANGELOG; the spec described the change
  as fully backwards compatible for callers, which holds only for
  non-literal construction.
- **R12 `Send + Sync`.** In 0.3.0 `FunctionTable<N>` stored
  `Box<dyn Fn>` without auto-trait bounds, so it was never `Send`/`Sync`. A
  trait object cannot be conditionally `Send`, so the table gained a
  defaulted second parameter (the stored closure type). `FunctionTable<N>` /
  `FunctionTable::new()` are unchanged; `FunctionTable::new_sync()` builds a
  `SyncFunctionTable<N>` that only accepts `Send + Sync` closures and is
  `Send + Sync` for every navigator type. The bounds are pinned by doctests (a
  `compile_fail` one for `new`, a passing one for `new_sync`) and an
  integration test sharing one table across threads; no `static_assertions`.
- **R11 kit shape.** The host passes `FnMut(&str) -> Result<(N, N::Node)>`
  and gets back a `Report`. The kit wraps the host's per-document navigators
  in a private multi-document navigator that forwards every method, hooks
  included, so `document()` can cross fixtures and the corpus exercises the
  host's hooks. `tests/jaxen.rs` is left unchanged as the original oracle.
  `child_by_key` cannot be checked against a schema the kit does not know.
  The kit probes it with single-occurrence child elements as string keys and
  requires any answer to equal the XPath `=` selection, and to come only when
  that selection has at most one entry.
- **Reference all-hooks adapter.** `conformance::ScanHooks<N>` answers every
  hook from scans of the wrapped navigator. It is the "navigator that
  implements all hooks" in the evidence below. The XML adapter itself still
  implements none of the new hooks, as the spec requires.

## Correctness evidence

| Evidence | Result |
|---|---|
| Jaxen corpus, XML adapter (`tests/jaxen.rs`, unchanged) | 296 definitions in 62 contexts pass |
| Jaxen corpus via the kit, XML adapter | pass, same accounting (62, 277, 19, 9, 15) |
| Jaxen corpus via the kit, `ScanHooks` (every hook answers) | pass; matrix 0 violations |
| libxml2 2.14.6 differential, plain | 10,000 cases, 0 differences |
| libxml2 differential, `XPATH1_HOOKS=1` (every hook answers) | 10,000 cases, 0 differences; client output byte-identical to the plain run |
| Fixture matrix (`tests/conformance.rs`): all-hooks vs none, Debug-formatted values | 32,972 cases identical; every hook consulted |
| Lying hooks: 10 lies (reversed / duplicated children, reversed descendants, self in descendants, count off by one, negated existence, wrong key entry, ambiguous key answer, `name_matches` always true, attributes ordered after children) | each reported by `check_hooks`, and only under its own hook name |
| Corpus-only sensitivity to the same lies (ignored diagnostic test) | the corpus alone notices only 2 of 10 (reversed children: 31 failures; `name_matches`: 14); the matrix notices all 10, which is why the kit has one |
| Property: 200 random trees × 20 random (node, name) draws | `count == children_named.len()`, `exists == !children_named.is_empty()`, kit passes, 8 expression shapes equal to the scan |
| Budget refunds (`tests/pushdown.rs`) | a navigator that is asked and declines needs exactly the minimum budget of the XML adapter for 8 shapes; a `//name` attempt declined part-way (3 shapes) is refunded exactly |
| Hook answers metered | an oversized `descendants_named` answer is a `Limit` error; `count()` answers in one unit |
| Cancellation (`tests/cancellation.rs`) | flag set before / past deadline: `Limit` with zero navigator calls; flag set mid-run: ends within one check interval of child reads at 5 budget sizes; deadline passing mid-run: `Limit`; flag set from another thread: `Limit` |

## Test accounting

`cargo test --locked --all-features`: 91 Rust tests and 3 doctests pass,
1 diagnostic test ignored (0.3.0: 69 tests).

| Target | Tests | New in 0.4.0 |
|---|---:|---|
| library unit tests | 27 | — |
| `tests/cancellation.rs` (R10, no features) | 6 | all |
| `tests/comparisons.rs` | 4 | — (one `Limits` literal gained `..Limits::default()`) |
| `tests/conformance.rs` (R11, feature `conformance`) | 5 + 1 ignored | all: XML adapter passes the kit; every-hook adapter passes the kit; fixture matrix; ten lies; random-tree property; ignored corpus-sensitivity diagnostic |
| `tests/core.rs` | 9 | — (two `Limits` literals updated) |
| `tests/embedding.rs` | 10 | 2: R7 key-run analysis agrees with the evaluator; R12 sync table shared across threads |
| `tests/jaxen.rs` | 1 | — (unchanged: 296 definitions) |
| `tests/namespaces.rs` | 6 | — |
| `tests/pool.rs` | 4 | — |
| `tests/pushdown.rs` (R8, R9, feature `conformance`) | 9 | all |
| `tests/xml.rs` | 10 | — |
| doctests | 3 | all: kit run on the XML adapter; `compile_fail` for a `new` table being `Send + Sync`; a `new_sync` table is |

`cargo test --locked --no-default-features`: 48 pass (the conformance,
pushdown, Jaxen, XML and embedding targets need features). Under Rust 1.85
(`cargo +1.85 test --locked --all-features`): 94 pass.

## Release gates

| Check | Result |
|---|---|
| `cargo test --locked --all-features` (Rust 1.97) | 91 + 3 doctests pass |
| `cargo +1.85 test --locked --all-features` | 94 pass |
| `cargo test --locked --no-default-features` | 48 pass |
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass (also `--no-default-features`) |
| `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features` | pass |
| `cargo package --allow-dirty --locked --features conformance` | packaged crate (with the vendored fixtures the kit embeds) builds |
| `cargo tree --no-default-features --edges normal` | no runtime dependencies |
| examples `xml`, `custom_tree` (no default features), `yang` | run |
| `tools/differential`, plain and `XPATH1_HOOKS=1` | 10,000 cases, 0 differences each; outputs byte-identical |

## Benchmarks (reported, not asserted)

Criterion, `cargo bench --bench xpath`, Linux x86-64 container, Rust 1.97;
medians.

New `pushdown` group (`Tree::interfaces(1000)` through `support::Indexed`, a
store-shaped adapter with key-only indexes; "without" declines every hook):

| Benchmark | Without hooks | With hooks |
|---|---:|---:|
| `count(child::if:interface)` at `interfaces`, 1,000 entries | 8.4 µs | 3.0 µs |
| `count(//if:mtu)` | 157 µs | 2.3 µs |
| `//if:mtu` (select 1,000 nodes) | 143 µs | 2.3 µs |

Existing suite against the 0.3.0 baseline (`--save-baseline` in a worktree of
the previous commit, then `--baseline`): no group moves consistently. Changes
are within about ±10%, and that is this machine's noise: `compile/axes`
moved +11% with the parser untouched. Repeated A/B runs of the smallest
expressions, after keeping the non-hook path to a single compare per charge
and an inline shape check before each boolean-position lookup:

| Benchmark | 0.3.0 | 0.4.0 |
|---|---:|---:|
| `when/false` | 456–494 ns | 465–466 ns |
| `xml/when` | 777–787 ns | 781–791 ns |
| `scalar/cached` | 297–313 ns | 319–335 ns |

`scalar/cached` (`(17 + 3) * 4 div 2 = 40 and not(false())`, no navigation)
stays about 15–20 ns (5–6%) slower. Part of that is the boolean-position
check on `and` / `not()` operands; the rest is at the noise floor.

## Concerns and follow-ups

- `//x[pred]` with a position-independent predicate could also be pushed
  (group the answer by parent and apply the predicate per group). It is
  left out to keep R8 an exact syntactic rewrite.
- The Jaxen corpus has no `count(name)` or bare `[name]` at an element
  focus, so for R9 the corpus checks nothing beyond the scan path. The R9
  evidence is the kit's matrix, `tests/pushdown.rs`, the fixture matrix and
  the random-tree property.
- Cancellation is cooperative: a single slow navigator or extension call is
  not interrupted, and the deadline is read only on the check cadence.
- `ScanHooks` answers `child_by_key` whenever the data selects at most one
  entry, which is stronger than a schema-driven store (which declines unless
  the keys cover the list's key leaves). Both are valid under the contract.
