#!/usr/bin/env python3
"""Read-heavy steps (observe mode: always executes), interleaved series.
usage: readheavy.py N series...  (series: <bin>[:ENV=V,...])"""
import os, shutil, subprocess, sys
sys.path.insert(0, "/s")
from common import *

n = int(sys.argv[1]); series = sys.argv[2:]
h = full(H)
tag = os.environ.get("TAG", "rh")
STEPS_RH = [
    "find . -path ./.quod -prune -o -type f -print0 | xargs -0 cat > /dev/null",
    "git log -p -300 > /dev/null",
]
os.makedirs(f"{O}/logs/{tag}", exist_ok=True)
state = f"{O}/q/rh-state"
shutil.rmtree(state, ignore_errors=True)
subprocess.run(["cp", "-a", f"{O}/q/snap-main", state], check=True)
for r in range(n):
    order = series if r % 2 == 0 else list(reversed(series))
    for s in order:
        binname, _, envs = s.partition(":")
        env = dict(kv.split("=", 1) for kv in envs.split(",") if kv)
        for i, st in enumerate(STEPS_RH):
            res = quod_step(f"{O}/bin/{binname}/quod", state, h, st, f"{O}/logs/{tag}/{r}-{i}-{s.replace(':', '_')}",
                            ["--mode", "observe"], env)
            ru = res.pop("run") or {}; ob = res.pop("obs") or {}
            row(dict(tag=tag, round=r, series=s, step=st, run=ru, obs=ob, **res))
            print(r, s, i, "wall %.2f exec %.3f" % (res["wall"], ru.get("exec_ms", 0) / 1000), res["exit"], ru.get("outcome"),
                  "ring", ob.get("ring_requests"), ob.get("ring_ops"), flush=True)
