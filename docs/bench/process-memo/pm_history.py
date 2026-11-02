#!/usr/bin/env python3
"""Per-process instrumentation of the 40-commit cli/cli history.

For each of the N first-parent commits ending at the pinned one (the same
commits as docs/bench/0b-x86-hetzner.md), as uid 1001 on a fresh checkout:
`go mod download`, then `go build ./...`, `go vet ./...`, `go test ./...`
under exactly Quod's warm-miss conditions (one GOCACHE persisting across
commits, GOFLAGS -count=1, GODEBUG goindex=0, a fresh HOME per job) with
every tool and test binary run through pmwrap (-toolexec and -exec), which
appends one JSON record per process to OUT/NN-sK.jsonl. Afterwards the last
commit runs once more on the same tree (series "again") to measure which
process keys are stable across two runs of one tree.

Nothing is skipped: this measures what a per-process memo *could* skip.
"""
import json, os, shutil, subprocess, sys, time

SRC = "/work/cli-src"
PIN = "9490d204a2ba95434153dedce99898ce9f281426"
N = int(os.environ.get("N", "40"))
OUT = os.environ.get("OUT", "/work/pm/hist")
STEPS = ["go build ./...", "go vet ./...", "go test ./..."]
RUNNER_UID = 1001
WRAP = "/work/pm/pmwrap"

WS = f"{OUT}/cli"
ENV = {
    "CGO_ENABLED": "0", "GOTOOLCHAIN": "local",
    "LANG": "C.UTF-8", "TZ": "UTC", "PATH": "/usr/local/go/bin:/usr/bin:/bin",
    "GOMODCACHE": "/work/hist/plain/gomod", "GOPATH": "/work/hist/plain/gopath",
    "GOTELEMETRY": "off",
    "GOFLAGS": f"-mod=mod -count=1 -toolexec={WRAP} -exec={WRAP}",
    "GODEBUG": "goindex=0",
    "GOCACHE": f"{OUT}/gocache", "HOME": f"{OUT}/home",
    "PMWRAP_TMP": f"{OUT}/tmp",
}
ROWS = f"{OUT}/rows.jsonl"


def load():
    return open("/proc/loadavg").read().split()[0]


def as_runner():
    os.setgid(RUNNER_UID); os.setgroups([]); os.setuid(RUNNER_UID)


def row(d):
    with open(ROWS, "a") as f:
        f.write(json.dumps(d) + "\n")


def fresh_checkout(commit):
    if os.path.exists(WS):
        shutil.rmtree(WS)
    subprocess.run(["git", "clone", "-q", "--shared", "--no-checkout", SRC, WS],
                   check=True, preexec_fn=as_runner)
    subprocess.run(["git", "-C", WS, "checkout", "-q", "-B", "ci", commit],
                   check=True, preexec_fn=as_runner)


def step(series, i, commit, s, k):
    env = dict(ENV)
    tag = "A" if s == "go mod download" else f"s{k}"
    if s == "go mod download":
        env["GOPROXY"] = "https://proxy.golang.org,direct"
        env["GOFLAGS"] = "-mod=mod"
    else:
        env["GOPROXY"] = "off"
    env["PMWRAP_OUT"] = f"{OUT}/procs/{i:02d}-{series}-{tag}.jsonl"
    base = f"{OUT}/logs/{i:02d}-{commit[:10]}-{series}-{tag}"
    l0 = load()
    t = time.monotonic()
    with open(base + ".out", "wb") as o, open(base + ".err", "wb") as e:
        p = subprocess.run(["/bin/sh", "-e", "-c", s], cwd=WS, env=env,
                           stdout=o, stderr=e, preexec_fn=as_runner)
    wall = time.monotonic() - t
    row(dict(series=series, i=i, commit=commit, step=s, wall=wall, exit=p.returncode,
             load_before=l0, load_after=load(), procs=env["PMWRAP_OUT"], log=base))
    print(f"  {series} {tag} {s}: {wall:.1f}s exit={p.returncode} load={load()}", flush=True)


def job(series, i, commit, checkout=True):
    if os.path.exists(ENV["HOME"]):
        shutil.rmtree(ENV["HOME"])
    for d in (ENV["GOCACHE"], ENV["HOME"], ENV["PMWRAP_TMP"]):
        os.makedirs(d, exist_ok=True); os.chown(d, RUNNER_UID, RUNNER_UID)
    if checkout:
        fresh_checkout(commit)
    step(series, i, commit, "go mod download", -1)
    for k, s in enumerate(STEPS):
        step(series, i, commit, s, k)


def main():
    commits = subprocess.run(["git", "-C", SRC, "log", "--first-parent", "--reverse",
                              "-n", str(N), "--format=%H", PIN],
                             capture_output=True, text=True, check=True).stdout.split()
    assert commits[-1] == PIN and len(commits) == N
    for d in (OUT, f"{OUT}/logs", f"{OUT}/procs"):
        os.makedirs(d, exist_ok=True)
    os.chmod(f"{OUT}/procs", 0o777)
    os.chown(OUT, RUNNER_UID, RUNNER_UID)   # the checkout is cloned as the runner
    start = int(os.environ.get("START", "0"))
    for i, c in enumerate(commits):
        if i < start:
            continue
        subj = subprocess.run(["git", "-C", SRC, "log", "-1", "--format=%s", c],
                              capture_output=True, text=True).stdout.strip()
        row(dict(series="meta", i=i, commit=c, subject=subj, load=load(), t=time.time()))
        print(f"[{time.strftime('%H:%M:%S')}] commit {i+1}/{N} {c[:10]} {subj[:50]}", flush=True)
        job("pm", i, c)
    # The same tree again: key stability.
    print(f"[{time.strftime('%H:%M:%S')}] again {N}/{N}", flush=True)
    job("again", N - 1, commits[-1], checkout=False)
    print("PM_DONE", flush=True)


if __name__ == "__main__":
    main()
