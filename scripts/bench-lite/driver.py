#!/usr/bin/env python3
"""bench-lite: Quod against plain on a generated fixture, every job both ways.

    driver.py --quod BIN --out DIR [--json FILE] [--md FILE]

For each job of fixture.py's history (jobs.json), interleaved on the same
machine (the order alternates per job):

  plain   a fresh `git clone --shared` checkout at a constant path (new
          mtimes, as actions/checkout gives), then the steps with the env
          Quod's profiles give them, a fresh HOME and a GOCACHE that persists
          across jobs (plain-warm: a perfect actions/cache, as 0b/0c).
  quod    one persistent checkout and state dir; the steps through
          `quod run` (keyed steps `--mode replay`, Go `--warm`, the shipped
          trust defaults), Node's test step again through `quod step` in a
          small rootfs, then `quod audit` for the audits the replays deferred
          (QUOD_AUDIT_SPAWN=0 makes them the driver's, timed apart).

Steps (STEPS below): py-deps (pip --no-index from the repo's wheel dir) and
net-fetch (a download from a loopback HTTP server) are dependency steps,
`--allow-net`, never memoized; go-test, node-test and py-test are keyed.

Correctness: every Quod result of a keyed step is compared with the plain
run of the same job (same commit, same tree), stdout and exit code, after
normalizing durations. A replay that differs is a wrong replay; any makes
the driver exit 1. Misses that differ are reported too (an execution that
differs from plain is not a wrong replay, but it is not expected either).
"""
import argparse
import functools
import http.server
import json
import os
import re
import shutil
import socketserver
import statistics as st
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fixture  # noqa: E402

GO, NODE, PY = "/usr/local/go/bin", "/usr/local/node/bin", "/usr/local/python/bin"

# name, toolchain, kind, script. The scripts run from the repo's top level.
STEPS = [
    ("py-deps", "python", "deps",
     f"{PY}/python3 -m venv --without-pip --clear .quod/venv && "
     f"{PY}/python3 -m pip --python .quod/venv/bin/python install -q --no-index "
     "--find-links py/wheels --require-hashes -r py/requirements.lock"),
    ("net-fetch", "python", "net", f"{PY}/python3 tools/fetch.py"),
    ("go-test", "go", "keyed", "cd go && go test -v ./... 2>&1"),
    ("node-test", "node", "keyed", "cd node && node --test 2>&1"),
    ("py-test", "python", "keyed", "cd py && python -m unittest discover -s tests -t . -v 2>&1"),
]
KEYED = [s[0] for s in STEPS if s[2] == "keyed"]

# What each step sees on the plain side: Quod's profile env (quod_key::profile)
# with the checkout's path for /workspace.
def plain_env(tc, ws, home, gocache):
    if tc == "go":
        return {"CGO_ENABLED": "0", "GOCACHE": gocache, "GOFLAGS": "-mod=mod", "GOMODCACHE": f"{ws}/.quod/gomod",
                "GOPATH": f"{ws}/.quod/gopath", "GOPROXY": "off", "GOTOOLCHAIN": "local", "HOME": home,
                "LANG": "C.UTF-8", "PATH": f"{GO}:/usr/bin:/bin", "TZ": "UTC", "GOTELEMETRY": "off"}
    if tc == "node":
        return {"CI": "true", "HOME": home, "LANG": "C.UTF-8", "PATH": f"{NODE}:/usr/bin:/bin", "TZ": "UTC"}
    return {"CI": "true", "HOME": home, "LANG": "C.UTF-8", "PATH": f"{ws}/.quod/venv/bin:{PY}:/usr/bin:/bin",
            "PIP_DISABLE_PIP_VERSION_CHECK": "1", "PIP_NO_INPUT": "1", "PYTHONDONTWRITEBYTECODE": "1",
            "PYTHONHASHSEED": "0", "PYTHONNOUSERSITE": "1", "SOURCE_DATE_EPOCH": "315532800", "TZ": "UTC",
            "VIRTUAL_ENV": f"{ws}/.quod/venv", "PIP_CACHE_DIR": f"{home}/pip-cache"}


