import json, sys, statistics as st
from collections import defaultdict
tags = sys.argv[1:]
d = defaultdict(list)
for l in open("/work/obsspeed/rows.jsonl"):
    r = json.loads(l)
    if r.get("tag") not in tags:
        continue
    ru = r.get("run") or {}
    ex = ru.get("exec_ms", r["wall"] * 1000) / 1000
    ob = r.get("obs") or {}
    d[(r["tag"], r["step"], r["series"])].append((r["wall"], ex, r["exit"], ru.get("outcome"), ob.get("ring_requests"), ru.get("manifest_id", "")[-8:], ru.get("observer")))
for k in sorted(d):
    v = d[k]
    w = [x[0] for x in v]; e = [x[1] for x in v]
    print(f"{k[0]:14} {k[1]:15} {k[2]:34} n={len(v)} wall med {st.median(w):6.2f} min {min(w):6.2f}  exec med {st.median(e):6.2f} min {min(e):6.2f}  exits {sorted(set(x[2] for x in v))} {sorted(set(str(x[3]) for x in v))} ring {v[0][4]} manifests {sorted(set(x[5] for x in v))} {sorted(set(str(x[6]) for x in v))}")
