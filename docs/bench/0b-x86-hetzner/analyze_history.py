#!/usr/bin/env python3
"""Summarize /work/hist/rows.jsonl into markdown tables (stdout)."""
import json, statistics as st, sys
from collections import defaultdict

rows = [json.loads(l) for l in open(sys.argv[1])]
STEPS = ["go build ./...", "go vet ./...", "go test ./..."]
meta = {r["i"]: r for r in rows if r["series"] == "meta"}
sizes = {r["i"]: r for r in rows if r["series"] == "sizes"}
by = defaultdict(dict)   # (series, i) -> step -> row
for r in rows:
    if r["series"] in ("warm", "cold", "quod"):
        by[(r["series"], r["i"])][r["step"]] = r
N = len(meta)
idx = sorted(meta)


def job(series, i, with_setup=True):
    d = by.get((series, i))
    if not d or not all(s in d for s in STEPS):
        return None
    t = sum(d[s]["wall"] for s in STEPS)
    if with_setup:
        t += d.get("go mod download", {}).get("wall", 0) + d.get("checkout", {}).get("wall", 0)
    return t


def med(v):
    return st.median(v) if v else float("nan")


def outcome(r):
    e = r.get("run_event") or {}
    return e.get("outcome"), e.get("hit"), e.get("first_mismatch"), e.get("candidates")


print("| # | Commit | Subject | plain-warm job | plain-cold job | quod job | build | vet | test | test first changed input |")
print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
tot = defaultdict(float)
hits = defaultdict(int)
cold_pairs = []
for i in idx:
    m = meta[i]
    w, c, q = job("warm", i), job("cold", i), job("quod", i)
    if w is None or q is None:
        continue
    tot["warm"] += w; tot["quod"] += q
    if c is not None:
        cold_pairs.append((i, w, c, q))
    cells = []
    for s in STEPS:
        r = by[("quod", i)][s]
        o, hit, fm, cand = outcome(r)
        if hit:
            hits[s] += 1
        cells.append(f"{'HIT' if hit else o} {r['wall']:.1f}s" + ("" if r["exit"] == 0 else f" exit {r['exit']}"))
    fm = outcome(by[("quod", i)]["go test ./..."])[2]
    pw = "".join("" if by[("warm", i)][s]["exit"] == 0 else f" ({s.split()[1]} exit {by[('warm', i)][s]['exit']})" for s in STEPS)
    print(f"| {i+1} | `{m['commit'][:10]}` | {m['subject'][:48]} | {w:.1f}s{pw} | {'' if c is None else f'{c:.1f}s'} | {q:.1f}s | {cells[0]} | {cells[1]} | {cells[2]} | `{fm or '-'}` |")
n = sum(1 for i in idx if job("warm", i) is not None and job("quod", i) is not None)
print()
print(f"commits: {n}")
for s in STEPS:
    print(f"hits {s}: {hits[s]}/{n}")
print(f"total plain-warm {tot['warm']:.0f}s, quod {tot['quod']:.0f}s, saved vs warm {(1-tot['quod']/tot['warm'])*100:.1f}%")
ws = [job("warm", i) for i in idx if job("warm", i)]
qs = [job("quod", i) for i in idx if job("quod", i)]
print(f"median job plain-warm {med(ws):.1f}s quod {med(qs):.1f}s")
if cold_pairs:
    print(f"cold sample n={len(cold_pairs)}: median cold {med([c for _,_,c,_ in cold_pairs]):.1f}s, warm on same commits {med([w for _,w,_,_ in cold_pairs]):.1f}s, quod on same {med([q for *_,q in cold_pairs]):.1f}s")
    print(f"  extrapolated cold total = median cold x n = {med([c for _,_,c,_ in cold_pairs])*n:.0f}s")
# per-step medians
for series in ("warm", "cold", "quod"):
    for s in STEPS + ["go mod download", "checkout"]:
        v = [by[(series, i)][s]["wall"] for i in idx if (series, i) in by and s in by[(series, i)]]
        if v:
            print(f"step median {series:5s} {s:16s} n={len(v):2d} median {med(v):6.1f}s total {sum(v):7.1f}s")
# quod misses breakdown by outcome
oc = defaultdict(lambda: defaultdict(int))
for i in idx:
    for s in STEPS:
        r = by.get(("quod", i), {}).get(s)
        if r:
            oc[s][outcome(r)[0]] += 1
print("outcomes:", {s: dict(v) for s, v in oc.items()})
# resources
cpu = [r["proc_cpu_s"] for i in idx for s in STEPS if (r := by.get(("quod", i), {}).get(s)) and r.get("proc_cpu_s") is not None]
hwm = [r["proc_hwm_kb"] for i in idx for s in STEPS if (r := by.get(("quod", i), {}).get(s)) and r.get("proc_hwm_kb")]
su = [(r["observer_event"] or {}).get("serve_user_ms", 0) + (r["observer_event"] or {}).get("serve_sys_ms", 0) for i in idx for s in STEPS if (r := by.get(("quod", i), {}).get(s)) and r.get("observer_event")]
if cpu:
    print(f"quod process CPU s per step: median {med(cpu):.2f} max {max(cpu):.2f} total {sum(cpu):.0f}")
if hwm:
    print(f"quod process peak RSS MB: median {med(hwm)/1024:.0f} max {max(hwm)/1024:.0f}")
if su:
    print(f"observer serving CPU (mounted) s: median {med(su)/1000:.2f} max {max(su)/1000:.2f}")
if sizes:
    a, b = sizes[min(sizes)], sizes[max(sizes)]
    for k in ("state_bytes", "cas_bytes", "warm_bytes", "deps_bytes", "ledger_bytes", "plain_gocache_bytes"):
        print(f"size {k}: after first commit {a[k]/1e6:.0f} MB, after last {b[k]/1e6:.0f} MB")