NORMALIZE = [
    (re.compile(r"\(cached\)"), "T"),                               # go: (cached)
    (re.compile(r"\b\d+(?:\.\d+)?(?:ms|s)\b"), "T"),                  # go, unittest, node spec: 0.12s, 38.1ms
    (re.compile(r"duration_ms:? [\d.]+"), "duration_ms T"),           # node TAP and spec summary
    (re.compile(r"\((node|npm):\d+\)"), r"(\1:PID)"),                  # node's process warnings
]


def normalize(text):
    for rx, sub in NORMALIZE:
        text = rx.sub(sub, text)
    return text


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], check=True, text=True, capture_output=True).stdout.strip()


class Bench:
    def __init__(self, args):
        self.quod = os.path.abspath(args.quod)
        self.out = os.path.abspath(args.out)
        self.rows = []
        self.logs = f"{self.out}/logs"
        self.src = f"{self.out}/src"
        self.ws = f"{self.out}/plain/ws"
        self.repo = f"{self.out}/q/repo"
        self.state = f"{self.out}/q/state"
        self.gocache = f"{self.out}/plain/gocache"
        self.rootfs = None
        self.with_step = not args.no_step

    def row(self, **r):
        self.rows.append(r)
        with open(f"{self.out}/rows.jsonl", "a") as f:
            f.write(json.dumps(r) + "\n")
        return r

    # -- setup ---------------------------------------------------------------
    def setup(self):
        if os.path.exists(self.out):
            shutil.rmtree(self.out)
        os.makedirs(self.logs)
        self.jobs = fixture.build(self.out)
        class Quiet(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *a):
                pass
        handler = functools.partial(Quiet, directory=f"{self.out}/registry")
        socketserver.TCPServer.allow_reuse_address = True
        self.httpd = socketserver.ThreadingTCPServer(("127.0.0.1", fixture.PORT), handler)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        subprocess.run(["git", "clone", "-q", self.src, self.repo], check=True)
        with open(f"{self.repo}/.git/info/exclude", "a") as f:
            f.write(".quod/\n")
        os.makedirs(self.gocache)
        if self.with_step:
            self.rootfs = self.mini_rootfs()

    def mini_rootfs(self):
        """A rootfs holding sh and node with their libraries, as the step
        tests' mini_rootfs builds it; imported once, not timed with the jobs."""
        d = f"{self.out}/rootfs"
        os.makedirs(d)
        for top in ("bin", "sbin", "lib", "lib64", "lib32"):
            if os.path.islink(f"/{top}"):
                os.symlink(os.readlink(f"/{top}"), f"{d}/{top}")

        def place(host):
            parts = host.strip("/").split("/")
            first = f"/{parts[0]}"
            base = os.path.join(d, os.readlink(first).lstrip("/")) if os.path.islink(first) else os.path.join(d, parts[0])
            return os.path.join(base, *parts[1:])

        def bring(host):
            real = os.path.realpath(host)
            dest = place(real)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            if not os.path.exists(dest):
                shutil.copy2(real, dest)
            if os.path.islink(host):
                link = place(host)
                os.makedirs(os.path.dirname(link), exist_ok=True)
                if not os.path.lexists(link):
                    os.symlink(os.readlink(host), link)

        for b in ("/bin/sh", f"{NODE}/node"):
            bring(b)
            for line in subprocess.run(["ldd", b], capture_output=True, text=True).stdout.splitlines():
                lib = (line.split("=>")[1] if "=>" in line else line).split()
                if lib and lib[0].startswith("/"):
                    bring(lib[0])
        for rel, text in (("etc/os-release", "ID=quod-bench-lite\n"), ("etc/passwd", "root:x:0:0:root:/root:/bin/sh\n"),
                          ("etc/group", "root:x:0:\n")):
            os.makedirs(os.path.dirname(f"{d}/{rel}"), exist_ok=True)
            with open(f"{d}/{rel}", "w") as f:
                f.write(text)
        for sub in ("dev", "proc", "sys", "tmp", "workspace", "root"):
            os.makedirs(f"{d}/{sub}", exist_ok=True)
        t = time.monotonic()
        p = subprocess.run([self.quod, "--state", self.state, "rootfs", "import", d, "--source", "bench-lite-node"],
                           capture_output=True, text=True, check=True)
        digest = p.stdout.split()[0]
        self.row(series="setup", step="rootfs import", wall=time.monotonic() - t, digest=digest)
        return digest

    # -- one step --------------------------------------------------------------
    def run(self, argv, log, cwd=None, env=None):
        t = time.monotonic()
        with open(log + ".out", "wb") as o, open(log + ".err", "wb") as e:
            rc = subprocess.run(argv, cwd=cwd, env=env, stdout=o, stderr=e).returncode
        wall = time.monotonic() - t
        with open(log + ".out", "rb") as f:
            out = f.read().decode(errors="replace")
        return rc, wall, out

    def events_since(self, n):
        ev = f"{self.state}/events.jsonl"
        if not os.path.exists(ev):
            return [], 0
        lines = open(ev).read().splitlines()
        return [json.loads(x) for x in lines[n:]], len(lines)

    def n_events(self):
        ev = f"{self.state}/events.jsonl"
        return sum(1 for _ in open(ev)) if os.path.exists(ev) else 0

    def plain_job(self, j):
        if os.path.exists(self.ws):
            shutil.rmtree(self.ws)
        home = f"{self.out}/plain/home"
        if os.path.exists(home):
            shutil.rmtree(home)
        os.makedirs(home)
        t0 = time.monotonic()
        subprocess.run(["git", "clone", "-q", "--shared", "--no-checkout", self.src, self.ws], check=True)
        git(self.ws, "checkout", "-q", "-B", j["branch"], j["commit"])
        self.row(series="plain", i=j["i"], scenario=j["scenario"], step="checkout", wall=time.monotonic() - t0, exit=0)
        for name, tc, kind, script in STEPS:
            log = f"{self.logs}/{j['i']:02d}-plain-{name}"
            rc, wall, out = self.run(["/bin/sh", "-e", "-c", script], log, cwd=self.ws,
                                     env=plain_env(tc, self.ws, home, self.gocache))
            self.row(series="plain", i=j["i"], scenario=j["scenario"], step=name, kind=kind, wall=wall, exit=rc,
                     out=normalize(out) if kind == "keyed" else None, log=log)

    def quod_job(self, j):
        t0 = time.monotonic()
        git(self.repo, "checkout", "-q", "-f", "-B", j["branch"], j["commit"])
        git(self.repo, "clean", "-q", "-fdx", "-e", ".quod/")
        self.row(series="quod", i=j["i"], scenario=j["scenario"], step="checkout", wall=time.monotonic() - t0, exit=0)
        env = dict(os.environ, QUOD_AUDIT_SPAWN="0")
        for name, tc, kind, script in STEPS:
            argv = [self.quod, "--state", self.state, "run", "--repo", self.repo, "--commit", j["commit"],
                    "--toolchain", tc]
            if kind == "keyed":
                argv += ["--mode", "replay"] + (["--warm"] if tc == "go" else [])
            else:
                argv += ["--allow-net"]
            argv += ["--step", script]
            log = f"{self.logs}/{j['i']:02d}-quod-{name}"
            n = self.n_events()
            rc, wall, out = self.run(argv, log, env=env)
            evs, _ = self.events_since(n)
            run = [e for e in evs if e.get("message") == "run"]
            ev = run[-1] if run else {}
            self.row(series="quod", i=j["i"], scenario=j["scenario"], step=name, kind=kind, wall=wall, exit=rc,
                     outcome=self.outcome(kind, ev), out=normalize(out) if kind == "keyed" else None,
                     deferred_audit=ev.get("deferred_audit"), first_mismatch=ev.get("first_mismatch"),
                     phases={k: ev.get(k) for k in ("setup_ms", "import_ms", "lookup_ms", "mount_ms", "exec_ms",
                                                     "diff_ms", "record_ms", "total_ms")},
                     cmd=" ".join(argv), log=log)
        if self.rootfs:
            self.quod_step(j, env)
        log = f"{self.logs}/{j['i']:02d}-quod-audit"
        rc, wall, _ = self.run([self.quod, "--state", self.state, "audit"], log, env=env)
        err = open(log + ".err").read()
        self.row(series="quod", i=j["i"], scenario=j["scenario"], step="audit", kind="audit", wall=wall, exit=rc,
                 audits=len(re.findall(r"(?mi)^\S*\s*audit", err)), diverged=bool(re.search(r"(?i)diverg|suspect", err)),
                 log=log)

    def quod_step(self, j, env):
        req = {"image": self.rootfs, "workspace": {"dir": f"{self.repo}/node"},
               "cmd": [f"{NODE}/node", "--test"],
               "cwd": "/workspace", "toolchain": "node", "lookup": True,
               "env": [["CI", "true"], ["HOME", "/tmp"], ["LANG", "C.UTF-8"], ["PATH", f"{NODE}:/usr/bin:/bin"],
                       ["TZ", "UTC"]]}
        log = f"{self.logs}/{j['i']:02d}-quod-step-node-test"
        with open(log + ".request.json", "w") as f:
            json.dump(req, f)
        rc, wall, out = self.run([self.quod, "--state", self.state, "step", "--request", log + ".request.json",
                                  "--response", log + ".response.json"], log, env=env)
        try:
            resp = json.load(open(log + ".response.json"))
        except (OSError, ValueError):
            resp = {}
        trust = " ".join(resp.get("trust") or [])
        outcome = "replayed" if resp.get("replayed") else ("diverged" if "DIVERGENCE" in trust else "ran")
        self.row(series="quod-step", i=j["i"], scenario=j["scenario"], step="node-test", kind="keyed", wall=wall,
                 exit=rc, outcome=outcome, out=normalize(out), log=log)

    @staticmethod
    def outcome(kind, ev):
        if kind != "keyed":
            return "net"
        if ev.get("hit"):
            return "replayed"
        if ev.get("outcome") in ("audit", "diverged"):
            return ev["outcome"]
        return "ran" if ev else "error"

    # -- the run ---------------------------------------------------------------
    def go(self):
        self.setup()
        self.row(series="meta", quod=self.quod, version=subprocess.run([self.quod, "--version"], capture_output=True,
                                                                        text=True).stdout.strip(),
                 host=os.uname().nodename, kernel=os.uname().release, arch=os.uname().machine, cpus=os.cpu_count(),
                 t=time.time())
        for j in self.jobs:
            self.row(series="job", **j)
            order = ("plain", "quod") if j["i"] % 2 == 0 else ("quod", "plain")
            for s in order:
                (self.plain_job if s == "plain" else self.quod_job)(j)
            self.check(j)
            print(f"[{time.strftime('%H:%M:%S')}] job {j['i'] + 1}/{len(self.jobs)} {j['scenario']} done", flush=True)
        self.httpd.shutdown()

    def check(self, j):
        """Every keyed Quod result against the plain run of the same job."""
        plain = {r["step"]: r for r in self.rows if r.get("series") == "plain" and r.get("i") == j["i"]}
        for r in self.rows:
            if r.get("i") != j["i"] or r.get("series") not in ("quod", "quod-step") or r.get("kind") != "keyed":
                continue
            p = plain[r["step"]]
            same = p["exit"] == r["exit"] and p["out"] == r["out"]
            r["matches_plain"] = same
            r["wrong_replay"] = r["outcome"] == "replayed" and not same
            if not same:
                print(f"  MISMATCH job {j['i']} {r['series']} {r['step']} ({r['outcome']}): exit {r['exit']} vs plain "
                      f"{p['exit']}; logs {r['log']}.out, {p['log']}.out", flush=True)
        if any(r.get("i") == j["i"] and r.get("step") in [s[0] for s in STEPS] and r.get("exit") != 0
               for r in self.rows if r.get("series") == "plain"):
            print(f"  plain job {j['i']} had a failing step", flush=True)


