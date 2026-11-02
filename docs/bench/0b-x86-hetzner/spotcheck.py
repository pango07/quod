#!/usr/bin/env python3
"""Spot-check Quod hits: re-run each hit step plainly on a fresh checkout of
the same commit (go test with -count=1 so it really executes) and compare
exit code and normalized output with what Quod replayed.
Usage: spotcheck.py ROWS OUTDIR [MAX]"""
import json, os, re, shutil, subprocess, sys
sys.path.insert(0, "/work")
os.environ["OUT"] = sys.argv[2]
import history as h

rows = [json.loads(l) for l in open(sys.argv[1])]
MAX = int(sys.argv[3]) if len(sys.argv) > 3 else 100
hits = [r for r in rows if r["series"] == "quod" and (r.get("run_event") or {}).get("hit")]
os.makedirs(h.LOGS, exist_ok=True)
for d in (f"{h.OUT}/plain", f"{h.OUT}/plain/gomod", f"{h.OUT}/plain/gopath", f"{h.OUT}/plain/gocache"):
    os.makedirs(d, exist_ok=True); os.chown(d, h.RUNNER_UID, h.RUNNER_UID)


def norm(text, step):
    out = []
    for ln in text.splitlines():
        if step.startswith("go test"):
            ln = re.sub(r"\t\(cached\)", "", ln)
            ln = re.sub(r"\t[0-9.]+s(\t|$)", r"\1", ln)
            ln = re.sub(r"\s+$", "", ln)
        out.append(ln)
    return sorted(out) if step.startswith("go test") else out


print(f"{len(hits)} hits; checking up to {MAX}")
bad = 0
for k, r in enumerate(hits[:MAX]):
    step = r["step"]
    plain_step = step.replace("go test ", "go test -count=1 ") if step.startswith("go test") else step
    home = f"{h.OUT}/plain/home"
    shutil.rmtree(home, ignore_errors=True); os.makedirs(home); os.chown(home, h.RUNNER_UID, h.RUNNER_UID)
    h.fresh_checkout(r["commit"])
    h.plain("spot", r["i"], r["commit"], "go mod download", f"{h.OUT}/plain/gocache", home, f"A{k}")
    p = h.plain("spot", r["i"], r["commit"], plain_step, f"{h.OUT}/plain/gocache", home, f"s{k}")
    q_out = open(r["log"] + ".out", errors="replace").read()
    p_out = open(p["log"] + ".out", errors="replace").read()
    same_exit = p["exit"] == r["exit"]
    same_out = norm(q_out, step) == norm(p_out, step)
    ok = same_exit and same_out
    bad += not ok
    print(f"{'OK ' if ok else 'DIFF'} #{r['i']+1} {r['commit'][:10]} {step!r}: exit quod {r['exit']} plain {p['exit']}; "
          f"stdout lines {len(q_out.splitlines())} vs {len(p_out.splitlines())}, normalized {'equal' if same_out else 'DIFFERENT'}")
    if not same_out:
        a, b = set(norm(q_out, step)), set(norm(p_out, step))
        print("   only in quod:", sorted(a - b)[:10]); print("   only in plain:", sorted(b - a)[:10])
print(f"SPOT_DONE checked={min(len(hits), MAX)} differing={bad}")
