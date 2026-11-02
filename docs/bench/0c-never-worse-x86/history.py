#!/usr/bin/env python3
"""Real-history benchmark for Quod on cli/cli (docs/bench/0b-x86-hetzner.md).

For each of N consecutive first-parent commits ending at the pinned commit:
  - plain-warm: fresh checkout (as actions/checkout does: new mtimes), go mod
    download, then go build ./..., go vet ./..., go test ./... with a GOCACHE
    that persists across commits (a perfect actions/cache: no restore cost).
  - quod: persistent state dir (store, ledger, warm GOCACHE), step A
    (go mod download, network, never memoized), then the three steps through
    `quod run --mode replay --warm` with the shipped trust defaults
    (proofs 2, audit 5%).
  - plain-cold (every COLD_EVERY-th commit): fresh checkout, empty GOCACHE
    shared by the job's three steps.
Order of plain-warm and quod alternates per commit. Every row goes to
rows.jsonl with the exact command, wall, exit code, load average; quod rows
carry the run and observer events and the quod process's sampled CPU/peak RSS.
"""
import json, os, shutil, subprocess, sys, time, threading

SRC = "/work/cli-src"
PIN = "9490d204a2ba95434153dedce99898ce9f281426"
N = int(os.environ.get("N", "40"))
COLD_EVERY = int(os.environ.get("COLD_EVERY", "5"))
OUT = os.environ.get("OUT", "/work/nw/hist")
QUOD = "/opt/quod-nw/target/release/quod"
STEPS = ["go build ./...", "go vet ./...", "go test ./..."]
RUNNER_UID = 1001

PLAIN_ENV = {
    "CGO_ENABLED": "0", "GOFLAGS": "-mod=mod", "GOTOOLCHAIN": "local",
    "LANG": "C.UTF-8", "TZ": "UTC", "PATH": "/usr/local/go/bin:/usr/bin:/bin",
    "GOMODCACHE": f"{OUT}/plain/gomod", "GOPATH": f"{OUT}/plain/gopath",
    "GOTELEMETRY": "off",
}
WS = f"{OUT}/plain/cli"          # constant path, as on a CI runner
QREPO = f"{OUT}/q/cli"
QSTATE = f"{OUT}/q/state"
LOGS = f"{OUT}/logs"
ROWS = f"{OUT}/rows.jsonl"


def load():
    return open("/proc/loadavg").read().split()[0]


def sh(cmd, **kw):
    return subprocess.run(cmd, shell=True, check=True, **kw)


def as_runner():
    os.setgid(RUNNER_UID); os.setgroups([]); os.setuid(RUNNER_UID)


def row(d):
    with open(ROWS, "a") as f:
        f.write(json.dumps(d) + "\n")


def fresh_checkout(commit):
    if os.path.exists(WS):
        shutil.rmtree(WS)
    t = time.monotonic()
    subprocess.run(["git", "clone", "-q", "--shared", "--no-checkout", SRC, WS],
                   check=True, preexec_fn=as_runner)
    subprocess.run(["git", "-C", WS, "checkout", "-q", "-B", "ci", commit],
                   check=True, preexec_fn=as_runner)
    return time.monotonic() - t


def plain(series, i, commit, step, gocache, home, tag):
    env = dict(PLAIN_ENV, GOCACHE=gocache, HOME=home)
    if step == "go mod download":
        env["GOPROXY"] = "https://proxy.golang.org,direct"
    else:
        env["GOPROXY"] = "off"
    base = f"{LOGS}/{i:02d}-{commit[:10]}-{series}-{tag}"
    l0 = load()
    t = time.monotonic()
    with open(base + ".out", "wb") as o, open(base + ".err", "wb") as e:
        p = subprocess.run(["/bin/sh", "-e", "-c", step], cwd=WS, env=env,
                           stdout=o, stderr=e, preexec_fn=as_runner)
    wall = time.monotonic() - t
    r = dict(series=series, i=i, commit=commit, step=step, wall=wall,
             exit=p.returncode, load_before=l0, load_after=load(),
             cmd=f"cd {WS} && env {' '.join(f'{k}={v}' for k,v in sorted(env.items()))} /bin/sh -e -c '{step}'",
             log=base)
    row(r)
    return r


def sample_proc(pid, acc, stop):
    tck = os.sysconf("SC_CLK_TCK")
    while not stop.is_set():
        try:
            st = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
            acc["cpu"] = (int(st[11]) + int(st[12])) / tck
            for ln in open(f"/proc/{pid}/status"):
                if ln.startswith("VmHWM:"):
                    acc["hwm_kb"] = max(acc.get("hwm_kb", 0), int(ln.split()[1]))
        except (FileNotFoundError, ProcessLookupError, IndexError):
            pass
        stop.wait(0.1)


