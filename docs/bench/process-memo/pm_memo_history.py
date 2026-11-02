#!/usr/bin/env python3
"""The prototype on the 40-commit history: `go test ./...` with pmemo as
-toolexec/-exec in replay mode (links and test binaries memoized, proofs 2),
paired with a plain-warm run (Go's own caches, as the 0b bench's warm
series) on every commit, alternating order. Both as uid 1001 on a fresh
checkout per commit with their own persistent GOCACHE and a fresh HOME.
Rows to OUT/rows.jsonl; pmemo's per-process log to OUT/memo/NN.jsonl.
"""
import json, os, shutil, subprocess, sys, time

SRC = "/work/cli-src"
PIN = "9490d204a2ba95434153dedce99898ce9f281426"
N = int(os.environ.get("N", "40"))
OUT = os.environ.get("OUT", "/work/pm/memo")
RUNNER_UID = 1001
SHIM = "/work/pm/pmemo"
STEP = "go test ./..."

BASE = {
    "CGO_ENABLED": "0", "GOTOOLCHAIN": "local",
    "LANG": "C.UTF-8", "TZ": "UTC", "PATH": "/usr/local/go/bin:/usr/bin:/bin",
    "GOMODCACHE": "/work/hist/plain/gomod", "GOPATH": "/work/hist/plain/gopath",
    "GOTELEMETRY": "off", "GOPROXY": "off",
}
ROWS = f"{OUT}/rows.jsonl"


def load():
    return open("/proc/loadavg").read().split()[0]


def as_runner():
    os.setgid(RUNNER_UID); os.setgroups([]); os.setuid(RUNNER_UID)


def row(d):
    with open(ROWS, "a") as f:
        f.write(json.dumps(d) + "\n")


def fresh_checkout(ws, commit):
    if os.path.exists(ws):
        shutil.rmtree(ws)
    subprocess.run(["git", "clone", "-q", "--shared", "--no-checkout", SRC, ws], check=True, preexec_fn=as_runner)
    subprocess.run(["git", "-C", ws, "checkout", "-q", "-B", "ci", commit], check=True, preexec_fn=as_runner)


def prep(series):
    home = f"{OUT}/{series}/home"
    if os.path.exists(home):
        shutil.rmtree(home)
    for d in (f"{OUT}/{series}/gocache", home, f"{OUT}/{series}/tmp"):
        os.makedirs(d, exist_ok=True); os.chown(d, RUNNER_UID, RUNNER_UID)
    return home


def run_step(series, i, commit, ws, env, cmd):
    base = f"{OUT}/logs/{i:02d}-{commit[:10]}-{series}"
    l0 = load()
    t = time.monotonic()
    with open(base + ".out", "wb") as o, open(base + ".err", "wb") as e:
        p = subprocess.run(["/bin/sh", "-e", "-c", cmd], cwd=ws, env=env, stdout=o, stderr=e, preexec_fn=as_runner)
    wall = time.monotonic() - t
    row(dict(series=series, i=i, commit=commit, step=cmd, wall=wall, exit=p.returncode, load_before=l0, load_after=load(), log=base))
    print(f"  {series}: {wall:.1f}s exit={p.returncode} load={load()}", flush=True)
    return wall


def job(series, i, commit):
    ws = f"{OUT}/{series}/cli"
    home = prep(series)
    fresh_checkout(ws, commit)
    env = dict(BASE, GOCACHE=f"{OUT}/{series}/gocache", HOME=home)
    if series == "memo":
        env.update(GOFLAGS=f"-mod=mod -count=1 -toolexec={SHIM} -exec={SHIM}", GODEBUG="goindex=0",
                   PMEMO_STORE=f"{OUT}/store", PMEMO_MODE="replay", PMEMO_PROOFS=os.environ.get("PROOFS", "2"),
                   PMEMO_LOG=f"{OUT}/memo/{i:02d}.jsonl", PMEMO_TMP=f"{OUT}/{series}/tmp", PMEMO_WS=ws)
    else:
        env.update(GOFLAGS="-mod=mod")
    dl = dict(env, GOPROXY="https://proxy.golang.org,direct", GOFLAGS="-mod=mod")
    subprocess.run(["/bin/sh", "-e", "-c", "go mod download"], cwd=ws, env=dl, preexec_fn=as_runner,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return run_step(series, i, commit, ws, env, STEP)


def main():
    commits = subprocess.run(["git", "-C", SRC, "log", "--first-parent", "--reverse", "-n", str(N), "--format=%H", PIN],
                             capture_output=True, text=True, check=True).stdout.split()
    assert commits[-1] == PIN and len(commits) == N
    for d in (OUT, f"{OUT}/logs", f"{OUT}/memo", f"{OUT}/store", f"{OUT}/memo", f"{OUT}/warm"):
        os.makedirs(d, exist_ok=True)
    for d in (f"{OUT}/memo", f"{OUT}/store", f"{OUT}/warm"):
        os.chown(d, RUNNER_UID, RUNNER_UID)
    start = int(os.environ.get("START", "0"))
    for i, c in enumerate(commits):
        if i < start:
            continue
        subj = subprocess.run(["git", "-C", SRC, "log", "-1", "--format=%s", c], capture_output=True, text=True).stdout.strip()
        row(dict(series="meta", i=i, commit=c, subject=subj, load=load(), t=time.time()))
        print(f"[{time.strftime('%H:%M:%S')}] commit {i+1}/{N} {c[:10]} {subj[:50]}", flush=True)
        series = os.environ.get("SERIES", "warm,memo").split(",")
        for s in (series if i % 2 == 0 else series[::-1]):
            job(s, i, c)
    print("MEMO_DONE", flush=True)


if __name__ == "__main__":
    main()
