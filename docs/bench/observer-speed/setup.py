#!/usr/bin/env python3
"""One-time setup: warm plain and Quod at the parent commit P, download H's
modules, snapshot both warm states."""
import os, shutil, subprocess, sys
sys.path.insert(0, "/s")
from common import *

BIN = os.environ.get("BIN", "main")
QUOD = f"{O}/bin/{BIN}/quod"
p, h = full(P), full(H)
os.makedirs(f"{O}/logs", exist_ok=True)
for d in (PLAIN, f"{PLAIN}/gomod", f"{PLAIN}/gopath", f"{PLAIN}/home"):
    os.makedirs(d, exist_ok=True); os.chown(d, RUNNER, RUNNER)
os.chown(O, RUNNER, RUNNER)

if not os.path.exists(f"{O}/src"):
    subprocess.run(["git", "clone", "-q", "--bare", SRC, f"{O}/src"], check=True)
    subprocess.run(["chown", "-R", f"{RUNNER}:{RUNNER}", f"{O}/src"], check=True)
what = sys.argv[1:] or ["plain", "quod"]

if "plain" in what:
    gc = f"{PLAIN}/gocache"
    shutil.rmtree(gc, ignore_errors=True); os.makedirs(gc); os.chown(gc, RUNNER, RUNNER)
    fresh_checkout(p)
    print("plain A", plain_step("go mod download", gc, f"{PLAIN}/home", f"{O}/logs/setup-plain-A", net=True), flush=True)
    for i, s in enumerate(STEPS):
        print("plain", s, plain_step(s, gc, f"{PLAIN}/home", f"{O}/logs/setup-plain-{i}"), flush=True)
    fresh_checkout(h)
    print("plain A@H", plain_step("go mod download", gc, f"{PLAIN}/home", f"{O}/logs/setup-plain-AH", net=True), flush=True)
    shutil.rmtree(f"{PLAIN}/gcsnap", ignore_errors=True)
    subprocess.run(["cp", "-a", gc, f"{PLAIN}/gcsnap"], check=True)

if "quod" in what:
    if not os.path.exists(QREPO):
        os.makedirs(os.path.dirname(QREPO), exist_ok=True)
        subprocess.run(["git", "clone", "-q", SRC, QREPO], check=True)
        with open(f"{QREPO}/.git/info/exclude", "a") as f:
            f.write(".quod/\n")
    shutil.rmtree(QSTATE, ignore_errors=True)
    git("-C", QREPO, "checkout", "-q", "-f", "-B", "quod-hist", p)
    git("-C", QREPO, "clean", "-q", "-fdx", "-e", ".quod/")
    r = quod_step(QUOD, QSTATE, p, "go mod download", f"{O}/logs/setup-q-A", ["--allow-net"])
    print("quod A", r["wall"], r["exit"], flush=True)
    for i, s in enumerate(STEPS):
        r = quod_step(QUOD, QSTATE, p, s, f"{O}/logs/setup-q-{i}", ["--mode", "replay", "--warm"])
        print("quod", s, r["wall"], r["exit"], (r["run"] or {}).get("outcome"), flush=True)
    git("-C", QREPO, "checkout", "-q", "-f", "-B", "quod-hist", h)
    git("-C", QREPO, "clean", "-q", "-fdx", "-e", ".quod/")
    r = quod_step(QUOD, QSTATE, h, "go mod download", f"{O}/logs/setup-q-AH", ["--allow-net"])
    print("quod A@H", r["wall"], r["exit"], flush=True)
    snap = f"{O}/q/snap-{BIN}"
    shutil.rmtree(snap, ignore_errors=True)
    subprocess.run(["cp", "-a", QSTATE, snap], check=True)
    subprocess.run(["du", "-sh", snap, f"{PLAIN}/gcsnap"])
