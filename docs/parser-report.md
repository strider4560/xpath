# XPath parser implementation report

Implemented only `src/ast.rs` and `src/parser.rs`, following the AST contract in `INTERFACES.md`. The parser is a hand-written UTF-8 lexer and recursive-descent parser for XPath 1.0 productions 1–39. It supports all thirteen axes, node tests, namespace wildcards, literals, decimal numbers, variables, function calls, filters, paths, abbreviations, unary minus, and every binary operator with XPath precedence and left associativity.

Operator words are recognized by grammatical context: `and`, `or`, `div`, and `mod` remain legal names and function names. QName components and `:*` must remain contiguous tokens. XML whitespace is exactly space, tab, CR, and LF. Unicode NCNames follow XML 1.0 Fifth Edition NameStartChar/NameChar ranges excluding colon. Strings have XPath 1.0's literal delimiter rules, without escapes or doubled-quote syntax. Syntax diagnostics retain original UTF-8 byte ranges; a missing token at EOF has a zero-width range.

Compilation has fixed protective limits, independent of evaluation limits: 1,048,576 source bytes, 65,536 tokens, 64 nested expression calls, and AST height 128. Paths and argument/predicate lists are processed iteratively. Unary-minus chains are processed iteratively and rejected before unbounded recursion. Left-associated binary chains are bounded by AST height, protecting later recursive evaluation, cloning, and destruction. Limit failures use `ErrorKind::Limit` rather than a syntax error. These limits intentionally reject sufficiently large otherwise-valid expressions and should be described in public documentation.

## Verification

Tests were written before the parser implementation. With the minimal parser stub, six of seven unit tests failed for missing behavior; the invalid-syntax test necessarily passed because the stub rejected every expression. After implementation all seven parser tests pass under Rust 1.85.1, covering every axis, node test, abbreviation, filters, operator contexts/precedence, Unicode names, numeric forms, variables/functions, syntax rejection, precise spans, and adversarial depth/work limits.

An independent temporary standalone test enumerated every `select` attribute in the pinned Jaxen `vendor/jaxen/xml/test/tests.xml`. It checked 358 expressions: 350 accepted expressions and all eight explicit malformed expressions rejected. The unbound-prefix expected-exception case compiles, correctly leaving namespace resolution to evaluation. The standalone run passed eight tests including this corpus check. The temporary wrappers are outside the repository and do not alter any fixture.

`cargo test --offline` initially could not start because the Jaxen manifest target had not yet been created. A later full-crate attempt compiled and passed all 23 library unit tests, including the seven parser tests. It then failed all seven evaluator integration tests because `eval.rs` was still the deliberate implementation stub: `existential_comparisons_and_scalar_coercions`, `external_variables_and_function_focus`, `limits_and_context_validation`, `path_focus_and_reverse_axes`, `short_circuit_and_errors`, `storage_errors_propagate`, and `union_sorting_dedup_and_step_predicates`. These are root-owned work in progress, not parser failures. Root should rerun the full suite after evaluator integration.

`rustfmt --edition 2021 src/ast.rs src/parser.rs` completed successfully. No AST contract changes, external dependencies, or fixture modifications were made.

## References and limitations

Reviewed the primary grammars at <https://www.w3.org/TR/1999/REC-xpath-19991116/> and <https://www.w3.org/TR/REC-xml/#NT-NameStartChar>. The XML Fifth Edition name repertoire is broader than the historical 1999 XML name repertoire and is the explicitly implemented compatibility choice. Function availability, namespace binding, variable binding, and runtime node-set type errors are intentionally evaluation concerns. The fixture corpus and focused tests provide conformance evidence, not proof of exhaustive parser correctness.

Final integration gate: all 47 all-feature tests now pass, including the completed
evaluator and full Jaxen harness. The temporary integration blockers recorded
above describe intermediate development only. See `CONFORMANCE.md`.
