# XPath performance

Measured 2026-10-10. The crate now includes 35 Criterion workloads, plus an
untimed navigator-call diagnostic. The public API and runtime dependencies are
unchanged. The application-owned tree benchmarks also run without the XML feature.

## Measured results

These are synthetic in-memory workloads on one Linux x86-64 VM, not measurements
of libyang or an application's persistent storage. Times are per operation,
including destruction of its result, in microseconds; lower is better.

| Workload | Before (µs) | After (µs) | Speedup |
|---|---:|---:|---:|
| Select one schema-shaped declaration | 0.400 | 0.237 | 1.69× |
| Select all 1,000 interface instances | 61.957 | 32.902 | 1.88× |
| Select by list key among 1,000 instances | 539.429 | 416.930 | 1.29× |
| Union of overlapping selections, 1,000 instances | 486.948 | 337.897 | 1.44× |
| True `when` expression | 0.956 | 0.627 | 1.53× |
| False `when` expression, short circuit | 0.458 | 0.310 | 1.48× |
| Evaluate `when` at 1,000 prepared contexts | 718.233 | 473.438 | 1.52× |
| Equality of disjoint 512-node sets | 8,459.626 | 137.920 | 61.34× |
| Inequality of uniform 512-node sets | 8,624.549 | 97.504 | 88.45× |
| XML adapter: list-key selection, 1,000 instances | 681.627 | 580.979 | 1.17× |

The complete [35-row timing table](benchmarks/2026-10-10/timings.md) includes small
inputs, descendant/reverse axes, late equality matches, compilation, scalar
evaluation and XML parsing. Compilation and XML parsing were not optimized;
their small timing changes are controls, not claimed improvements.

The large comparison gains apply to the deliberately difficult cases that used
to exhaust the Cartesian product. Early-match equality and early-success
inequality already short-circuit; no comparable speedup is claimed for those
cases. Their two-read short circuit and fallible-access behavior are tested,
but they are not separate timing workloads in this suite.

## What changed and why

1. **Canonical node sets:** sort/deduplicate in place instead of allocating a
   hash set and cloning every node handle. Skip work for empty/singleton sets.
   All expression results remain sorted and unique; external variables and
   function results are validated and normalized when entering the evaluator.
2. **Buffer reuse:** consume owned node sets instead of cloning their vectors.
   Compact node tests and predicates in place. Predicate position/size and
   reverse-axis order remain intact. Completed paths and filters no longer
   sort already-canonical results a second time.
3. **Union:** merge two canonical operands in linear time. An intermediate
   version sorted their concatenation; call counts exposed its extra ordering
   comparisons, so it was replaced before the final measurement.
4. **Existential comparisons:** evaluate the first comparison row with the
   existing early-exit order. For equality, index right-hand strings if another
   left-hand node could require them. For inequality, if the first row did not
   succeed, every right-hand value equals the first left value, so subsequent
   left values need only be compared with that representative. Both algorithms
   read each node's string at most once per operand. Equality and inequality
   remain independent existential operations; neither is defined by negating
   the other.

The separate diagnostic gives deterministic evidence for the largest gains:

| Diagnostic | Before | After |
|---|---:|---:|
| String reads: disjoint equality, 512 × 512 | 262,656 | 1,024 |
| String reads: uniform inequality, 512 × 512 | 262,656 | 1,024 |
| Order comparisons: select all 1,000 entries | 1,998 | 999 |
| Order comparisons: overlapping union, 1,000 entries | 9,996 | 7,999 |

Equality now takes an expected linear number of hash operations, plus string
processing, and keeps a temporary index of distinct right-hand strings. That
trades memory for eliminating repeated reads/comparisons; inequality needs no
such index. There is no cross-evaluation cache. Node sets still materialize, and
in-place filters can retain vector capacity. These changes do not establish a
constant-memory bound. Navigation and string reads continue to consume the
evaluation work budget; string byte lengths and callback internals remain outside
that budget, as documented in the README.

## Workloads and measurement setup

- Criterion 0.5.1, Rust 1.85.1 / LLVM 19.1.7, default optimized Cargo bench
  profile; no target-specific CPU flags or LTO overrides.
- Intel Xeon Platinum 8573C virtual machine, Linux x86-64. Both executables were
  pinned to logical CPU 2 with `taskset`. Builds and test runs were kept out of
  the timing windows. This VM is not an isolated bare-metal performance lab.
- Forty samples per case, 300 ms warmup and one-second target measurement.
  Tables use Criterion's regression-slope point estimate (mean for flat
  sampling), with before/after ratios. Raw samples and 95% confidence intervals
  are retained. Ratios themselves are not confidence intervals.
- Fixtures, expressions, namespaces, variable bindings and contexts are prepared
  before evaluation timing. Compilation and XML parsing have separate cases.
  Every workload checks its expected answer outside the timing loop; inputs and
  returned values pass through optimization barriers/ Criterion's timing loop.
- The arena adapter stores integer handles, namespaces and typed-leaf string
  values. The schema-shaped case models the selection topology of one list
  declaration, not a schema compiler. Data cases have 10, 100 or 1,000 interface
  instances with name/enabled/kind/mtu children. List-key selection binds `$wanted`
  to the middle entry. The `when` expression is
  `../enabled = 'true' and ../kind = 'ethernet'`; this measures XPath evaluation,
  not the caller's construction of a YANG accessible tree or dummy node.
