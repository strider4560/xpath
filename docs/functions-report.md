# Values and core functions

Implemented `src/value.rs` and `src/functions.rs` against `INTERFACES.md`. Dispatch contains the 27 XPath 1.0 core functions; unknown names return `None`, recognized names validate arity before conversion, and navigation errors retain their original category/message.

Conversions select the earliest document-order node even when a caller supplies an unsorted vector. The shared selection helper validates every handle through `Navigator::validate_node` before comparing any handles and propagates validation errors through conversions and name functions. Numeric lexical parsing accepts only XPath decimal syntax and XML whitespace. String formatting covers IEEE special values, emits the exact decimal for finite integral binary64 values, and uses Rust's shortest round-tripping decimal display for fractional values. Rounding avoids the extra rounding error of `(x + 0.5).floor()` and preserves negative zero. String operations use Unicode scalar characters; normalization and ID splitting use XML's four whitespace characters. Translation uses the first mapping occurrence. ID results are unique and sorted. Language matching walks element ancestors and stops at the closest `xml:lang` declaration. Internal dispatch receives the evaluator’s remaining operation budget; checked subtraction meters each language parent traversal/visited attribute, each ID input-node read/lookup, and each sum node read.

## Verification

- Wrote 16 unit tests before implementation. After correcting one test's generic type annotation, the standalone Rust 1.85.1 harness ran all 16 and failed at the missing implementation stubs. All 16 passed after implementation.
- Reviewed upstream Jaxen `NumberTest`, `RoundTest`, `SubstringTest`, `NormalizeSpaceTest`, `TranslateFunctionTest`, `NamespaceURITest`, `LangTest`, `CeilingTest`, and `FloorTest`, plus the vendored XML fixture cases for substring, translate, and normalize-space. Tests include first-node conversion, forbidden exponent/plus/infinity syntax, negative-zero rounding, fractional/infinite/NaN substring positions, supplementary Unicode, duplicate translation keys, XML whitespace, name kinds, ID lookup, language inheritance, all 27 arity ranges, node-set type errors, and navigation error propagation.
- The initial full `cargo test` reached 23 passing library tests but encountered seven evaluator integration failures from the temporary evaluator stub; those have since been resolved.
- Added `conversions_and_names_validate_every_handle_before_document_order_comparison` before the validation fix. Its custom Navigator rejects a foreign handle and panics if comparison reaches one; the initial focused run reproduced the comparator panic. The fix passed the focused regression, including conversion/name propagation and a single invalid handle.
- An earlier full `cargo test` passed 24 library tests, seven core integration tests, and the unmodified Jaxen corpus test, then aborted at `deeply_nested_xml_import_uses_bounded_call_stack`. That adapter issue has since been resolved.
- Added operation-budget regressions on a 10,000-element chain with counted navigation calls and on ID/sum reads. Before the fix, the low-budget cases incorrectly returned success; after the fix, exhausted budgets return `ErrorKind::Limit` before the next charged navigation operation. Tests also cover zero budgets, attribute scans, successful exact-budget ID evaluation, and empty ID/sum inputs.
- Added an exact integral-decimal regression for positive/negative `1000000000000000128`, `2^80`, and `1e21`. The initial run produced the incorrect shortened decimal `1000000000000000100`; the integer-formatting branch passed afterward. All 20 focused tests pass in the standalone Rust harness.
- A full Cargo run immediately after changing internal dispatch was temporarily blocked at the root-owned evaluator’s old three-argument call. After the evaluator update, current `cargo test --all-features` exits 0 with 27 library tests, eight core tests, one Jaxen corpus test, nine XML tests, and zero doctests passing. Its output still prepends the prior E0061 diagnostic despite the source having the fourth argument and the completed run succeeding; this appears to be cached diagnostic replay.
- Rustfmt became available and formatted only the two owned Rust files.


## Limits of this check

These unit tests do not constitute a run of Jaxen's Java suite. Rust strings cannot express the malformed UTF-16 surrogate strings covered by some upstream Java tests. Builtins assume the Navigator's documented total document order, stable node identity, and tree parent invariants. IDs depend on the adapter's `element_by_id` policy; ordinary attributes named `id` are not automatically promoted.

The operation counter bounds the charged builtin navigation listed above. It is not a byte/allocation limit for strings, tokenization, concatenation, or adapter-internal work inside a single Navigator call; those require separate embedding policy.

Final root verification supersedes the intermediate logs above: a clean command
run passes all 47 all-feature tests, all 35 core-only tests, formatting and clippy.
No compiler diagnostics remain. See `CONFORMANCE.md`.
