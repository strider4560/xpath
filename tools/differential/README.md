# Optional differential verification

This standalone client and script preserve the independent review's 10,000-case
deterministic comparison against Python lxml/libxml2. They add no runtime
dependency to the XPath crate and are not part of its normal test suite.

Requirements: Rust/Cargo and Python 3 with `lxml` installed. The review used
libxml2 2.14.6; the script prints the version used by each run.

From the crate root:

```sh
cargo build --manifest-path tools/differential/Cargo.toml --target-dir target/differential
python3 tools/differential/check.py target/differential/debug/xpath1-differential-client
```

The generator uses Python `random.Random(7)` and a fixed fixture, exercising
eleven axes, several node tests, sequential predicates, and count/name/string
conversion. Initial contexts are elements. The script exits unsuccessfully for
any mismatch, client failure, or missing output row. No generated corpus or
binaries need to be retained.

This is supplementary evidence, not exhaustive conformance testing. Attribute
and namespace axes are intentionally absent from this retained comparison:
namespace order is implementation-defined, and libxml2's following-axis behavior
from attribute/namespace contexts is unsuitable as a normative oracle. Dedicated
crate tests and the XPath specification cover those cases.
