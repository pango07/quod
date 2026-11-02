#!/usr/bin/env python3
"""bench-real-small: Quod against plain on small, popular, real repositories
(one each for Go, Node and Python; versions.toml [bench_real.*]), on their
newest first-parent commits, chosen by rule only.

    driver.py --quod BIN --out DIR [--repos go,node,python] [--n 20]

Per repo, the N consecutive first-parent commits ending at the pinned
commit (`git log --first-parent -n N PIN`, oldest first). Nothing is picked
or skipped. Three series, interleaved per commit on one machine:

  plain    each distinct tree once: a fresh `git clone --shared` checkout
           at a constant path (new mtimes, as actions/checkout gives), the
           setup step (the dependency install) then the test step, with the
           env Quod's profile gives the step, a fresh HOME and warm caches
           that persist across commits (Go: GOCACHE and GOMODCACHE; Node:
           npm's cache; Python: uv's cache): plain-warm, a perfect
           actions/cache.
  history  Quod, one job per commit on `main`, as docs/bench/0b did:
           checkout, step A (`--allow-net`, never memoized), the test step
           `quod run --mode replay` (Go: `--warm`), shipped trust defaults.
  traffic  Quod in its own state dir, as docs/bench/trust-v2/traffic.py: per
           commit the pull request head (the merge's second parent for a
           merge commit, else the commit itself on branch `pr`), then the
           merge on `main`, then with probability RERUN (seeded, 0.10) a
           re-run of the merge job.

Before the timed passes, a prefetch pass runs every commit's setup step
once plainly and once through Quod (untimed), so the timed passes find
every dependency in the caches and no timed run waits on the network.

Correctness: every Quod test result is compared with the plain run of the
same tree (stdout and exit code; durations normalized as Quod normalizes
them). A replay that differs is a wrong replay, and the driver exits 1.
A tree no plain run covered yet (a PR head) is run plainly first.

Rows: DIR/rows.jsonl (one line per step run, every command and its log);
report.py turns them into the publishable report.
"""
import argparse
import json
import os
import platform
import random
import re
import shutil
import subprocess
import sys
import time

try:
    import tomllib
except ImportError:  # pragma: no cover
    tomllib = None

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "bench-lite"))
from driver import normalize  # noqa: E402  (bench-lite's driver.py: its normalization)

GO, NODE, PY, UV = "/usr/local/go/bin", "/usr/local/node/bin", "/usr/local/python/bin", "/usr/local/uv/bin"
RERUN = float(os.environ.get("RERUN", "0.10"))
SEED = int(os.environ.get("SEED", "42"))


def corpus():
    with open(os.path.join(ROOT, "versions.toml"), "rb") as f:
        v = tomllib.load(f)
    return v["bench_real"]


def plain_env(tc, ws, home, caches):
    if tc == "go":
        return {"CGO_ENABLED": "0", "GOCACHE": caches["gocache"], "GOFLAGS": "-mod=mod", "GOMODCACHE": caches["gomod"],
                "GOPATH": f"{caches['root']}/gopath", "GOPROXY": "off", "GOTOOLCHAIN": "local", "HOME": home,
                "LANG": "C.UTF-8", "PATH": f"{GO}:/usr/bin:/bin", "TZ": "UTC", "GOTELEMETRY": "off"}
    if tc == "node":
        return {"CI": "true", "HOME": home, "LANG": "C.UTF-8", "PATH": f"{NODE}:/usr/bin:/bin", "TZ": "UTC",
                "npm_config_audit": "false", "npm_config_cache": caches["npm"], "npm_config_fund": "false",
                "npm_config_update_notifier": "false", "npm_config_prefer_offline": "true"}
    return {"CI": "true", "HOME": home, "LANG": "C.UTF-8", "PATH": f"{ws}/.venv/bin:{PY}:{UV}:/usr/bin:/bin",
            "PYTHONDONTWRITEBYTECODE": "1", "PYTHONHASHSEED": "0", "PYTHONNOUSERSITE": "1",
            "SOURCE_DATE_EPOCH": "315532800", "TZ": "UTC", "UV_CACHE_DIR": caches["uv"], "UV_COMPILE_BYTECODE": "1",
            "UV_FROZEN": "1", "UV_LINK_MODE": "copy", "UV_NO_PROGRESS": "1", "UV_PROJECT_ENVIRONMENT": f"{ws}/.venv",
            "UV_PYTHON": f"{PY}/python3", "UV_PYTHON_DOWNLOADS": "never", "UV_PYTHON_PREFERENCE": "only-system",
            "VIRTUAL_ENV": f"{ws}/.venv"}


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], check=True, text=True, capture_output=True).stdout.strip()


