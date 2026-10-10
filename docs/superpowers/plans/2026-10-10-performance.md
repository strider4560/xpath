# XPath performance implementation plan

**Goal:** Add repeatable benchmarks and improve measured evaluator bottlenecks for an embedded Rust/YANG caller.

**Architecture:** Benchmark compilation separately from evaluation of reusable expressions and contexts. Use both an application-owned arena and the optional XML adapter. Preserve the public API, XPath 1.0 semantics, fallible navigation, limits, and the pinned Jaxen expectations.

**Tech stack:** Rust 1.85, Criterion 0.5.1 as a development dependency, Python standard library for report extraction.

## Tasks

- [x] Add `benches/xpath.rs` and shared synthetic tree fixtures. Cover parsing, schema-like traversal, list selection at multiple sizes, true/false `when`, reverse axes, union, node-set comparison, and optional XML parsing/evaluation. Assert expected answers before timing; keep setup outside cached-evaluation loops. Save baseline raw samples and compiler/CPU metadata before changing the evaluator.
- [x] Use timing and navigator-call counts to choose optimizations. Add semantic regression cases for affected behavior, then change only the measured paths. Re-run identical release benchmarks, retain before/after evidence, and check small and large workloads for regressions.
- [x] Run all-feature and dependency-free correctness suites, Jaxen oracle, formatting, clippy, and a fresh independent code review. Document commands, setup, improvements, limitations and next bottlenecks. Prepare updated source archive.

## Review focus

- Reverse-axis position and `last()` remain correct before document-order normalization.
- External node sets remain validated, sorted and deduplicated.
- Node-set equality and inequality are independent existential comparisons, including empty sets.
- Fallible iterators and string access propagate errors; limits still bound actual evaluator work.
- Benchmarks consume results, exclude fixture setup, and keep equivalent inputs, compiler flags and sampling settings between revisions.

## Execution record

- Existing isolated local branch `implementation`, base `be5aec4`; no unrelated edits.
- The user authorized implementation and performance work. Proceed inline without introducing an additional approval gate.
- Use a pinned, established Criterion release compatible with Rust 1.85; runtime dependencies remain unchanged.
- Benchmark baseline `7cfe64e`: 35 smoke-tested workloads, 40 samples per case,
  immutable release executable retained and measured twice on CPU 2.
- Regression first failed with 16,512 string reads for two 128-node sets. The
  optimized evaluator `c1e7993` passes that regression and the full oracle.
- Changes: in-place canonicalization and filtering, consume owned vectors,
  remove redundant normalization, merge unions, and linear string-read
  equality/inequality. Initial union sorting increased order-call counts;
  linear merge resolved that before final measurement.
- Final verification: 52 all-feature tests, 40 no-default-feature tests,
  296 Jaxen assertions, 10,000 libxml2 comparisons with zero mismatches, rustfmt,
  clippy with warnings denied, documentation with warnings denied, no runtime
  dependencies without XML, and a successful YANG example.
- Independent static review: no blocking findings. Measurement scope and noise
  cautions are documented in PERFORMANCE.md. Raw captures, call counts and a
  baseline restoration patch make the performance claims reviewable.
