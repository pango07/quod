#!/usr/bin/env python3
"""Before/after of the 0b history: rows.jsonl of main (before) and of this branch (after).
Commit 1 (index 0) is the warm-up, excluded as in the 0b report. Job = step A + 3 steps
(+ checkout for plain), as analyze_history.py counts it."""
import json, statistics as st, sys
from collections import defaultdict

STEPS = ["go build ./...", "go vet ./...", "go test ./..."]


def load(p):
    by = defaultdict(dict)
    for l in open(p):
        r = json.loads(l)
        if r["series"] in ("warm", "quod"):
            by[(r["series"], r["i"])][r["step"]] = r
    return by


def job(by, s, i):
    d = by.get((s, i))
    if not d or not all(x in d for x in STEPS):
        return None
    return sum(d[x]["wall"] for x in STEPS) + d.get("go mod download", {}).get("wall", 0) + d.get("checkout", {}).get("wall", 0)


def ev(r):
    e = r.get("run_event") or {}
    return e.get("fields", e)


def summary(name, by):
    idx = sorted({i for (_, i) in by if i > 0})
    out = {}
    for s in ("warm", "quod"):
        js = [job(by, s, i) for i in idx]
        js = [j for j in js if j is not None]
        out[s] = (sum(js), st.median(js), len(js))
    print(f"## {name}")
    for s, (t, m, n) in out.items():
        print(f"  {s:5} total {t:7.0f} s  median job {m:5.1f} s  ({n} commits)")
    print(f"  quod/warm total: {out['quod'][0] / out['warm'][0]:.2f}x")
    slower = sum(1 for i in idx if job(by, 'quod', i) and job(by, 'warm', i) and job(by, 'quod', i) > job(by, 'warm', i))
    print(f"  quod slower than plain-warm on {slower} of {len(idx)} commits")
    for x in ["go mod download"] + STEPS:
        for s in ("warm", "quod"):
            w = [by[(s, i)][x]["wall"] for i in idx if x in by.get((s, i), {})]
            print(f"  {x:16} {s:5} median {st.median(w):5.2f} s total {sum(w):7.1f} s")
        q = [ev(by[("quod", i)][x]) for i in idx if x in by.get(("quod", i), {})]
        outc = defaultdict(int)
        for e in q:
            outc[e.get("outcome")] += 1
        ph = {k: st.median([e.get(k) or 0 for e in q]) for k in ("setup_ms", "import_ms", "lookup_ms", "exec_ms", "record_ms")}
        print(f"    quod outcomes {dict(outc)}  median phases ms " + " ".join(f"{k[:-3]}={v:.0f}" for k, v in ph.items()))
    rss = [by[("quod", i)]["go test ./..."].get("proc_hwm_kb") or 0 for i in idx]
    look = [ev(by[("quod", i)]["go test ./..."]).get("lookup_ms") or 0 for i in idx]
    print(f"  go test lookup ms first/last {look[0]:.0f}/{look[-1]:.0f}, quod peak RSS MB first/last {rss[0]/1024:.0f}/{rss[-1]/1024:.0f}")
    return out


for name, p in zip(["before (main 5d296c4, 0b report)", "after (exp/never-worse)"], sys.argv[1:]):
    summary(name, load(p))
