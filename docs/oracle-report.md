# XML adapter and Jaxen oracle report

The pinned, unchanged `vendor/jaxen/xml/test/tests.xml` contains 34 document
groups, 62 contexts, and 296 assertion definitions: 143 `test` and 153 `valueOf`.
Of these, 277 exercise XPath 1.0 core behavior and 19 exercise Jaxen extensions.
There are nine expected exceptions and 15 nested assertions. Every context must
select at least one node. The harness counts executions and requires exactly
296, including all 15 nested assertions; no case is ignored or silently skipped.
The 296 definitions comprise 128 count expectations, 153 string expectations,
nine expected exceptions, and six selection containers without a count.

Each context is selected against its specified fixture document. Namespace
bindings come from the in-scope declarations on the context or assertion, and
variable values come from the catalog's `var:` attributes. Count assertions
require a node set. Six `test` containers in `moreover.xml` omit `count`; as in
upstream Java, these select the first node for their nested `valueOf` without
asserting an invented count. `valueOf` uses XPath string conversion. Expected exceptions
accept a compile or evaluation error. As in upstream `XPathTestBase`, nested
assertions are evaluated against the first selected node, with focus reset to
position/size 1/1. Each failed assertion includes fixture path, catalog line and
expression.

The extension whitelist enumerates 17 exact direct selectors. Its 19 assertion
definitions consist of six `evaluate()` assertions, seven string extension
assertions (`upper-case`, `lower-case`, `ends-with`), four direct `document()`
assertions and two nested assertions whose context is supplied by `document()`.
All run through harness-only `Context::with_function` hooks. A composite
navigator routes document handles among already parsed fixture documents;
`document()` cannot load arbitrary files or use a network. `evaluate()` retains
the current node, position, size and catalog namespaces. Case-conversion locale
arguments are accepted for the catalog's ASCII servlet name; this is not a claim
of full locale-sensitive Jaxen extension conformance. These hooks are not
production builtins.

The XML adapter owns its arena and exposes seven node kinds. Namespace nodes
include every nonempty in-scope binding and the implicit `xml` binding, and each
element owns separate namespace identities. Namespace and attribute nodes have
element parents and are not children. The root retains document comments and
processing instructions. Element/root string values concatenate descendant
text only. Adjacent text, CDATA and internal entity expansion coalesce.
Expanded names use namespace URIs while lexical qualified names retain the
original source prefix, including alias prefixes bound to the same URI.
Document IDs form part of each handle; fallible navigation rejects handles from
another document. Infallible `kind` and ordering operations require valid handles
and panic on an ownership violation. `validate_node` checks ownership before
evaluation; foreign context, variable and extension-returned handles produce
navigation errors through the public evaluation API.

Internal DTD declarations are allowed for entity expansion. The parser does not
fetch external DTDs or resolve external entities; a referenced external entity
is rejected. DTD validation and automatic ID typing are outside this adapter.
`Document::register_id` explicitly indexes application-known element IDs; an
ordinary `id` attribute is not automatically an ID. The corpus contains no
`id()` assertion and does not require inferred DTD ID typing.

XML has a documented limit of 256 expanded element levels. The underlying
roxmltree 0.20 tokenizer uses recursive calls for nesting, so an iterative
preflight follows internal entity replacements and counts their combined depth
with surrounding document elements before invoking it. This preflight also
enforces roxmltree's entity-recursion and expansion bounds. Comments, CDATA and
processing-instruction data do not count as markup. Arena import and descendant
string-value traversal use explicit stacks. This depth limit is separate from
the evaluator's expression and operation limits; it is not a general memory
quota for large XML documents.

Validation ledger:

- Tests were written before adapter implementation; the first Cargo attempt
  was blocked by incomplete project assembly (missing example/test targets).
- With the crate assembled and evaluator stub still present, the complete
  harness failed all 62 contexts with `evaluator not yet implemented`, providing
  its initial red baseline.
- Ten adapter integration tests pass, covering node kinds, lexical names,
  coalescing, namespaces, ordering, ID registration, foreign handles, internal
  entities, entity-markup prefix preservation, rejection of referenced external
  entities, depth-limit handling, layered entities and Unicode names.
- A 10,000-level XML regression exposed stack overflow first in recursive arena
  import and then in the underlying tokenizer. Iterative import plus the depth
  preflight resolve it: 10,000 levels return a limit error, 256 levels parse, and
  layered entities at 320 levels fail while 80 levels parse.
- Review found a DTD preflight bypass: fake entity declarations in processing
  instruction data could shadow real declarations, and `]>` in PI data could
  terminate the perceived internal subset early. A safe 257-level regression
  failed before the fix. Both scanner layers now skip complete processing
  instructions; entity collection also skips complete other declarations,
  quoted contents and the external identifier. The 257-level controls return
  limit errors while the corresponding 256-level input parses.
- `cargo test --test xml --test jaxen -- --nocapture` passes all ten XML tests
  and the complete oracle: 62 contexts, 277 core executions, 19 extension
  executions, nine expected exceptions, 15 nested executions, zero failures.
- `diff -qr vendor/jaxen/xml ../jaxen-upstream/core/xml` produces no differences
  against the pinned upstream checkout.

The available corpus is substantial regression evidence, not exhaustive proof
of XPath 1.0 conformance. Core behaviors absent from this catalog require the
crate's supplemental tests.