# -- summary ---------------------------------------------------------------------

def summarize(rows):
    meta = next((r for r in rows if r.get("series") == "meta"), {})
    jobs = [r for r in rows if r.get("series") == "job"]
    out = {"meta": meta, "scenarios": [], "steps": {}, "totals": {}}
    steps = [s[0] for s in STEPS]
    for j in jobs:
        i = j["i"]
        pr = [r for r in rows if r.get("series") == "plain" and r.get("i") == i and r["step"] in steps]
        qr = [r for r in rows if r.get("series") == "quod" and r.get("i") == i and r["step"] in steps]
        sr = [r for r in rows if r.get("series") == "quod-step" and r.get("i") == i]
        au = [r for r in rows if r.get("series") == "quod" and r.get("i") == i and r["step"] == "audit"]
        keyed = [r for r in qr if r["kind"] == "keyed"]
        out["scenarios"].append(dict(
            i=i, scenario=j["scenario"], branch=j["branch"], commit=j["commit"], note=j["note"],
            plain_s=round(sum(r["wall"] for r in pr), 2), quod_s=round(sum(r["wall"] for r in qr), 2),
            audit_s=round(sum(r["wall"] for r in au), 2),
            replayed=sum(r["outcome"] == "replayed" for r in keyed), ran=sum(r["outcome"] in ("ran", "error") for r in keyed),
            audited=sum(r["outcome"] in ("audit", "diverged") for r in keyed) + sum(1 for r in keyed if r.get("deferred_audit")),
            diverged=sum(r["outcome"] == "diverged" for r in keyed + sr),
            outcomes={r["step"]: r["outcome"] for r in keyed},
            step_outcome=sr[0]["outcome"] if sr else None, step_s=round(sr[0]["wall"], 2) if sr else None,
            wrong=sum(bool(r.get("wrong_replay")) for r in keyed + sr),
            mismatched=sum(r.get("matches_plain") is False for r in keyed + sr),
            failed=sum(r["exit"] != 0 for r in pr + qr)))
    for name in steps + ["node-test (quod step)"]:
        series, step = ("quod-step", "node-test") if name.endswith("(quod step)") else ("quod", name)
        q = {r["i"]: r for r in rows if r.get("series") == series and r.get("step") == step}
        p = {r["i"]: r for r in rows if r.get("series") == "plain" and r.get("step") == step}
        miss = [i for i, r in q.items() if r.get("outcome") != "replayed" and i in p and i > 0]
        hit = [i for i, r in q.items() if r.get("outcome") == "replayed" and i in p]
        if not q:
            continue
        over = [q[i]["wall"] - p[i]["wall"] for i in miss]
        pct = [100 * (q[i]["wall"] - p[i]["wall"]) / p[i]["wall"] for i in miss if p[i]["wall"] > 0]
        out["steps"][name] = dict(
            runs=len(q), replayed=len(hit), executed=len(miss),
            diverged=sum(r.get("outcome") == "diverged" for r in q.values()),
            plain_median_s=round(st.median(r["wall"] for r in p.values()), 3) if p else None,
            miss_overhead_median_s=round(st.median(over), 3) if over else None,
            miss_overhead_median_pct=round(st.median(pct), 1) if pct else None,
            replay_median_s=round(st.median(q[i]["wall"] for i in hit), 3) if hit else None)
    sc = [s for s in out["scenarios"] if s["i"] > 0]
    out["totals"] = dict(
        jobs=len(out["scenarios"]), plain_s=round(sum(s["plain_s"] for s in sc), 1),
        quod_s=round(sum(s["quod_s"] for s in sc), 1), audit_s=round(sum(s["audit_s"] for s in sc), 1),
        replayed=sum(s["replayed"] for s in sc), ran=sum(s["ran"] for s in sc), audited=sum(s["audited"] for s in sc),
        diverged=sum(s["diverged"] for s in sc),
        wrong_replays=sum(s["wrong"] for s in out["scenarios"]),
        mismatches=sum(s["mismatched"] for s in out["scenarios"]),
        failed_steps=sum(s["failed"] for s in out["scenarios"]),
        note="totals exclude job 0 (init, the warm-up)")
    t = out["totals"]
    t["quod_over_plain"] = round(t["quod_s"] / t["plain_s"], 3) if t["plain_s"] else None
    return out


