# Conformance and release verification

Verified on 2026-10-10 using `rustc 1.85.1 (4eb161250 2025-03-15)` and
`cargo 1.85.1`, Linux x86-64. The crate uses edition 2021 and forbids unsafe code.

## Oracle provenance and accounting

The repository vendors the unmodified `core/xml` directory and license from
[Jaxen](https://github.com/jaxen-xpath/jaxen) commit
`860cfd1899a1d662e4cb187ffd10fef2fbeb01bc`. Every vendored file was compared against
the fetched pinned source, and all 30 recorded SHA-256 hashes match. The manifest
is `vendor/jaxen/SHA256SUMS`; run `sha256sum -c SHA256SUMS` from that directory.

The XML catalog `tests.xml` is read directly by the Rust integration harness.
Neither expressions nor expected results are rewritten.

| Category | Count | Result |
|---|---:|---|
| Context selections | 62 | Pass |
| XPath 1.0 core assertion definitions | 277 | Pass |
| Extension-dependent assertion definitions | 19 | Pass through test-only callbacks |
| Total assertion definitions | 296 | Pass; none skipped |
| Expected exceptions (included above) | 9 | All raised |
| Nested assertions (included above) | 15 | Pass |

Six `<test>` elements omit `count`; they select the first node for their nested
assertions, exactly as the upstream Java harness does. No count expectation is
invented. The harness asserts exact catalog and execution totals, verifies every
extension whitelist entry, and fails on missing/empty contexts.

Test-only extensions are `document`, `evaluate`, `upper-case`, `lower-case`, and
`ends-with`. They are not XPath 1.0 builtins. `document` can only access documents
already loaded from the pinned fixture corpus; it performs no network loading.

The parser was also checked against all 358 catalog `select` attributes: 350
compile and eight malformed expressions produce syntax errors. The ninth expected
exception is an evaluation-time unbound namespace prefix.

## Release gates

| Check | Result |
|---|---|
| `cargo test --locked --all-features` | 47 Rust tests pass |
| Library unit tests | 27 pass |
| Custom-tree/evaluator tests | 8 pass |
| Embedding namespace test | 1 passes |
| Jaxen integration test | 1 passes, executing 296 definitions |
| XML adapter tests | 10 pass |
| `cargo test --locked --no-default-features` | 35 applicable tests pass |
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | Pass |
| `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features` | Pass |
| XML, custom-tree and YANG examples | All run successfully |
| `cargo package --allow-dirty --locked` | Packaged source compiles successfully |
| `cargo tree --no-default-features --edges normal` | No runtime dependencies |

The package check emits only Cargo's informational warning that the new local
crate has no repository/homepage/documentation URL. No registry publication was
performed.

An independent review compared 10,000 deterministic combinations of element
contexts, axes, predicates and value conversions against libxml2 2.14.6 with no
mismatches. The reproducible supplementary client and generator are retained in
`tools/differential`. That comparison is not the normative oracle: namespace
ordering can be implementation-defined, and adapter differences matter.

## Review findings resolved

- Exact integral binary64 values now render their integer decimal, avoiding
  Rust's shortened round-trip representation for values such as
  `1000000000000000128`.
- `lang`, `id` and `sum` share the evaluator's operation budget for their explicit
  navigation. Low-budget regressions verify early failure.
- Foreign node handles are validated before infallible kind/order operations.
- XML import uses an explicit stack; expanded XML nesting over 256 is rejected
  before entering the third-party recursive tokenizer. DTD PI decoys, including
  fake entity declarations and `]>`, cannot bypass the preflight guard.

Each finding has a regression that failed before its fix. The independent review
closed all three numbered findings; see `review-report.md` for evidence and scope.

## Scope and practical limits

The implementation follows the [XPath 1.0 Recommendation](https://www.w3.org/TR/1999/REC-xpath-19991116/).
Unicode names use the XML 1.0 Fifth Edition repertoire. All thirteen axes and
27 core functions are implemented. The XML adapter's DTD typing/defaults and
external entity resolution are explicitly outside its scope; ID registration is
available to the embedding. Full Jaxen Java unit tests were not ported or executed.
The corpus and differential runs provide evidence, not exhaustive proof.

Compilation limits, configurable evaluation limits, the optional 256-level XML
depth limit, and string/host-callback budgeting boundaries are documented in the
README. Intermediate node sets are materialized. No optimizer, index pushdown,
constant-memory claim, hot-path benchmark, or full YANG validator is included.

The YANG integration points are fallible storage iterators, stable node handles,
expanded names, an explicit module-default element namespace, variable bindings,
and extension functions with initial/current context. Accessible-tree rules and
YANG-specific semantics belong to the caller.
