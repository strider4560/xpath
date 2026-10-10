# XPath 1.0 crate design

Build a reusable Rust dependency for a future Rust implementation of libyang. The evaluator must operate over application-owned trees and stable node handles, without requiring XML or a YANG object graph. XML is an optional adapter and the host for the Jaxen fixture suite.

## Design decisions

A hand-written lexer and recursive-descent parser compile expressions once into an immutable AST. Evaluation implements XPath 1.0 directly; CEL would introduce incompatible coercions and add another evaluator. All thirteen axes, node tests, predicates, union, arithmetic, comparisons, variables, namespaces and the complete core function library are in scope. Evaluation produces one of the four XPath values. Node sets are unique and in document order; reverse-axis predicates retain axis order until filtering finishes.

`XPath::compile` and `XPath::evaluate(&Context)` are the main API. `Navigator` supplies node kinds, expanded names, string values, parent links, fallible child/attribute/namespace iterators, stable document order and ID lookup. It can wrap a snapshot and K/V handles. Storage failures propagate as errors. The core has no dependencies; the optional `xml` feature uses roxmltree. Results and intermediate node sets are materialized. This version does not promise a query optimizer, bounded-memory streaming, or benchmarked performance.

`Context` supplies the context node, namespace and variable bindings, extension functions, and explicit evaluation limits. Extension functions receive current focus and initial node, allowing the future YANG library to implement current(), deref(), re-match(), identity functions and its own accessible-tree rules. These YANG functions are not part of XPath 1.0 itself. Unprefixed XPath element names select the null namespace by default. An explicit `with_default_element_namespace` opt-in supports the calling YANG module's namespace without changing attribute, namespace-node, function or variable resolution.

An owned XML tree adapter models root, elements, attributes, namespace nodes, text, comments and processing instructions. It preserves in-scope namespaces and merges adjacent text. XML DTD validation and external entity loading are outside the XML adapter. ID lookup is an embedding responsibility, with explicit registration in the XML adapter where needed. Expanded XML element depth is limited to 256 to protect the third-party parser's recursive tokenizer. The arena import and XPath navigation themselves use explicit stacks.

## Correctness evidence

Vendor the unmodified Jaxen core/xml fixtures, tests.xml and license at commit 860cfd1899a1d662e4cb187ffd10fef2fbeb01bc. The harness evaluates contexts, count assertions, valueOf assertions, expected exceptions and nested assertions. Account for every assertion: no silent skips and no changes to expected values. Jaxen extension-function cases are explicitly classified outside the XPath 1.0 core. Supplement the fixture suite with tests for spec edge cases, parser rejection, fallible custom navigators and extension contexts. Passing the available corpus is evidence, not a proof of exhaustive conformance.

Deliver source, Cargo.lock, documentation, examples, vendored fixtures, provenance, exact test accounting and a reproducible archive. Use Rust 1.85 as the verified minimum version. Do not publish to crates.io or a remote repository.
