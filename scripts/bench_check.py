#!/usr/bin/env python3
"""Check benchmark results against fixed budgets and, on pull requests, against the base branch.

    bench_check.py --budget bench/budget.txt --head head.txt [--base base.txt]

Each results file holds one or more runs of the bench example (lines like `sort   1055 ms`);
the fastest run per operation counts, which filters out most noise of shared CI machines.
Fails when an operation is over its budget, or more than MAX_SLOWDOWN slower than on the base
branch (and by more than NOISE_MS). Prints a Markdown table for the job summary.
"""
import argparse
import re
import sys

MAX_SLOWDOWN = 1.25
NOISE_MS = 30.0
LINE = re.compile(r"^(\S.*?)\s*?\s([\d.]+) ms")


def parse(path):
    best = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = LINE.match(line.rstrip())
            if m:
                name, ms = m.group(1).strip(), float(m.group(2))
                best[name] = min(ms, best.get(name, ms))
    return best


def parse_budget(path):
    budget = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if line:
                name, ms = line.rsplit(None, 1)
                budget[name.strip()] = float(ms)
    return budget


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--budget", required=True)
    ap.add_argument("--head", required=True)
    ap.add_argument("--base")
    a = ap.parse_args()
    budget, head = parse_budget(a.budget), parse(a.head)
    base = parse(a.base) if a.base else {}
    failures = []
    print("| operation | this branch | base | change | budget | |")
    print("|---|---:|---:|---:|---:|---|")
    for name in sorted(set(head) | set(budget), key=lambda n: list(budget).index(n) if n in budget else 99):
        h = head.get(name)
        if h is None:
            failures.append(f"{name}: missing from the benchmark output")
            print(f"| {name} | missing | | | {budget.get(name, '')} | :x: |")
            continue
        b = base.get(name)
        lim = budget.get(name)
        bad = []
        if lim is None:
            bad.append("no budget: add it to bench/budget.txt")
        elif h > lim:
            bad.append(f"over budget ({h:.0f} > {lim:.0f} ms)")
        if b is not None and h > b * MAX_SLOWDOWN and h - b > NOISE_MS:
            bad.append(f"{(h / b - 1) * 100:.0f}% slower than base ({h:.0f} vs {b:.0f} ms)")
        change = f"{(h / b - 1) * 100:+.0f}%" if b else ""
        print(f"| {name} | {h:.1f} ms | {'' if b is None else f'{b:.1f} ms'} | {change} | {'' if lim is None else f'{lim:.0f} ms'} | {':x: ' + '; '.join(bad) if bad else ':white_check_mark:'} |")
        failures += [f"{name}: {x}" for x in bad]
    if failures:
        print("\n**Performance check failed:**\n" + "\n".join(f"- {f}" for f in failures))
        sys.exit(1)


if __name__ == "__main__":
    main()
