#!/usr/bin/env python3
"""Spot-check Quod replays (0b's spotcheck.py, made standalone for both
drivers' rows): re-run each replayed step plainly on a fresh checkout of the
same commit, as uid 1001 with the plain baseline's env (`go test` as
`go test -count=1`, so every package really executes), and compare the exit
code and the normalized stdout with what Quod replayed. go test lines are
sorted, durations and `(cached)` removed.

    spotcheck.py OUTDIR ROWS [ROWS ...]      (env MAX caps the number checked)
"""
import json, os, re, shutil, subprocess, sys, time

SRC = "/work/cli-src"
RUNNER_UID = 1001
OUT = sys.argv[1]
MAX = int(os.environ.get("MAX", "1000"))
ENV = {
    "CGO_ENABLED": "0", "GOFLAGS": "-mod=mod", "GOTOOLCHAIN": "local",
    "LANG": "C.UTF-8", "TZ": "UTC", "PATH": "/usr/local/go/bin:/usr/bin:/bin",
    "GOTELEMETRY": "off", "GOCACHE": f"{OUT}/gocache", "GOMODCACHE": f"{OUT}/gomod",
    "GOPATH": f"{OUT}/gopath", "HOME": f"{OUT}/home",
}
WS = f"{OUT}/cli"
LOGS = f"{OUT}/logs"


def as_runner():
    os.setgid(RUNNER_UID); os.setgroups([]); os.setuid(RUNNER_UID)


def norm(text, step):
    out = []
    for ln in text.splitlines():
        if step.startswith("go test"):
            ln = re.sub(r"\t\(cached\)", "", ln)
            ln = re.sub(r"\t[0-9.]+s(\t|$)", r"\1", ln)
            ln = re.sub(r"\s+$", "", ln)
        out.append(ln)
    return sorted(out) if step.startswith("go test") else out


def run(step, tag, proxy):
    e = dict(ENV, GOPROXY="https://proxy.golang.org,direct" if proxy else "off")
    base = f"{LOGS}/{tag}"
    t = time.monotonic()
    with open(base + ".out", "wb") as o, open(base + ".err", "wb") as er:
        rc = subprocess.run(["/bin/sh", "-e", "-c", step], cwd=WS, env=e, stdout=o, stderr=er,
                            preexec_fn=as_runner).returncode
    return rc, open(base + ".out", errors="replace").read(), time.monotonic() - t


def main():
    hits = []
    for p in sys.argv[2:]:
        for l in open(p):
            r = json.loads(l)
            ev = r.get("run_event") or {}
            f = ev.get("fields", ev)
            if r.get("series") == "quod" and f.get("hit") and f.get("outcome") == "replayed":
                r["_rows"] = os.path.basename(os.path.dirname(p))
                hits.append(r)
    for d in (OUT, LOGS, ENV["GOCACHE"], ENV["GOMODCACHE"], ENV["GOPATH"]):
        os.makedirs(d, exist_ok=True); os.chown(d, RUNNER_UID, RUNNER_UID)
    print(f"{len(hits)} replays; checking up to {MAX}", flush=True)
    bad = 0
    for k, r in enumerate(hits[:MAX]):
        step = r["step"]
        plain_step = step.replace("go test ", "go test -count=1 ") if step.startswith("go test") else step
        shutil.rmtree(WS, ignore_errors=True)
        shutil.rmtree(ENV["HOME"], ignore_errors=True)
        os.makedirs(ENV["HOME"]); os.chown(ENV["HOME"], RUNNER_UID, RUNNER_UID)
        subprocess.run(["git", "clone", "-q", "--shared", "--no-checkout", SRC, WS], check=True, preexec_fn=as_runner)
        subprocess.run(["git", "-C", WS, "checkout", "-q", "-B", "ci", r["commit"]], check=True, preexec_fn=as_runner)
        run("go mod download", f"{k:03d}-A", True)
        rc, p_out, wall = run(plain_step, f"{k:03d}-s", False)
        q_out = open(r["log"] + ".out", errors="replace").read()
        same_exit = rc == r["exit"]
        same_out = norm(q_out, step) == norm(p_out, step)
        ok = same_exit and same_out
        bad += not ok
        where = f"{r['_rows']} #{r['i']+1} {r.get('phase', 'merge')} {r['commit'][:10]}"
        print(f"{'OK ' if ok else 'DIFF'} {where} {step!r}: exit quod {r['exit']} plain {rc}; "
              f"stdout lines {len(q_out.splitlines())} vs {len(p_out.splitlines())}, "
              f"normalized {'equal' if same_out else 'DIFFERENT'}; plain {wall:.1f}s", flush=True)
        if not same_out:
            a, b = set(norm(q_out, step)), set(norm(p_out, step))
            print("   only in quod:", sorted(a - b)[:10]); print("   only in plain:", sorted(b - a)[:10])
    print(f"SPOT_DONE checked={min(len(hits), MAX)} differing={bad}", flush=True)


if __name__ == "__main__":
    main()
