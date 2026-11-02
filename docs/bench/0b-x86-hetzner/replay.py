#!/usr/bin/env python3
"""Replay speed at the pinned commit (unchanged inputs), interleaved rounds.

Series per round (order rotates each round):
  warm      plain job, fresh checkout, GOCACHE populated (warm actions/cache)
  warm-ws   plain job, same checkout as the last run (persistent workspace:
            mtimes kept, so Go's test cache can hit tests that open files)
  cold      plain job, fresh checkout, empty GOCACHE
  quod      quod job: step A + three replays, persistent store/ledger
  quod-nowarm  same, with the quod warm GOCACHE deleted first (a fresh
            runner with only Quod's store and ledger)
Round 0 is warm-up and is dropped in the report.
"""
import os, shutil, subprocess, sys, time
os.environ.setdefault("OUT", "/work/rs")
sys.path.insert(0, "/work")
import history as h

ROUNDS = int(os.environ.get("ROUNDS", "6"))


def plain_job(r, series, gocache, fresh):
    home = f"{h.OUT}/plain/home-{series}"
    if os.path.exists(home):
        shutil.rmtree(home)
    if series == "cold" and os.path.exists(gocache):
        shutil.rmtree(gocache)
    for d in (gocache, home):
        os.makedirs(d, exist_ok=True); os.chown(d, h.RUNNER_UID, h.RUNNER_UID)
    if fresh or not os.path.exists(h.WS):
        co = h.fresh_checkout(h.PIN)
        h.row(dict(series=series, i=r, commit=h.PIN, step="checkout", wall=co, exit=0))
    h.plain(series, r, h.PIN, "go mod download", gocache, home, "A")
    for k, s in enumerate(h.STEPS):
        h.plain(series, r, h.PIN, s, gocache, home, f"s{k}")


def quod_job(r, nowarm):
    if nowarm:
        shutil.rmtree(f"{h.QSTATE}/warm", ignore_errors=True)
    h.subprocess.run(["git", "-C", h.QREPO, "checkout", "-q", "-f", "-B", "quod-hist", h.PIN], check=True)
    tag = "quod-nowarm" if nowarm else "quod"
    h.quod(r, h.PIN, "go mod download", f"{tag}-A", ["--allow-net"])
    for k, s in enumerate(h.STEPS):
        x = h.quod(r, h.PIN, s, f"{tag}-s{k}", ["--mode", "replay", "--warm"])


def main():
    os.makedirs(h.LOGS, exist_ok=True)
    for d in (f"{h.OUT}/plain", f"{h.OUT}/plain/gomod", f"{h.OUT}/plain/gopath"):
        os.makedirs(d, exist_ok=True); os.chown(d, h.RUNNER_UID, h.RUNNER_UID)
    if not os.path.exists(h.QREPO):
        h.sh(f"git clone -q {h.SRC} {h.QREPO}")
        h.sh(f"echo '.quod/' >> {h.QREPO}/.git/info/exclude")
    # Prime: plain warm cache filled once; quod key proven (2 executions).
    plain_job(-1, "warm", f"{h.OUT}/plain/gocache-warm", True)
    quod_job(-2, False)
    quod_job(-1, False)
    series = ["warm", "quod", "cold", "quod-nowarm"]
    for r in range(ROUNDS):
        rot = series[r % len(series):] + series[:r % len(series)]
        for s in rot:
            if s == "warm":
                plain_job(r, "warm", f"{h.OUT}/plain/gocache-warm", True)
                # the same job again on the same checkout (persistent workspace)
                plain_job(r, "warm-ws", f"{h.OUT}/plain/gocache-warm", False)
            elif s == "cold":
                plain_job(r, "cold", f"{h.OUT}/plain/gocache-cold", True)
            elif s == "quod":
                quod_job(r, False)
            else:
                quod_job(r, True)
        print(f"[{time.strftime('%H:%M:%S')}] round {r} done load={h.load()}", flush=True)


if __name__ == "__main__":
    main()
