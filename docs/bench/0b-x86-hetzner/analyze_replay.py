#!/usr/bin/env python3
"""Summarize /work/rs/rows.jsonl (replay speed). Round 0 dropped."""
import json, statistics as st, sys
from collections import defaultdict

rows = [json.loads(l) for l in open(sys.argv[1])]
STEPS = ["go build ./...", "go vet ./...", "go test ./..."]
v = defaultdict(list)       # (series, step) -> walls
job = defaultdict(lambda: defaultdict(float))
outc = defaultdict(list)
for r in rows:
    if r.get("i", -1) < 1 or r["series"] in ("meta", "sizes"):
        continue
    s = r["series"]
    if s == "quod":
        tag = r["log"].rsplit("-quod", 1)[1]
        s = "quod-nowarm" if tag.startswith("-nowarm") else "quod"
        e = r.get("run_event") or {}
        if r["step"] in STEPS:
            outc[s].append((r["i"], r["step"], e.get("outcome"), e.get("hit"), r["exit"]))
    v[(s, r["step"])].append(r["wall"])
    job[s][r["i"]] += r["wall"]
series = ["warm", "warm-ws", "cold", "quod", "quod-nowarm"]
print("| Step | " + " | ".join(series) + " |")
print("| --- |" + " --- |" * len(series))
for step in ["checkout", "go mod download"] + STEPS:
    cells = []
    for s in series:
        w = v.get((s, step))
        cells.append(f"{st.median(w):.2f}s (n={len(w)}, {min(w):.2f}-{max(w):.2f})" if w else "-")
    print(f"| {step} | " + " | ".join(cells) + " |")
cells = []
for s in series:
    w = list(job[s].values())
    cells.append(f"{st.median(w):.2f}s" if w else "-")
print("| **job (all rows)** | " + " | ".join(cells) + " |")
for s in ("quod", "quod-nowarm"):
    print(s, "outcomes:", sorted(set((o, h, x) for _, _, o, h, x in outc[s])))
