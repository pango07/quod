#!/usr/bin/env python3
"""0e extras: why misses missed (first changed input, by top-level path),
lookup time and quod peak RSS over the series, replay wall, store sizes.

    extra.py HIST_ROWS TRAFFIC_ROWS
"""
import json, statistics as st, sys
from collections import Counter

STEPS = ["go build ./...", "go vet ./...", "go test ./..."]


def ev(r):
    e = r.get("run_event") or {}
    return e.get("fields", e)


def bucket(fm):
    if fm in ("-", None):
        return "(none)"
    if fm == ".git" or fm.startswith(".git/"):
        return ".git (VCS stamping)"
    if fm.startswith(".quod/gomod"):
        return "module cache (dependency bump)"
    if fm.endswith("_test.go") or "/testdata/" in fm:
        return "test file or testdata"
    if fm.endswith(".go") or "/" not in fm or fm.startswith(("api", "internal", "pkg", "git", "script", "cmd")):
        return "Go source file or a directory listing"
    return fm.split("/")[0] + "/..."


for label, path in zip(("history", "traffic"), sys.argv[1:]):
    rows = [json.loads(l) for l in open(path)]
    q = [r for r in rows if r.get("series") == "quod"]
    print(f"## {label}\n")
    print("Misses by first changed input:\n")
    for s in STEPS:
        c = Counter(bucket(ev(r).get("first_mismatch")) for r in q if r.get("step") == s and ev(r).get("outcome") == "miss")
        print(f"- `{s}`: " + ", ".join(f"{k} {v}" for k, v in c.most_common()))
    t = [r for r in q if r.get("step") == "go test ./..."]
    look = [ev(r).get("lookup_ms") or 0 for r in t]
    print(f"\ngo test lookup ms: run 2 {look[1]:.0f}, median {st.median(look):.0f}, last {look[-1]:.0f} "
          f"(candidates at the last run: {ev(t[-1]).get('candidates')})")
    if t and t[0].get("proc_hwm_kb"):
        rss = [round((r.get("proc_hwm_kb") or 0) / 1024) for r in t]
        print(f"go test quod peak RSS MB: first {rss[0]}, median {st.median(rss)}, last {rss[-1]}")
    reps = [r["wall"] for r in q if ev(r).get("outcome") == "replayed"]
    print(f"replays: {len(reps)}, wall median {st.median(reps):.2f} s (min {min(reps):.2f}, max {max(reps):.2f})")
    sizes = [r for r in rows if r.get("series") == "sizes"]
    if sizes:
        print("sizes after the last commit (MB): " + ", ".join(f"{k[:-6]} {v / 2**20:,.0f}" for k, v in sizes[-1].items() if k.endswith("_bytes")))
    done = [r for r in rows if r.get("step") == "done"]
    if done and done[-1].get("ledger_bytes"):
        print(f"ledger at the end: {done[-1]['ledger_bytes'] / 2**20:,.0f} MB")
    print()