def host_facts():
    cpu = ""
    try:
        for line in open("/proc/cpuinfo"):
            if line.startswith(("model name", "Model")):
                cpu = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    mem = 0
    try:
        mem = int(next(x for x in open("/proc/meminfo") if x.startswith("MemTotal")).split()[1]) // 1024
    except (OSError, StopIteration):
        pass
    return dict(host=platform.node(), kernel=platform.release(), arch=platform.machine(),
                cpus=len(os.sched_getaffinity(0)), cpus_total=os.cpu_count(),
                cpu_model=cpu, mem_mib=mem, runner=os.environ.get("RUNNER_NAME"),
                github_run=os.environ.get("GITHUB_RUN_ID"))


class Repo:
    def __init__(self, bench, name, cfg):
        self.b, self.name, self.cfg = bench, name, cfg
        self.tc = cfg["toolchain"]
        self.dir = f"{bench.out}/{name}"
        self.src = f"{self.dir}/src"
        self.ws = f"{self.dir}/plain/ws"
        self.caches = {"root": f"{self.dir}/plain", "gocache": f"{self.dir}/plain/gocache",
                       "gomod": f"{self.dir}/plain/gomod", "npm": f"{self.dir}/plain/npm-cache",
                       "uv": f"{self.dir}/plain/uv-cache"}
        self.plain_by_tree = {}
        self.pending = []

    def prepare(self):
        # Every run starts cold: no plain cache, Quod state or log of an
        # earlier run survives (only the pristine clone in src/ is reused).
        for sub in ("plain", "history", "traffic", "logs"):
            shutil.rmtree(f"{self.dir}/{sub}", ignore_errors=True)
        os.makedirs(f"{self.dir}/logs", exist_ok=True)
        for attempt in range(4):
            if os.path.isdir(f"{self.src}/.git"):
                break
            if attempt:
                time.sleep(15 * attempt)
            shutil.rmtree(self.src, ignore_errors=True)
            subprocess.run(["git", "clone", "-q", self.cfg["url"], self.src])
        else:
            raise SystemExit(f"bench-real: could not clone {self.cfg['url']}")
        try:
            git(self.src, "cat-file", "-e", self.cfg["commit"] + "^{commit}")
        except subprocess.CalledProcessError:
            git(self.src, "fetch", "-q", "origin", self.cfg["commit"])
        self.commits = git(self.src, "log", "--first-parent", "--reverse", "-n", str(self.b.n), "--format=%H",
                           self.cfg["commit"]).split()
        assert self.commits[-1] == self.cfg["commit"], "the pin must be the newest commit"
        for d in self.caches.values():
            os.makedirs(d, exist_ok=True)
        for series in ("history", "traffic"):
            repo = f"{self.dir}/{series}/repo"
            if not os.path.isdir(repo):
                subprocess.run(["git", "clone", "-q", self.src, repo], check=True)
                with open(f"{repo}/.git/info/exclude", "a") as f:
                    f.write(".quod/\n")

    def pr_head(self, c):
        try:
            return git(self.src, "rev-parse", "--verify", "-q", f"{c}^2")
        except subprocess.CalledProcessError:
            return c

    def tree(self, c):
        return git(self.src, "rev-parse", f"{c}^{{tree}}")

    # -- plain ---------------------------------------------------------------
    def plain(self, i, c, why, timed=True, steps=("setup", "step")):
        if os.path.exists(self.ws):
            shutil.rmtree(self.ws)
        home = f"{self.dir}/plain/home"
        if os.path.exists(home):
            shutil.rmtree(home)
        os.makedirs(home)
        t = time.monotonic()
        subprocess.run(["git", "clone", "-q", "--shared", "--no-checkout", self.src, self.ws], check=True)
        git(self.ws, "checkout", "-q", "-B", "ci", c)
        out = {}
        if timed:
            self.b.row(repo=self.name, series="plain", i=i, commit=c, why=why, step="checkout", wall=time.monotonic() - t)
        for s in steps:
            env = plain_env(self.tc, self.ws, home, self.caches)
            if s == "setup" and self.tc == "go":
                env["GOPROXY"] = "https://proxy.golang.org,direct"
            log = f"{self.dir}/logs/{i:02d}-plain-{c[:10]}-{s}" + ("" if timed else "-prefetch")
            rc, wall, text = self.b.run(["/bin/sh", "-e", "-c", self.cfg[s]], log, cwd=self.ws, env=env)
            # A stack trace names the checkout's path; Quod's step sees it
            # at /workspace.
            out[s] = (rc, normalize(text.replace(self.ws, "/workspace")))
            if timed:
                self.b.row(repo=self.name, series="plain", i=i, commit=c, tree=self.tree(c), why=why, step=s,
                           wall=wall, exit=rc, out_sha=_sha(out[s][1]), log=log)
        if timed:
            self.plain_by_tree[self.tree(c)] = out["step"]
        return out

    # -- quod ----------------------------------------------------------------
    def quod_job(self, series, i, phase, branch, c, timed=True, steps=("setup", "step")):
        repo, state = f"{self.dir}/{series}/repo", f"{self.dir}/{series}/state"
        t = time.monotonic()
        git(repo, "checkout", "-q", "-f", "-B", branch, c)
        git(repo, "clean", "-q", "-fdx", "-e", ".quod/")
        if timed:
            self.b.row(repo=self.name, series=series, phase=phase, i=i, commit=c, step="checkout",
                       wall=time.monotonic() - t)
        env = dict(os.environ, QUOD_AUDIT_SPAWN="0")
        for s in steps:
            argv = [self.b.quod, "--state", state, "run", "--repo", repo, "--commit", c, "--toolchain", self.tc]
            argv += ["--allow-net"] if s == "setup" else (["--mode", "replay"] + (["--warm"] if self.tc == "go" else []))
            argv += ["--step", self.cfg[s]]
            log = f"{self.dir}/logs/{i:02d}-{series}-{phase}-{c[:10]}-{s}" + ("" if timed else "-prefetch")
            n = _n_events(state)
            rc, wall, text = self.b.run(argv, log, env=env)
            ev = _run_event(state, n)
            if not timed:
                continue
            outcome = "net" if s == "setup" else ("replayed" if ev.get("hit") else
                                                  ev.get("outcome") if ev.get("outcome") in ("audit", "diverged")
                                                  else ("ran" if ev else "error"))
            r = self.b.row(repo=self.name, series=series, phase=phase, i=i, commit=c, tree=self.tree(c), step=s,
                           wall=wall, exit=rc, outcome=outcome, first_mismatch=ev.get("first_mismatch"),
                           deferred_audit=ev.get("deferred_audit"), cmd=" ".join(argv), log=log,
                           out_sha=_sha(normalize(text)) if s == "step" else None,
                           phases={k: ev.get(k) for k in ("setup_ms", "import_ms", "lookup_ms", "mount_ms", "exec_ms",
                                                          "diff_ms", "record_ms", "total_ms")})
            if s == "step":
                self.pending.append((r, rc, normalize(text)))
        if timed:
            log = f"{self.dir}/logs/{i:02d}-{series}-{phase}-{c[:10]}-audit"
            rc, wall, _ = self.b.run([self.b.quod, "--state", state, "audit"], log, env=env)
            err = open(log + ".err").read()
            self.b.row(repo=self.name, series=series, phase=phase, i=i, commit=c, step="audit", wall=wall, exit=rc,
                       diverged=bool(re.search(r"(?i)diverg|suspect", err)), log=log)

    def resolve(self):
        """After a commit's series: every Quod test result against a plain
        run of the same tree, running one for a tree no plain run covered."""
        for r, rc, text in self.pending:
            self.check(r, rc, text)
        self.pending = []

    def check(self, r, rc, text):
        tree = r["tree"]
        if tree not in self.plain_by_tree:
            self.plain(r["i"], r["commit"], "spot-check")
        prc, ptext = self.plain_by_tree[tree]
        same = prc == rc and ptext == text
        r["matches_plain"] = same
        r["wrong_replay"] = r["outcome"] == "replayed" and not same
        if not same:
            print(f"  MISMATCH {self.name} {r['series']} {r['phase']} {r['commit'][:10]} ({r['outcome']}), log {r['log']}",
                  flush=True)
        self.b.amend(r)


