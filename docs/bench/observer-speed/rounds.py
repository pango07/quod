#!/usr/bin/env python3
"""Interleaved rounds at the heavy commit H from the warm state of its parent P.
usage: rounds.py N series...   series: plain | quod:<bin>[:ENV=V,...] | plainx:ENV=V,...
Each round restores the warm state, then runs build, vet, test (the job)."""
import os, shutil, subprocess, sys, time
sys.path.insert(0, "/s")
from common import *

n = int(sys.argv[1]); series = sys.argv[2:]
LIGHT = os.environ.get("LIGHT") == "1"
# LIGHT: a vet and test miss at P itself, fully warm (the ledger is dropped so Quod misses)
h = full(P) if LIGHT else full(H)
if LIGHT:
    STEPS = STEPS[1:]
    git("-C", QREPO, "checkout", "-q", "-f", "-B", "quod-hist", h)
tag = os.environ.get("TAG", "r")
os.makedirs(f"{O}/logs/{tag}", exist_ok=True)


def parse_env(s):
    return dict(kv.split("=", 1) for kv in s.split(",") if kv) if s else {}


def newfiles(d, marker):
    out = subprocess.run(f"find {d} -type f -newerBm {marker} | wc -l", shell=True, capture_output=True, text=True).stdout
    return int(out.strip() or 0)


def mark():
    m = "/work/obsspeed/marker"
    open(m, "w").close(); time.sleep(0.01)
    return m


def restore(src, dst):
    shutil.rmtree(dst, ignore_errors=True)
    subprocess.run(["cp", "-a", src, dst], check=True)
    subprocess.run(["sync"])


for r in range(n):
    order = series if r % 2 == 0 else list(reversed(series))
    for s in order:
        kind, _, rest = s.partition(":")
        if kind in ("plain", "plainx"):
            env = parse_env(rest)
            gc = f"{PLAIN}/gocache"
            restore(f"{PLAIN}/gcsnap", gc)
            home = f"{PLAIN}/home-run"
            shutil.rmtree(home, ignore_errors=True); os.makedirs(home); os.chown(home, RUNNER, RUNNER)
            fresh_checkout(h)
            for i, st in enumerate(STEPS):
                l0 = load(); m = mark()
                res = plain_step(st, gc, home, f"{O}/logs/{tag}/{r}-{s.replace('/', '_')}-{i}", extra_env=env)
                res["cache_new"] = newfiles(gc, m)
                row(dict(tag=tag, round=r, series=s, step=st, load_before=l0, **res))
                print(r, s, st, "%.2f" % res["wall"], res["exit"], "busy %.1f" % res["cpu_busy"], "new", res["cache_new"], flush=True)
        else:
            binname, _, envs = rest.partition(":")
            quod = f"{O}/bin/{binname}/quod"
            snap = f"{O}/q/snap-{binname}"
            restore(snap if os.path.exists(snap) else f"{O}/q/snap-main", QSTATE)
            if LIGHT:
                for f in os.listdir(QSTATE):
                    if f.startswith("ledger"):
                        os.unlink(f"{QSTATE}/{f}")
            for i, st in enumerate(STEPS):
                l0 = load(); m = mark()
                res = quod_step(quod, QSTATE, h, st, f"{O}/logs/{tag}/{r}-{s.replace('/', '_').replace(':', '_')}-{i}",
                                ["--mode", "replay", "--warm"], parse_env(envs))
                res["cache_new"] = newfiles(f"{QSTATE}/warm", m)
                row(dict(tag=tag, round=r, series=s, step=st, load_before=l0, **res))
                ex = (res["run"] or {}).get("exec_ms", 0) / 1000
                print(r, s, st, "%.2f" % res["wall"], res["exit"], (res["run"] or {}).get("outcome"), "exec %.2f" % ex,
                      "busy %.1f" % res["cpu_busy"], "new", res["cache_new"], flush=True)
if LIGHT:
    git("-C", QREPO, "checkout", "-q", "-f", "-B", "quod-hist", full(H))
