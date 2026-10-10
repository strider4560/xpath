#!/usr/bin/env python3
"""Reproduce the independent review's deterministic 10,000-case comparison."""

import argparse
from pathlib import Path
import random
import subprocess
import sys

from lxml import etree


def expressions():
    # Keep the seed, choice ordering and random calls stable to preserve the corpus.
    rng = random.Random(7)
    axes = (
        "self child parent descendant ancestor following preceding "
        "following-sibling preceding-sibling ancestor-or-self descendant-or-self"
    ).split()
    predicates = [
        "", "[1]", "[last()]", "[position()>1]", "[2][1]",
        '[name()="b"]', "[string(.) > 4]",
    ]
    for _ in range(10_000):
        path = rng.choice(["/r", "/r/a[1]", "/r/a[2]", "/r/a[1]/b[2]/c"])
        for _ in range(rng.randint(1, 4)):
            path += (
                "/" + rng.choice(axes) + "::"
                + rng.choice(["*", "node()", "text()", "b", "a"])
                + rng.choice(predicates)
            )
        yield rng.choice(["count", "name", "string"]) + "(" + path + ")"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("client", type=Path, help="compiled Rust client executable")
    args = parser.parse_args()
    root = etree.parse(str(Path(__file__).with_name("fixture.xml")))
    corpus = list(expressions())
    process = subprocess.run(
        [str(args.client.resolve())],
        input="\n".join(corpus) + "\n",
        text=True,
        capture_output=True,
        check=True,
    )
    actual = process.stdout.splitlines()
    if len(actual) != len(corpus):
        raise RuntimeError(f"client returned {len(actual)} rows for {len(corpus)} cases")
    differences = []
    for expression, observed in zip(corpus, actual):
        expected = root.xpath("string(" + expression + ")")
        if observed != expected:
            differences.append((expression, expected, observed))
    print(f"libxml2 version: {'.'.join(map(str, etree.LIBXML_VERSION))}")
    print(f"evaluated {len(corpus)} cases; differences: {len(differences)}")
    for expression, expected, observed in differences[:20]:
        print(f"{expression}\n  expected: {expected!r}\n  observed: {observed!r}")
    return 1 if differences else 0


if __name__ == "__main__":
    sys.exit(main())
