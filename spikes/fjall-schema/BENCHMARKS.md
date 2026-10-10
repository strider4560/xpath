# Flat-schema benchmark baseline

Measured on 2026-10-10 with Rust 1.90.0, Fjall 3.1.10, Criterion 0.5.1,
Linux x86_64, and an Intel Xeon Platinum 8573C shared VM. The fixture has 29
compiled rows including its virtual root. Each workload uses 30 samples,
200 ms warmup, and a one-second measurement period. The repository also
contains [machine-readable estimates](benchmark-results.json).

Run from this directory:

```sh
cargo bench --locked --bench schema
```

The table reports Criterion estimates and 95% confidence intervals. These are
small, warm benchmarks over a newly imported, resident Fjall database. Import,
fsync, expression compilation, context setup, and file opening are excluded.
They do not predict cold disk performance, large-schema scaling, or throughput
under concurrent writes.

| Workload | Estimate | 95% interval |
|---|---:|---:|
| Read and decode one flat row | 1.19 µs | 1.18–1.20 µs |
| Decode all 29 rows with a preorder cursor | 52.9 µs | 46.0–61.4 µs |
| Open interface subtree, read root, range-skip descendants | 3.82 µs | 3.81–3.84 µs |
| Select `static-address` by schema path | 33.2 µs | 33.0–33.4 µs |
| Select `//*[@config='false']` (6 results) | 204 µs | 199–210 µs |
| Build keys/state/constraints selection plan | 790 µs | 736–851 µs |
| Iterate an already-built 14-row plan | 4.74 ns | 4.10–5.72 ns |

The final row measures only references passing through `black_box`; it does
not decode, format, visit data values, or perform useful per-node application
work. Its purpose is to distinguish the cost of walking a flat plan from
constructing it. The subtree-skip workload deliberately performs less work
than the full traversal, so those two timings are not a before/after speedup.

## Improvement examined

Metadata-axis enumeration originally constructed each attribute's string
value to determine whether it existed. Only `config` and `type` are optional.
The implementation now checks their `Option` fields and creates string values
only when requested. This keeps presence enumeration free of explicit string
construction in the adapter.

A saved baseline followed by a focused comparison produced:

| Workload | Before | After | Interpretation |
|---|---:|---:|---|
| Schema path (no metadata predicate) | 34.2 µs | 33.2 µs | Control; not attributable to this change |
| Metadata selection | 206 µs | 204 µs | Criterion: within noise threshold |
| Three-criterion plan | 757 µs | 790 µs | Criterion: no statistically detected change |

No reliable latency improvement is claimed. Allocations were not independently
counted, so these measurements also do not establish an allocation reduction
after optimization by the Rust compiler. Shared-machine variation is visible,
especially in whole-tree and plan-building samples.

## What to measure next

The implemented child index limits a child-axis scan to one parent. Preorder
subtree bounds allow range skips, and checked handles cache decoded rows for
infallible identity/order operations. These structural choices are already in
the baseline, not measured against an alternative implementation.

Broad XPath predicates still traverse nodes, JSON row decoding is repeated,
and each criterion evaluates separately. A representative corpus of larger
compiled schemas would support useful comparisons of compact row encoding,
a bounded snapshot-local row cache, shared criterion scans, or metadata/name
indexes. The latter require query-planning support; the generic XPath
Navigator alone does not push predicates into Fjall. Keep the independent
fixture expectations and the core Jaxen suite as correctness gates for any
such optimization.
