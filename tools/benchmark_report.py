#!/usr/bin/env python3
"""Archive Criterion samples and compare two captures (Python standard library)."""

import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess


def output(*command):
    return subprocess.check_output(command, text=True).strip()


def capture(args):
    benchmarks = {}
    for estimates_path in sorted(args.directory.rglob("estimates.json")):
        if estimates_path.parent.name != args.baseline:
            continue
        directory = estimates_path.parent
        benchmark = json.loads((directory / "benchmark.json").read_text())
        benchmarks[benchmark["full_id"]] = {
            "estimates": json.loads(estimates_path.read_text()),
            "sample": json.loads((directory / "sample.json").read_text()),
        }
    if not benchmarks:
        raise SystemExit(f"no samples for baseline {args.baseline!r}")
    rustc = os.environ.get("RUSTC") or shutil.which("rustc")
    if rustc is None:
        rustc = str(Path.home() / ".cargo/bin/rustc")
    result = {
        "label": args.label,
        "source_revision": args.revision or output("git", "rev-parse", "HEAD"),
        "captured_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(),
        "rustc": output(rustc, "-Vv"),
        "benchmark_command": args.command,
        "criterion_baseline": args.baseline,
        "benchmarks": benchmarks,
    }
    if Path("/proc/cpuinfo").exists():
        result["cpu"] = next(
            line.split(":", 1)[1].strip()
            for line in Path("/proc/cpuinfo").read_text().splitlines()
            if line.startswith("model name")
        )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Captured {len(benchmarks)} benchmarks in {args.output}")


def estimate(benchmark):
    # Criterion uses a regression slope for linear sampling. Fall back to the
    # mean when it chooses flat sampling for a slow workload.
    estimates = benchmark["estimates"]
    return estimates.get("slope") or estimates["mean"]


def compare(args):
    before = json.loads(args.before.read_text())["benchmarks"]
    after = json.loads(args.after.read_text())["benchmarks"]
    if before.keys() != after.keys():
        raise SystemExit("benchmark sets differ; refusing a partial comparison")
    print("| Workload | Before (µs) | After (µs) | Speedup |")
    print("|---|---:|---:|---:|")
    for name in before:
        a = estimate(before[name])["point_estimate"]
        b = estimate(after[name])["point_estimate"]
        print(f"| {name} | {a / 1000:.3f} | {b / 1000:.3f} | {a / b:.2f}× |")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    record = commands.add_parser("capture", help="archive one Criterion baseline")
    record.add_argument("directory", type=Path)
    record.add_argument("baseline", help="saved baseline name, or new for latest run")
    record.add_argument("output", type=Path)
    record.add_argument("--label", required=True)
    record.add_argument("--revision", help="commit of the measured source")
    record.add_argument("--command", required=True, help="exact benchmark invocation")
    record.set_defaults(function=capture)
    comparison = commands.add_parser("compare", help="print all timings as Markdown")
    comparison.add_argument("before", type=Path)
    comparison.add_argument("after", type=Path)
    comparison.set_defaults(function=compare)
    args = parser.parse_args()
    args.function(args)


if __name__ == "__main__":
    main()
