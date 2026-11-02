#!/usr/bin/env python3
"""Test-level replay on real repositories (docs/test-level.md, "Measured").

    testlevel.py --quod BIN --out DIR --corpus attrs|moto-ec2|moto-de [--n N]

Per corpus, the N consecutive first-parent commits ending at its pin,
oldest first, all of them. Per commit, interleaved (the order alternates):

  plain  a fresh `git clone --shared` checkout, the setup step (the
         dependency install, untimed), then the test step with the env
         Quod's Python profile gives it, warm uv cache: pytest alone.
  quod   one job per commit in one state dir: checkout, step A
         (`--allow-net`, untimed), then the test step `quod run --mode
         replay` with the shipped trust defaults, at test level.

The test step is the corpus's pytest command plus `-rA -p no:cacheprovider`
on both sides, so every test's outcome is in the output. Correctness, per
commit: the exit code and pytest's summary counts must equal plain's, and
every test Quod replayed (in any session of its output) must have passed
(or xfailed) in the plain run of the same tree: the plain run re-runs every
replayed test. A replayed test that did not is a wrong replay; the driver
exits 1 if there is any.

Rows: DIR/rows.jsonl. The summary: DIR/summary.md.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time

PY, UV = "/usr/local/python/bin", "/usr/local/uv/bin"
FLAGS = " -rA -p no:cacheprovider"

CORPORA = {
    "attrs": dict(
        url="https://github.com/python-attrs/attrs",
        pin="9d9f94a043de7fc4012459853a4ed597a5577855",
        setup="uv sync --frozen",
        step="python -m pytest",
        env={},
    ),
    # docs/bench/0d-moto-gha.md's window: 6bae955b..3a9a6f30, 15 commits.
    "moto-ec2": dict(
        url="https://github.com/getmoto/moto",
        pin="3a9a6f30c",
        setup='uv venv -q --allow-existing "$VIRTUAL_ENV" && uv pip install -q -e ".[all,server]" -r requirements-tests.txt',
        step="python -m pytest -n 4 --dist loadscope tests/test_ec2",
        env={"TESTS_SKIP_REQUIRES_DOCKER": "true", "MOTO_CALL_RESET_API": "false"},
    ),
    "moto-de": dict(
        url="https://github.com/getmoto/moto",
        pin="3a9a6f30c",
        setup='uv venv -q --allow-existing "$VIRTUAL_ENV" && uv pip install -q -e ".[all,server]" -r requirements-tests.txt',
        step="python -m pytest tests/test_d* tests/test_e* --ignore=tests/test_dynamodb --ignore=tests/test_ec2",
        env={"TESTS_SKIP_REQUIRES_DOCKER": "true"},
    ),
}


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], check=True, text=True, capture_output=True).stdout.strip()


def plain_env(ws, home, uvcache, extra):
    e = {"CI": "true", "HOME": home, "LANG": "C.UTF-8", "PATH": f"{ws}/.venv/bin:{PY}:{UV}:/usr/bin:/bin",
         "PYTHONDONTWRITEBYTECODE": "1", "PYTHONHASHSEED": "0", "PYTHONNOUSERSITE": "1",
         "SOURCE_DATE_EPOCH": "315532800", "TZ": "UTC", "UV_CACHE_DIR": uvcache, "UV_COMPILE_BYTECODE": "1",
         "UV_FROZEN": "1", "UV_LINK_MODE": "copy", "UV_NO_PROGRESS": "1", "UV_PROJECT_ENVIRONMENT": f"{ws}/.venv",
         "UV_PYTHON": f"{PY}/python3", "UV_PYTHON_DOWNLOADS": "never", "UV_PYTHON_PREFERENCE": "only-system",
         "VIRTUAL_ENV": f"{ws}/.venv"}
    e.update(extra)
    return e


WORDS = ("PASSED", "FAILED", "ERROR", "XFAIL", "XPASS")


def outcomes(text):
    """Per test, from -rA's short summary of the last session: outcome word."""
    i = text.rfind("short test summary info")
    out = {}
    for l in (text[i:] if i >= 0 else text).splitlines():
        w, _, rest = l.partition(" ")
        if w not in WORDS:
            continue
        rest = rest.removeprefix("(replayed) ")
        out[rest.split(" - ")[0].strip()] = w
    return out


