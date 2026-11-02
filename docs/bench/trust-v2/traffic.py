#!/usr/bin/env python3
"""Trust v2 hit-rate benchmark on cli/cli's real history (docs/trust-v2.md).

Quod only (the plain baselines are in docs/bench/0b-x86-hetzner.md). For the
N consecutive first-parent commits ending at the pinned commit, through one
persistent state dir (store, ledger, warm GOCACHE):

  MODE=history   one job per commit, as 0b's history did: checkout, step A
                 (`go mod download`, network, never memoized), then
                 `go build ./...`, `go vet ./...`, `go test ./...` through
                 `quod run --mode replay --warm` plus $POLICY.
  MODE=traffic   realistic CI traffic: each commit first as its pull request
                 head (the merge's second parent when it is a merge commit,
                 the commit itself on a `pr` branch otherwise), then as the
                 merge on `main`, then with probability RERUN (seeded) a
                 re-run of the merge job.

$POLICY is appended to every keyed step: empty for trust v2's defaults
(`--proofs 1 --shadow 2 --audit 0.05`), `--proofs 2 --shadow 0` for v1's.
Every step run is a row in $OUT/rows.jsonl with its phase, wall, exit code
and the quod `run` event (outcome, hit, first mismatch), and summary.py
turns the rows into the tables.
"""
import json, os, random, subprocess, sys, time

SRC = os.environ.get("SRC", "/work/cli-src")
PIN = os.environ.get("PIN", "9490d204a2ba95434153dedce99898ce9f281426")
N = int(os.environ.get("N", "40"))
MODE = os.environ.get("MODE", "history")
RERUN = float(os.environ.get("RERUN", "0.10"))
SEED = int(os.environ.get("SEED", "42"))
OUT = os.environ["OUT"]
QUOD = os.environ.get("QUOD", "/opt/quod-v2/target/release/quod")
POLICY = os.environ.get("POLICY", "").split()
STEPS = ["go build ./...", "go vet ./...", "go test ./..."]

REPO = f"{OUT}/cli"
STATE = f"{OUT}/state"
LOGS = f"{OUT}/logs"
ROWS = f"{OUT}/rows.jsonl"


def load():
    return open("/proc/loadavg").read().split()[0]


def row(d):
    with open(ROWS, "a") as f:
        f.write(json.dumps(d) + "\n")


def git(*args, **kw):
    return subprocess.run(["git", "-C", REPO, *args], check=True, text=True,
                          capture_output=True, **kw).stdout.strip()


def checkout(branch, commit):
    git("checkout", "-q", "-f", "-B", branch, commit)
    git("clean", "-q", "-fdx", "-e", ".quod/")


def quod(i, phase, commit, step, tag, extra):
    ev = f"{STATE}/events.jsonl"
    before = sum(1 for _ in open(ev)) if os.path.exists(ev) else 0
    args = [QUOD, "--state", STATE, "run", "--repo", REPO, "--commit", commit] + extra + ["--step", step]
    base = f"{LOGS}/{i:02d}-{phase}-{commit[:10]}-{tag}"
    l0 = load()
    t = time.monotonic()
    with open(base + ".out", "wb") as o, open(base + ".err", "wb") as e:
        rc = subprocess.run(args, stdout=o, stderr=e).returncode
    wall = time.monotonic() - t
    events = [json.loads(l) for l in open(ev).read().splitlines()[before:]] if os.path.exists(ev) else []
    msg = lambda x: x.get("fields", x).get("message") or x.get("message")
    run = [x for x in events if msg(x) == "run"]
    r = dict(i=i, phase=phase, commit=commit, step=step, wall=wall, exit=rc,
             load_before=l0, load_after=load(), cmd=" ".join(args), log=base,
             run_event=run[-1] if run else None)
    row(r)
    return r


def job(i, phase, branch, commit):
    """One CI job: checkout, step A, the three keyed steps."""
    checkout(branch, commit)
    t = time.monotonic()
    quod(i, phase, commit, "go mod download", "A", ["--allow-net"])
    for k, s in enumerate(STEPS):
        quod(i, phase, commit, s, f"s{k}", ["--mode", "replay", "--warm"] + POLICY)
    row(dict(i=i, phase=phase, commit=commit, step="job", wall=time.monotonic() - t, exit=0))


def pr_head(commit):
    """The pull request head a merge commit merged, else the commit itself."""
    try:
        return subprocess.run(["git", "-C", SRC, "rev-parse", "--verify", "-q", f"{commit}^2"],
                              check=True, text=True, capture_output=True).stdout.strip()
    except subprocess.CalledProcessError:
        return commit


def main():
    commits = subprocess.run(["git", "-C", SRC, "log", "--first-parent", "--reverse",
                              "-n", str(N), "--format=%H", PIN],
                             capture_output=True, text=True, check=True).stdout.split()
    assert commits[-1] == PIN and len(commits) == N
    os.makedirs(LOGS, exist_ok=True)
    if not os.path.exists(REPO):
        subprocess.run(["git", "clone", "-q", SRC, REPO], check=True)
        with open(f"{REPO}/.git/info/exclude", "a") as f:
            f.write(".quod/\n")
    rng = random.Random(SEED)
    row(dict(step="meta", mode=MODE, policy=POLICY, rerun=RERUN, seed=SEED, quod=QUOD,
             quod_version=subprocess.run([QUOD, "--version"], capture_output=True, text=True).stdout.strip(),
             n=N, t=time.time()))
    start = int(os.environ.get("START", "0"))
    for i, c in enumerate(commits):
        if i < start:
            continue
        subj = subprocess.run(["git", "-C", SRC, "log", "-1", "--format=%s", c],
                              capture_output=True, text=True).stdout.strip()
        head = pr_head(c)
        row(dict(step="commit", i=i, commit=c, subject=subj, pr_head=head,
                 merge_commit=(head != c), load=load(), t=time.time()))
        if MODE == "history":
            job(i, "merge", "quod-hist", c)
        else:
            job(i, "pr", "pr", head)
            job(i, "merge", "main", c)
            if rng.random() < RERUN:
                job(i, "rerun", "main", c)
        print(f"[{time.strftime('%H:%M:%S')}] {MODE} commit {i+1}/{N} {c[:10]} done load={load()}", flush=True)
    row(dict(step="done", t=time.time(),
             ledger_bytes=os.path.getsize(f"{STATE}/ledger.sqlite")))


if __name__ == "__main__":
    main()
