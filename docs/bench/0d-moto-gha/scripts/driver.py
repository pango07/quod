#!/usr/bin/env python3
"""Benchmark driver: for each mirrored commit (mmirror.txt, 1-based), push it
to the demo repo's main (which runs quod.yml), wait for that run, then
dispatch plain.yml on main and wait for it. One workflow at a time, so the two
runners never share the machine. Records runs, jobs and logs under data/.

  driver.py FROM TO
"""
import json, subprocess, sys, time, pathlib, datetime as dt, gzip

REPO = "pango07/quod-demo-moto"
HERE = pathlib.Path(__file__).resolve().parent.parent
DATA = HERE / "data"
DATA.mkdir(exist_ok=True)


def gh(*args, check=True):
    for attempt in range(8):
        try:
            r = subprocess.run(["gh", *args], capture_output=True, text=True, timeout=120)
        except subprocess.TimeoutExpired:
            time.sleep(15); continue
        if not r.returncode or not check:
            return r.stdout
        if "HTTP 4" in r.stderr and "HTTP 429" not in r.stderr:
            break
        time.sleep(15 * (attempt + 1))
    raise RuntimeError(f"gh {' '.join(args)}: {r.stderr.strip()}")


def api(path):
    return json.loads(gh("api", path))


def log(msg):
    print(f"{dt.datetime.now().strftime('%H:%M:%S')} {msg}", flush=True)


def find_run(workflow, sha, event, after):
    for _ in range(120):
        runs = api(f"repos/{REPO}/actions/workflows/{workflow}/runs?per_page=10")["workflow_runs"]
        for r in runs:
            if r["head_sha"] == sha and r["event"] == event and r["created_at"] >= after:
                return r
        time.sleep(5)
    raise RuntimeError(f"no {workflow} run for {sha}")


def wait_run(rid):
    while True:
        r = api(f"repos/{REPO}/actions/runs/{rid}")
        if r["status"] == "completed":
            return r
        time.sleep(15)


def jobs(rid):
    out, page = [], 1
    while True:
        j = api(f"repos/{REPO}/actions/runs/{rid}/jobs?per_page=100&page={page}")
        out += j["jobs"]
        if len(out) >= j["total_count"] or not j["jobs"]:
            return out
        page += 1


def save(i, kind, run):
    js = jobs(run["id"])
    d = DATA / f"{i:02d}"
    d.mkdir(exist_ok=True)
    (d / f"{kind}-run.json").write_text(json.dumps(run, indent=1))
    (d / f"{kind}-jobs.json").write_text(json.dumps(js, indent=1))
    for j in js:
        lg = b""
        for _ in range(6):
            try:
                rr = subprocess.run(["gh", "api", f"repos/{REPO}/actions/jobs/{j['id']}/logs"],
                                    capture_output=True, timeout=180)
                if rr.returncode == 0 and rr.stdout:
                    lg = rr.stdout; break
            except subprocess.TimeoutExpired:
                pass
            time.sleep(20)
        with gzip.open(d / f"{kind}-{j['name']}.log.gz", "wb") as f:
            f.write(lg)
    return js


def secs(a, b):
    p = lambda s: dt.datetime.fromisoformat(s.replace("Z", "+00:00"))
    return (p(b) - p(a)).total_seconds()


def main():
    lo, hi = int(sys.argv[1]), int(sys.argv[2])
    mirror = [l.split() for l in (HERE / "mmirror.txt").read_text().splitlines()]
    for i in range(lo, hi + 1):
        sha, up = mirror[i - 1]
        # Plain and quod on separate hosts of the same type: start both, wait for both.
        now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        subprocess.run([str(HERE / "bin" / "push-m"), str(i)], check=True)
        q = find_run("quod.yml", sha, "push", now)
        gh("workflow", "run", "plain.yml", "-R", REPO, "--ref", "main")
        p = find_run("plain.yml", sha, "workflow_dispatch", now)
        log(f"{i} {sha[:8]} (upstream {up[:8]}) quod run {q['id']} plain run {p['id']}")
        q = wait_run(q["id"])
        p = wait_run(p["id"])
        qj = save(i, "quod", q)
        pj = save(i, "plain", p)
        qs = secs(q["run_started_at"], q["updated_at"])
        ps = secs(p["run_started_at"], p["updated_at"])
        bad = [j["name"] for j in qj + pj if j["conclusion"] != "success"]
        log(f"{i} done: quod {qs:.0f}s ({q['conclusion']}), plain {ps:.0f}s ({p['conclusion']})"
            + (f"; not success: {bad}" if bad else ""))


if __name__ == "__main__":
    main()