def replayed(text):
    return sorted({l[len("PASSED (replayed) "):].strip() for l in text.splitlines()
                   if l.startswith("PASSED (replayed) ")})


def summary(text):
    for l in reversed(text.splitlines()):
        if re.search(r"\d+ (passed|failed|error)", l):
            l = l.strip("= ")
            return re.sub(r" in [0-9.]+s.*$", "", l)
    return ""


class Bench:
    def __init__(self, a):
        self.quod, self.out, self.n = os.path.abspath(a.quod), os.path.abspath(a.out), a.n
        self.name = a.corpus
        self.cfg = CORPORA[a.corpus]
        os.makedirs(f"{self.out}/logs", exist_ok=True)
        self.rows_path = f"{self.out}/rows.jsonl"
        self.rows = []

    def row(self, **r):
        r["t"] = time.time()
        self.rows.append(r)
        with open(self.rows_path, "a") as f:
            f.write(json.dumps(r) + "\n")
        return r

    def run(self, argv, log, cwd=None, env=None):
        t = time.monotonic()
        with open(log + ".out", "wb") as o, open(log + ".err", "wb") as e:
            rc = subprocess.run(argv, cwd=cwd, env=env, stdout=o, stderr=e).returncode
        wall = time.monotonic() - t
        return rc, wall, open(log + ".out", "rb").read().decode(errors="replace"), \
            open(log + ".err", "rb").read().decode(errors="replace")

    def prepare(self):
        root = f"{self.out}/{self.name}"
        self.src, self.ws, self.repo = f"{root}/src", f"{root}/plain/ws", f"{root}/quod/repo"
        self.state, self.home, self.uv = f"{root}/quod/state", f"{root}/plain/home", f"{root}/plain/uv-cache"
        for d in ("plain", "quod"):
            shutil.rmtree(f"{root}/{d}", ignore_errors=True)
        if not os.path.isdir(f"{self.src}/.git"):
            subprocess.run(["git", "clone", "-q", self.cfg["url"], self.src], check=True)
        pin = git(self.src, "rev-parse", self.cfg["pin"])
        self.commits = git(self.src, "log", "--first-parent", "--reverse", "-n", str(self.n), "--format=%H",
                           pin).split()
        subprocess.run(["git", "clone", "-q", self.src, self.repo], check=True)
        with open(f"{self.repo}/.git/info/exclude", "a") as f:
            f.write(".quod/\n")
        os.makedirs(self.uv, exist_ok=True)

    def plain(self, i, c):
        shutil.rmtree(self.ws, ignore_errors=True)
        shutil.rmtree(self.home, ignore_errors=True)
        os.makedirs(self.home)
        subprocess.run(["git", "clone", "-q", "--shared", "--no-checkout", self.src, self.ws], check=True)
        git(self.ws, "checkout", "-q", "-B", "ci", c)
        env = plain_env(self.ws, self.home, self.uv, self.cfg["env"])
        log = f"{self.out}/logs/{i:02d}-plain-{c[:10]}"
        rc, _, _, err = self.run(["/bin/sh", "-e", "-c", self.cfg["setup"]], log + "-setup", cwd=self.ws, env=env)
        if rc:
            raise SystemExit(f"plain setup failed at {c}: {err[-2000:]}")
        rc, wall, text, _ = self.run(["/bin/sh", "-e", "-c", self.cfg["step"] + FLAGS], log, cwd=self.ws, env=env)
        return self.row(series="plain", i=i, commit=c, wall=wall, exit=rc, summary=summary(text),
                        outcomes=outcomes(text), log=log)

    def quod(self, i, c):
        git(self.repo, "checkout", "-q", "-f", "-B", "main", c)
        git(self.repo, "clean", "-q", "-fdx", "-e", ".quod/")
        env = dict(os.environ, QUOD_AUDIT_SPAWN="0")
        base = [self.quod, "--state", self.state, "run", "--repo", self.repo, "--commit", c, "--toolchain", "python"]
        keyed = [x for k, v in self.cfg["env"].items() for x in ("--env", f"{k}={v}")]
        log = f"{self.out}/logs/{i:02d}-quod-{c[:10]}"
        rc, _, _, err = self.run(base + ["--allow-net", "--step", self.cfg["setup"]], log + "-setup", env=env)
        if rc:
            raise SystemExit(f"quod setup failed at {c}: {err[-2000:]}")
        n = sum(1 for _ in open(f"{self.state}/events.jsonl"))
        rc, wall, text, err = self.run(base + keyed + ["--mode", "replay", "--step", self.cfg["step"] + FLAGS], log,
                                       env=env)
        evs = [json.loads(x) for x in open(f"{self.state}/events.jsonl").read().splitlines()[n:]]
        run = ([e for e in evs if e.get("message") == "run"] or [{}])[-1]
        tests = ([e for e in evs if e.get("message") == "tests"] or [{}])[-1]
        return self.row(series="quod", i=i, commit=c, wall=wall, exit=rc, summary=summary(text),
                        outcomes=outcomes(text), replayed=replayed(text), outcome=run.get("outcome"),
                        hit=run.get("hit"), tests_event={k: tests.get(k) for k in (
                            "collected", "replayed", "executed", "diverged", "planned", "known", "full",
                            "session_files", "wall_ms")},
                        rerun="running every test" in err, log=log)

    def check(self, p, q):
        wrong = [t for t in q["replayed"] if p["outcomes"].get(t) not in ("PASSED", "XFAIL")]
        if q.get("hit"):
            # A whole-step replay: its result is the plain run's if the
            # summary and exit code agree.
            pass
        differs = [t for t, w in q["outcomes"].items() if p["outcomes"].get(t) != w]
        same = p["exit"] == q["exit"] and p["summary"] == q["summary"]
        return dict(wrong_replays=wrong, outcome_differs=differs[:20], n_outcome_differs=len(differs),
                    matches_plain=same and not differs)

    def go(self):
        self.prepare()
        self.row(series="meta", corpus=self.name, cfg=self.cfg, commits=self.commits, n=self.n,
                 subjects=[git(self.src, "log", "-1", "--format=%s", c) for c in self.commits])
        wrong = 0
        for i, c in enumerate(self.commits):
            order = ("plain", "quod") if i % 2 == 0 else ("quod", "plain")
            got = {}
            for s in order:
                got[s] = self.plain(i, c) if s == "plain" else self.quod(i, c)
            v = self.check(got["plain"], got["quod"])
            wrong += len(v["wrong_replays"])
            self.row(series="check", i=i, commit=c, **v)
            te = got["quod"]["tests_event"]
            print(f"[{time.strftime('%H:%M:%S')}] {self.name} {i + 1}/{len(self.commits)} {c[:10]} "
                  f"plain {got['plain']['wall']:.1f}s quod {got['quod']['wall']:.1f}s "
                  f"({'whole-step replay' if got['quod'].get('hit') else te.get('executed')} ran, "
                  f"{te.get('replayed')} replayed) wrong {len(v['wrong_replays'])} "
                  f"match {v['matches_plain']}", flush=True)
        self.row(series="done", wrong_replays=wrong)
        return wrong


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--quod", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--corpus", required=True, choices=sorted(CORPORA))
    ap.add_argument("--n", type=int, default=20)
    a = ap.parse_args()
    wrong = Bench(a).go()
    print(f"testlevel: wrong replays {wrong}")
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())