def quod(i, commit, step, tag, extra):
    ev = f"{QSTATE}/events.jsonl"
    before = sum(1 for _ in open(ev)) if os.path.exists(ev) else 0
    args = [QUOD, "--state", QSTATE, "run", "--repo", QREPO, "--commit", commit] + extra + ["--step", step]
    base = f"{LOGS}/{i:02d}-{commit[:10]}-quod-{tag}"
    l0 = load()
    acc = {}
    stop = threading.Event()
    t = time.monotonic()
    with open(base + ".out", "wb") as o, open(base + ".err", "wb") as e:
        p = subprocess.Popen(args, stdout=o, stderr=e)
        th = threading.Thread(target=sample_proc, args=(p.pid, acc, stop)); th.start()
        rc = p.wait()
        stop.set(); th.join()
    wall = time.monotonic() - t
    events = [json.loads(l) for l in open(ev).read().splitlines()[before:]]
    run = [x for x in events if x.get("fields", x).get("message") == "run" or x.get("message") == "run"]
    obs = [x for x in events if x.get("fields", x).get("message") == "observer" or x.get("message") == "observer"]
    r = dict(series="quod", i=i, commit=commit, step=step, wall=wall, exit=rc,
             load_before=l0, load_after=load(), cmd=" ".join(args), log=base,
             proc_cpu_s=acc.get("cpu"), proc_hwm_kb=acc.get("hwm_kb"),
             run_event=run[-1] if run else None, observer_event=obs[-1] if obs else None,
             n_events=len(events))
    row(r)
    return r


def quod_commit(i, commit):
    subprocess.run(["git", "-C", QREPO, "checkout", "-q", "-f", "-B", "quod-hist", commit], check=True)
    subprocess.run(["git", "-C", QREPO, "clean", "-q", "-fdx", "-e", ".quod/"], check=True)
    quod(i, commit, "go mod download", "A", ["--allow-net"])
    for k, s in enumerate(STEPS):
        quod(i, commit, s, f"s{k}", ["--mode", "replay", "--warm"])


def plain_commit(i, commit, series):
    home = f"{OUT}/plain/home-{series}"
    gocache = f"{OUT}/plain/gocache-{series}"
    if os.path.exists(home):
        shutil.rmtree(home)          # HOME is fresh per job
    if series == "cold" and os.path.exists(gocache):
        shutil.rmtree(gocache)       # cold: a runner with no cache
    for d in (gocache, home):
        os.makedirs(d, exist_ok=True); os.chown(d, RUNNER_UID, RUNNER_UID)
    co = fresh_checkout(commit)
    row(dict(series=series, i=i, commit=commit, step="checkout", wall=co, exit=0, load_before=load()))
    plain(series, i, commit, "go mod download", gocache, home, "A")
    for k, s in enumerate(STEPS):
        plain(series, i, commit, s, gocache, home, f"s{k}")


def du(p):
    out = subprocess.run(["du", "-sb", p], capture_output=True, text=True).stdout.split()
    return int(out[0]) if out else 0


def main():
    commits = subprocess.run(["git", "-C", SRC, "log", "--first-parent", "--reverse",
                              "-n", str(N), "--format=%H", PIN],
                             capture_output=True, text=True, check=True).stdout.split()
    assert commits[-1] == PIN and len(commits) == N
    os.makedirs(LOGS, exist_ok=True)
    for d in (f"{OUT}/plain", f"{OUT}/plain/gomod", f"{OUT}/plain/gopath"):
        os.makedirs(d, exist_ok=True); os.chown(d, RUNNER_UID, RUNNER_UID)
    if not os.path.exists(QREPO):
        sh(f"git clone -q {SRC} {QREPO}")
        sh(f"grep -qx '.quod/' {QREPO}/.git/info/exclude || echo '.quod/' >> {QREPO}/.git/info/exclude")
    start = int(os.environ.get("START", "0"))
    for i, c in enumerate(commits):
        if i < start:
            continue
        subj = subprocess.run(["git", "-C", SRC, "log", "-1", "--format=%s", c],
                              capture_output=True, text=True).stdout.strip()
        row(dict(series="meta", i=i, commit=c, subject=subj, load=load(), t=time.time()))
        order = ["warm", "quod"] if i % 2 == 0 else ["quod", "warm"]
        for s in order:
            if s == "quod":
                quod_commit(i, c)
            else:
                plain_commit(i, c, "warm")
        if COLD_EVERY and i % COLD_EVERY == COLD_EVERY - 1:
            plain_commit(i, c, "cold")
        row(dict(series="sizes", i=i, commit=c, state_bytes=du(QSTATE),
                 cas_bytes=du(f"{QSTATE}/cas"), warm_bytes=du(f"{QSTATE}/warm"),
                 deps_bytes=du(f"{QSTATE}/deps"),
                 ledger_bytes=os.path.getsize(f"{QSTATE}/ledger.sqlite"),
                 plain_gocache_bytes=du(f"{OUT}/plain/gocache-warm")))
        print(f"[{time.strftime('%H:%M:%S')}] commit {i+1}/{N} {c[:10]} done load={load()}", flush=True)


if __name__ == "__main__":
    main()