def table(s, base=None):
    """Markdown: the scenario table, the per-step overheads, and with `base`
    (another run's JSON) the deltas against it."""
    t = s["totals"]
    m = s["meta"]
    b = {x["scenario"] + str(x["i"]): x for x in (base or {}).get("scenarios", [])}
    L = [f"bench-lite: {t['jobs']} jobs, quod {m.get('version', '?')} on {m.get('host', '?')} "
         f"({m.get('arch', '?')}, {m.get('cpus', '?')} CPUs, kernel {m.get('kernel', '?')})", "",
         "| # | scenario | plain s | quod s | quod/plain | replayed | ran | audited | quod step | wrong |"
         + (" Δ quod s |" if base else ""),
         "|---|---|---:|---:|---:|---:|---:|---:|---|---:|" + ("---:|" if base else "")]
    for x in s["scenarios"]:
        ratio = x["quod_s"] / x["plain_s"] if x["plain_s"] else 0
        d = ""
        if base:
            o = b.get(x["scenario"] + str(x["i"]))
            d = f" {x['quod_s'] - o['quod_s']:+.2f} |" if o else " - |"
        L.append(f"| {x['i']} | {x['scenario']} | {x['plain_s']:.2f} | {x['quod_s']:.2f} | {ratio:.2f}x | "
                 f"{x['replayed']} | {x['ran']} | {x['audited']} | {x['step_outcome'] or '-'} "
                 f"{x['step_s'] if x['step_s'] is not None else ''} | {x['wrong']} |" + d)
    L += ["", f"Totals (jobs 1-{t['jobs'] - 1}): plain {t['plain_s']} s, quod {t['quod_s']} s "
          f"(**{t['quod_over_plain']}x**), deferred audits {t['audit_s']} s after the jobs; keyed steps "
          f"{t['replayed']} replayed, {t['ran']} ran, {t['audited']} audited ({t['diverged']} audits diverged, "
          f"quod step included); "
          f"**wrong replays {t['wrong_replays']}**, results differing from plain {t['mismatches']}, "
          f"failed steps {t['failed_steps']}."]
    if base:
        bt = base["totals"]
        bm = base.get("meta", {})
        if (bm.get("arch"), bm.get("cpus")) != (m.get("arch"), m.get("cpus")):
            L.append(f"(The baseline ran on another host class: {bm.get('arch')}, {bm.get('cpus')} CPUs, "
                     f"{bm.get('host')}; compare the replay counts and outcomes, not the seconds.)")
        L.append(f"Against the baseline ({base['meta'].get('version', '?')}): quod {bt['quod_s']} -> {t['quod_s']} s "
                 f"({t['quod_s'] - bt['quod_s']:+.1f} s), quod/plain {bt['quod_over_plain']} -> {t['quod_over_plain']}, "
                 f"replayed {bt['replayed']} -> {t['replayed']}.")
    L += ["", "| step | runs | replayed | executed | diverged | plain median s | quod overhead per miss (median) | replay median s |"
          + (" baseline overhead |" if base else ""),
          "|---|---:|---:|---:|---:|---:|---:|---:|" + ("---:|" if base else "")]
    for name, x in s["steps"].items():
        ov = "-" if x["miss_overhead_median_s"] is None else f"{x['miss_overhead_median_s']:+.2f} s ({x['miss_overhead_median_pct']:+.0f}%)"
        bo = ""
        if base:
            y = base["steps"].get(name, {})
            bo = " - |" if y.get("miss_overhead_median_s") is None else f" {y['miss_overhead_median_s']:+.2f} s |"
        L.append(f"| {name} | {x['runs']} | {x['replayed']} | {x['executed']} | {x['diverged']} | {x['plain_median_s']} | {ov} | "
                 f"{x['replay_median_s'] if x['replay_median_s'] is not None else '-'} |" + bo)
    return "\n".join(L) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--quod", required=False)
    ap.add_argument("--out", required=False)
    ap.add_argument("--json", help="write the summary JSON here")
    ap.add_argument("--md", help="write the markdown table here")
    ap.add_argument("--against", help="a summary JSON to print deltas against")
    ap.add_argument("--no-step", action="store_true", help="skip the `quod step` series")
    ap.add_argument("--summarize", help="only summarize this rows.jsonl")
    a = ap.parse_args()
    if a.summarize:
        rows = [json.loads(x) for x in open(a.summarize)]
    else:
        b = Bench(a)
        b.go()
        rows = b.rows
    s = summarize(rows)
    base = json.load(open(a.against)) if a.against else None
    md = table(s, base)
    print(md)
    if a.json:
        with open(a.json, "w") as f:
            json.dump(s, f, indent=1)
    if a.md:
        with open(a.md, "w") as f:
            f.write(md)
    return 1 if s["totals"]["wrong_replays"] else 0


if __name__ == "__main__":
    sys.exit(main())
