# Independent review — XPath 1.0 crate

Reviewed 2026-10-10 against the design spec and XPath 1.0 Recommendation. The implementation was actively changing during review; findings below describe the observed revisions, and should be closed with regression tests before final delivery. No implementation was modified by this review.

## Actionable findings

### R1 — P2, CLOSED: integral binary64 string conversion loses exact integer digits

Location: `src/value.rs`, `number_to_string`.

`string(1000000000000000128)` returns `"1000000000000000100"`. The input integer 1000000000000000128 is exactly representable as binary64. Rust Display emits a shortest round-tripping representation, which is not the exact decimal representation of that integer.

XPath 1.0 section 4.2 specifies decimal representation of integral numbers; its rule permitting only the digits necessary to distinguish binary64 values is in the separate noninteger branch. This affects string(), concat(), comparisons that coerce numbers to strings, and callers of Value::to_string.

Reproduce against any valid context:

```rust
let doc = xpath1::xml::Document::parse("<r/>").unwrap();
let ctx = xpath1::Context::new(&doc, doc.root());
let value = xpath1::XPath::compile("string(1000000000000000128)")
    .unwrap().evaluate(&ctx).unwrap();
assert_eq!(value, xpath1::Value::String("1000000000000000128".into()));
```

Observed assertion failure: actual `1000000000000000100`. Add an integral finite formatting branch (fixed decimal precision zero), retaining existing handling for special values and nonintegers. Test both signs, an exactly represented integer beyond 2^53, and a very large finite integer.

Normative source: https://www.w3.org/TR/1999/REC-xpath-19991116/#function-string

### R2 — P1, CLOSED: lang() parent/attribute traversal bypasses the operation budget

Location: `src/functions.rs`, `language_matches`, and core-function dispatch in `src/eval.rs`.

A valid 10,000-deep application tree with no xml:lang attributes evaluates `lang('en')` successfully under `Limits { max_operations: 2, max_depth: 128 }`, making 10,001 parent() calls. Only the call AST and literal AST consume budget. This contradicts Limits::max_operations documentation, which includes visited parent links, and allows hostile queries to do arbitrarily large tree traversal despite the host's limit. Attribute iteration is likewise unmetered.

Reproduction: implement a chain navigator whose integer node n has parent n-1, node 0 is the root, all attributes iterators are empty, and count parent calls using Cell<usize>. Set context to node 10000, maximum operations to 2, and evaluate `lang('en')`. Observed result: `Ok(Boolean(false))`, parent calls: `10001`. Expected: `ErrorKind::Limit` before traversal proceeds beyond the available work allowance.

Pass the remaining evaluation budget into core functions and charge per parent/attribute visit. Review id() and sum() traversal for the same accounting policy. Application callback internals can remain explicitly outside the evaluator's budget.

The integrating agent was notified early and is already implementing this fix; this entry records the independently confirmed defect rather than requesting duplicate work.

### R3 — P1, CLOSED: DTD processing-instruction text bypasses expanded XML depth guard

Location: `src/xml.rs`, `collect_entities` (and the related `declaration_end` scanner).

The preflight treats fake `<!ENTITY ...>` text inside a legal DTD processing instruction as a real declaration. Its first-definition-wins map then ignores the actual later declaration. The guard therefore measures empty text while roxmltree expands the real deep markup. This defeats the documented maximum of 256 expanded element levels and leaves the recursive tokenizer stack-overflow risk reachable with larger input.

Safe reproduction (only 257 expanded levels; no giant crash attempted):

```rust
let markup = "<a>".repeat(256) + &"</a>".repeat(256);
let source = format!(
    "<!DOCTYPE r [<?pi <!ENTITY deep ''> ?><!ENTITY deep '{markup}'>]><r>&deep;</r>"
);
assert_eq!(xpath1::xml::Document::parse(&source).unwrap_err().kind,
           xpath1::ErrorKind::Limit);
```

Observed: parsing succeeds and `count(//*)` is 257. Removing the PI correctly returns `ErrorKind::Limit`. Both DTD scanning routines must skip complete processing instructions before interpreting quotes, brackets, or entity declarations in their content. Add PI decoy regressions to the preflight tests, including PI content containing `]>`.

## Scoped rereview and closure

R1 is closed: exact integral formatting uses fixed decimal precision zero, and the targeted regression passes for positive/negative exact integers and large values. The original independent client now returns `1000000000000000128`.

R2 is closed: core dispatch receives the evaluator's remaining operation budget; lang charges parent and attribute visits, and id/sum have explicit traversal charges. The targeted parent/attribute budget regression passes. The original 10,000-deep client now returns `ErrorKind::Limit` before any parent call at a budget of two.

The optional default element namespace applies only to unprefixed element-principal-axis name tests. Its focused core regression passes. A separate XML client also confirmed attributes, namespace nodes, unprefixed variables, extension functions, and explicit prefixes retain their intended namespaces.

R3 is closed after a second scoped rereview: `declaration_end` skips complete processing instructions before interpreting quotes and brackets, and `collect_entities` scans only the internal subset while skipping complete PIs, comments, other declarations and quoted contents. The focused `dtd_processing_instructions_cannot_shadow_depth_guard_entities` test passes: 257 expanded levels are rejected with plain, fake-ENTITY PI, bracket/terminator PI, and comment prefixes; the 256-level PI-decoy control succeeds. The XML import itself uses an explicit stack.

Scoped rereview did not repeat the broad differential suite. The integrating agent is responsible for the final full delivery gates.

## Validation and review coverage

- Read the design spec, parser, AST/evaluator, value conversions, all core functions, navigator/context contracts, XML adapter, and core tests.
- Reproduced both findings using a separate temporary Cargo client depending on the crate by path; no implementation edits.
- Ran cargo test --all-features: 24 unit tests, 7 core integration tests, and the Jaxen harness passed at the observed revision. XML tests reached the deep-import regression and aborted with stack overflow. The XML worker already owns that known issue, so it is not duplicated as a new finding.
- Ran 10,000 deterministic generated combinations of element-context forward/reverse axes, node tests, sequential predicates and count/name/string conversions against libxml2 2.14.6 via lxml: no mismatches. A preceding exploratory 3,000 cases including namespace/attribute contexts exposed only implementation-defined namespace order or libxml2 attribute/namespace-following behavior, which were not filed as defects in this crate.
- Inspected grammar precedence, contextual keywords, QName lexing, node tests, abbreviation handling, recursion limits, reverse-axis predicate focus, existential comparisons, NaN/signed zero, Unicode character operations, namespace node identity, storage error propagation and extension context focus. No additional actionable counterexamples were established.

The retained differential client, fixture and deterministic generator are in `tools/differential/`; see `tools/differential/README.md` for requirements and commands.

## Verdict

**Specification:** R1/R2/R3 are closed by inspected fixes and successful scoped regressions. Reviewed core semantics, namespace opt-in and XML depth guard match the design spec.

**Reusable dependency quality:** suitable for an application-owned libyang tree adapter. **No remaining blockers found in this review; approve subject to the integrating agent's final full test and packaging gates.** No optimizer is required by the specified scope.
