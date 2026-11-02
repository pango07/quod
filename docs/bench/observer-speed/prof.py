#!/usr/bin/env python3
"""perf record (system-wide, cpus 0-3) around one step of the job at H.
usage: prof.py <series> <stepidx> <outname>"""
import os, shutil, subprocess, sys, time
sys.path.insert(0, "/s")
from common import *

series, idx, name = sys.argv[1], int(sys.argv[2]), sys.argv[3]
h = full(H)
os.makedirs(f"{O}/prof", exist_ok=True)
kind, _, rest = series.partition(":")


CTL = f"{O}/prof/ctl"


def perf_start():
    # a sidecar container (perfside.sh) records while this file says so
    open(CTL + ".tmp", "w").write(name); os.rename(CTL + ".tmp", CTL)
    while not os.path.exists(CTL + ".on"):
        time.sleep(0.05)
    time.sleep(0.5)
    return True


def perf_stop(p):
    os.unlink(CTL)
    while os.path.exists(CTL + ".on"):
        time.sleep(0.05)


if kind == "plain":
    gc = f"{PLAIN}/gocache"
    shutil.rmtree(gc, ignore_errors=True); subprocess.run(["cp", "-a", f"{PLAIN}/gcsnap", gc], check=True); subprocess.run(["sync"])
    home = f"{PLAIN}/home-run"; shutil.rmtree(home, ignore_errors=True); os.makedirs(home); os.chown(home, RUNNER, RUNNER)
    fresh_checkout(h)
    for i, st in enumerate(STEPS[:idx + 1]):
        p = perf_start() if i == idx else None
        res = plain_step(st, gc, home, f"{O}/prof/{name}-{i}")
        if p: perf_stop(p)
        print(st, res, flush=True)
else:
    binname, _, envs = rest.partition(":")
    env = dict(kv.split("=", 1) for kv in envs.split(",") if kv)
    shutil.rmtree(QSTATE, ignore_errors=True); subprocess.run(["cp", "-a", f"{O}/q/snap-{binname}", QSTATE], check=True); subprocess.run(["sync"])
    for i, st in enumerate(STEPS[:idx + 1]):
        p = perf_start() if i == idx else None
        res = quod_step(f"{O}/bin/{binname}/quod", QSTATE, h, st, f"{O}/prof/{name}-{i}", ["--mode", "replay", "--warm"], env)
        if p: perf_stop(p)
        res.pop("obs"); r = res.pop("run") or {}
        print(st, res, r.get("exec_ms"), flush=True)
#os.system(f"perf report -i {O}/prof/{name}.data --no-children --sort comm --stdio 2>/dev/null | grep -v '^#' | grep -v '^$' | head -25")
#os.system(f"perf report -i {O}/prof/{name}.data --no-children --sort dso,sym --stdio -g none 2>/dev/null | grep -v '^#' | grep -v '^$' | head -45")
open(CTL + ".done", "w").close()