def _sha(text):
    import hashlib
    return hashlib.sha256(text.encode()).hexdigest()[:16]


def _n_events(state):
    ev = f"{state}/events.jsonl"
    return sum(1 for _ in open(ev)) if os.path.exists(ev) else 0


def _run_event(state, n):
    ev = f"{state}/events.jsonl"
    if not os.path.exists(ev):
        return {}
    evs = [json.loads(x) for x in open(ev).read().splitlines()[n:]]
    run = [e for e in evs if e.get("message") == "run"]
    return run[-1] if run else {}


class Bench:
    def __init__(self, a):
        self.quod, self.out, self.n = os.path.abspath(a.quod), os.path.abspath(a.out), a.n
        self.names = a.repos.split(",")
        self.rows_path = f"{self.out}/rows.jsonl"
        self.rows = []

    def row(self, **r):
        r["t"] = time.time()
        self.rows.append(r)
        with open(self.rows_path, "a") as f:
            f.write(json.dumps(r) + "\n")
        return r

    def amend(self, r):
        with open(self.rows_path, "a") as f:
            f.write(json.dumps(dict(r, amended=True)) + "\n")

    def run(self, argv, log, cwd=None, env=None):
        t = time.monotonic()
        with open(log + ".out", "wb") as o, open(log + ".err", "wb") as e:
            rc = subprocess.run(argv, cwd=cwd, env=env, stdout=o, stderr=e).returncode
        wall = time.monotonic() - t
        return rc, wall, open(log + ".out", "rb").read().decode(errors="replace")

    def go(self):
        os.makedirs(self.out, exist_ok=True)
        if os.path.exists(self.rows_path):
            os.rename(self.rows_path, self.rows_path + f".{int(time.time())}")
        cfgs = corpus()
        versions = {k: subprocess.run(v, capture_output=True, text=True).stdout.strip() for k, v in {
            "go": [f"{GO}/go", "version"], "node": [f"{NODE}/node", "--version"],
            "python": [f"{PY}/python3", "--version"], "uv": [f"{UV}/uv", "--version"],
            "git": ["git", "--version"], "quod": [self.quod, "--version"]}.items()}
        # From the host when the checkout is a worktree the container cannot resolve.
        quod_commit = os.environ.get("QUOD_COMMIT") or subprocess.run(
            ["git", "-c", "safe.directory=*", "-C", ROOT, "rev-parse", "HEAD"],
            capture_output=True, text=True).stdout.strip()
        self.row(series="meta", n=self.n, rerun=RERUN, seed=SEED, repos={k: cfgs[k] for k in self.names},
                 versions=versions, quod_commit=quod_commit, **host_facts())
        for name in self.names:
            r = Repo(self, name, cfgs[name])
            t0 = time.monotonic()
            r.prepare()
            self.row(repo=name, series="commits", commits=r.commits, subjects=[
                git(r.src, "log", "-1", "--format=%s", c) for c in r.commits], pr_heads=[r.pr_head(c) for c in r.commits])
            # Prefetch: every dependency into the caches, untimed.
            for c in r.commits:
                for cc in {r.pr_head(c), c}:
                    r.plain(-1, cc, "prefetch", timed=False, steps=("setup",))
                    r.quod_job("history", -1, "prefetch", "prefetch", cc, timed=False, steps=("setup",))
                    r.quod_job("traffic", -1, "prefetch", "prefetch", cc, timed=False, steps=("setup",))
            self.row(repo=name, series="prefetch", wall=time.monotonic() - t0)
            rng = random.Random(SEED)
            for i, c in enumerate(r.commits):
                head = r.pr_head(c)
                order = ["plain", "history", "traffic"]
                if i % 2:
                    order.reverse()
                for s in order:
                    if s == "plain":
                        r.plain(i, c, "merge")
                    elif s == "history":
                        r.quod_job("history", i, "merge", "main", c)
                    else:
                        r.quod_job("traffic", i, "pr", "pr", head)
                        r.quod_job("traffic", i, "merge", "main", c)
                        if rng.random() < RERUN:
                            r.quod_job("traffic", i, "rerun", "main", c)
                r.resolve()
                print(f"[{time.strftime('%H:%M:%S')}] {name} commit {i + 1}/{len(r.commits)} {c[:10]} done", flush=True)
            self.row(repo=name, series="repo-done", wall=time.monotonic() - t0)
        wrong = sum(1 for x in self.rows if x.get("wrong_replay"))
        self.row(series="done", wrong_replays=wrong)
        return wrong


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--quod", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--repos", default="go,node,python")
    ap.add_argument("--n", type=int, default=int(os.environ.get("N", "20")))
    a = ap.parse_args()
    wrong = Bench(a).go()
    print(f"bench-real: wrong replays {wrong}")
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())
