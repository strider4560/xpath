# Jaxen oracle provenance

Upstream: <https://github.com/jaxen-xpath/jaxen>

Pinned commit: `860cfd1899a1d662e4cb187ffd10fef2fbeb01bc`.

`xml/` is an unmodified copy of upstream `core/xml/`, including the catalog
`xml/test/tests.xml`, its stylesheet, all XML inputs, and the serialized variable
context fixture. `LICENSE.txt` is the upstream license. The harness consumes the
XML catalog directly; the Java integration tests are consulted for the rule that
nested assertions use the first selected node.

No oracle selectors or expected values have been rewritten. Jaxen extensions are
explicitly identified and implemented only inside `tests/jaxen.rs`; they do not
expand the crate's XPath 1.0 builtin function set. See `docs/oracle-report.md` for
exact accounting and data-model details.