- Comparison cases use two sets of 16, 128 or 512 nodes, with disjoint values,
  a match at the final pair, or all strings equal for `!=`. The XML adapter uses
  an equivalent 1,000-interface document, measured separately.

The baseline executable was retained unchanged and re-run after the first
optimization pass. Most baseline timings repeated within about 5%; the group
of 100-entry cases varied by roughly 18–26%. The tables consistently use the
original baseline, not a per-case choice of the slower run. The repeat capture
is included to expose that noise. Large improvements and reduced call counts
are clear; small percentage differences should be re-measured on the target
system. No statistically calibrated CI failure threshold is imposed.

## Run and compare

From the crate root:

```sh
cargo bench --locked --bench xpath
cargo bench --locked --no-default-features --bench xpath
cargo bench --locked --bench xpath -- --test
cargo bench --locked --bench xpath -- --counts
```

Criterion accepts a workload filter and sampling overrides, for example:

```sh
cargo bench --locked --bench xpath -- 'data/key' --measurement-time 5
cargo bench --locked --bench xpath -- --save-baseline before
# Change/build the evaluator, preserving the same workloads and configuration.
cargo bench --locked --bench xpath -- --baseline before
```

Record and compare results using only Python's standard library:

```sh
python3 tools/benchmark_report.py capture target/criterion before before.json \
  --label before --revision MEASURED_COMMIT \
  --command 'cargo bench --locked --bench xpath -- --save-baseline before'
python3 tools/benchmark_report.py capture target/criterion new after.json \
  --label after --revision MEASURED_COMMIT \
  --command 'cargo bench --locked --bench xpath -- --baseline before'
python3 tools/benchmark_report.py compare before.json after.json
```

Pass the revision of the source actually used to build the measured binary.
The capture tool's default `HEAD` is only correct for a binary built from that
commit without source edits. Use fresh baseline names/output directories for
changed workload sets: Criterion retains older results in its output directory.
On Linux, prefix both benchmark invocations with `taskset -c <available CPU>` to
pin them identically; CPU 2 is a measurement detail, not a portable requirement.

For this report, the baseline was built from `7cfe64e`, and the final evaluator
from `c1e7993`. Later report-only edits do not change the benchmark or evaluator.
The source archive includes [restore-baseline.patch](benchmarks/2026-10-10/restore-baseline.patch)
so the baseline can also be recreated without those Git commits: make a separate
copy of the crate, apply that patch with `git apply`, and run the same benchmarks
there. The patch intentionally restores the old repeated-read behavior, so the
new linear-read regression will fail on that benchmarking copy. Do not apply it
to the crate used by your application.

## Evidence and remaining opportunities

The [measurement directory](benchmarks/2026-10-10/) retains `before.json`,
`before-repeat.json`, `after.json`, both call-count CSVs, the measured executables'
SHA-256 hashes, the baseline source patch, and the full timing table. JSON captures
include the compiler, CPU, platform, revision, invocation, per-sample iterations
and times, estimates and confidence intervals. Executables themselves are not
distributed.

All 52 Rust tests pass, including the unchanged Jaxen harness's 296 assertion
definitions. The dependency-free build passes 40 tests. The new comparison
regression failed before optimization with 16,512 reads for two 128-node sets,
and passes after it. All pairs of 31 small string sets are also checked against
the direct existential definition of both operators. Reverse focus, external
sets, reached errors and low budgets have regression coverage. A fresh static
[independent review](performance-review.md) found no blocking issues.
The retained 10,000-case differential generator was re-run against libxml2 2.14.6
with zero mismatches after the changes. Formatting, clippy and documentation
checks pass with warnings denied where applicable.

List-key selection still scans: the 1,000-entry case makes 5,001 name calls and
1,000 string calls. A future adapter API for borrowed/interned names or indexed
child lookup is worth measuring with the actual YANG store. Descendant queries
still traverse and materialize intermediate sets, and relational node-set
comparisons still have a Cartesian-product comparison loop. Those remain
opportunities; this pass does not claim to optimize them or measure database I/O.

## Embedding hooks (0.2.0, D21)

Version 0.2.0 adds the navigator hooks that the list-key paragraph above asks
for: `name_matches` (R3, name tests without an owned name per candidate),
`children_named` (R6, `parent/leaf` without loading every child) and
`child_by_key` (R2, `list[key = e]` as one point lookup instead of a scan), plus
reusable contexts (R5: `Context::set_node` and a shared `FunctionTable`). Their
defaults keep the measurements above valid for adapters that do not implement
them; the bundled XML adapter does not. Their benefit depends on the adapter and
is to be measured with the actual YANG store, not in this suite. The tests in
`tests/embedding.rs` assert their call counts (one keyed lookup and no list
enumeration for `a/b[c = 'x']`; no `children()` call for `a/b/d` with
`children_named`).

**R4 (borrowed string values) is deferred:** `string_value()` returns
`String`; a `Cow<str>` or caller-supplied buffer would let leaf values come
straight from the row.

