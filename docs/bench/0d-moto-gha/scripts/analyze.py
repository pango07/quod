#!/usr/bin/env python3
"""Tables from the driver's data/: per commit and per job, plain against Quod.

Writes data/rows.jsonl (one row per commit and job) and prints markdown.
"""
import gzip, json, pathlib, re, statistics as st, subprocess, sys, datetime as dt

HERE = pathlib.Path(__file__).resolve().parent.parent
DATA = HERE / "data"
MIRROR = [l.split() for l in (HERE / "mmirror.txt").read_text().splitlines()]
ANSI = re.compile(r"\x1b\[[0-9;]*m")
TS = re.compile(r"^\d{4}-\d\d-\d\dT[\d:.]+Z ")


def p(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def secs(a, b):
    return (p(b) - p(a)).total_seconds()


def lines(path):
    if not path.exists():
        return []
    t = gzip.open(path, "rt", errors="replace").read()
    return [TS.sub("", ANSI.sub("", l)) for l in t.splitlines()]


SUMMARY = re.compile(r"^=+ (.*\b(passed|failed|errors?|skipped|deselected|no tests ran)\b.*) in [\d.]+s(?: \([\d:]+\))? =+$")


def pytest_summaries(ls):
    """Each pytest run's summary line, durations removed ("12 passed, 3 skipped")."""
    out = []
    for l in ls:
        m = SUMMARY.match(l.strip())
        if m:
            out.append(m.group(1).strip())
    return out


REC = re.compile(r"recorded (\S+)\s+replay (\S+)\s+saved (\S+)")
RUN = re.compile(r"proved by run (\S+) on (.+)$")


def dur(s):
    """`1m41s`, `0.0s`, `2m05s` -> seconds."""
    m = re.fullmatch(r"(?:(\d+)m)?([\d.]+)s", s)
    return (int(m.group(1) or 0) * 60 + float(m.group(2))) if m else None


def test_step(ls):
    """The test step's outcome under Quod: ("replayed", recorded_s, replay_s,
    proved_by) or ("executed", why) — the step whose log group starts with
    `dirs=$(` or `.venv/bin/pytest`."""
    group, out = "", None
    for idx, l in enumerate(ls):
        if l.startswith("##[group]Run ") and "/hooks/index.js" not in l:
            group = l[len("##[group]Run "):]
        is_test = group.startswith("dirs=$(") or group.startswith(".venv/bin/pytest")
        if not is_test:
            continue
        if l.startswith("QUOD  REPLAYED"):
            body = " ".join(ls[idx + 1: idx + 5])
            r, p = REC.search(body), RUN.search(ls[idx + 1].strip())
            out = ("replayed", dur(r.group(1)) if r else None, dur(r.group(2)) if r else None,
                   p.group(1) if p else None)
        elif l.startswith("QUOD  EXECUTED  [executed] "):
            out = ("executed", l.split(": ok in ", 1)[-1] if ": ok in " in l else l)
    return out


def upstream_files(sha):
    r = subprocess.run(["git", "-C", str(HERE / "moto"), "diff-tree", "--no-commit-id", "--name-only", "-r", sha],
                       capture_output=True, text=True)
    return r.stdout.split()


PARALLEL = ["batch", "dynamodb", "ec2", "s3", "sqs"]
SHARDS = {"services a-b": "ab", "services c": "c", "services d-e": "de",
          "services f-m": "fghijklm", "services n-r": "nopqr", "services s-z": "stuvwxyz"}
GLOBAL_MOTO = {"core", "utilities", "moto_api", "moto_server", "moto_proxy", "packages"}


def job_of_service(svc):
    if svc in PARALLEL:
        return svc
    for name, letters in SHARDS.items():
        if svc[:1] in letters:
            return name
    return None


def path_filter(files):
    """The jobs a hand-written path filter would run: a service's job when
    moto/<svc>/ or tests/test_<svc>/ changes; every job when moto's shared code
    (core, utilities, ...), a top-level module of moto/ or tests/, or the
    packaging changes; none for anything else (docs, other_langs, .github)."""
    alljobs = set(PARALLEL) | set(SHARDS)
    out = set()
    for f in files:
        parts = f.split("/")
        if parts[0] in ("setup.cfg", "setup.py", "pyproject.toml") or parts[0].startswith("requirements"):
            return alljobs
        if parts[0] == "moto":
            if len(parts) == 2 or parts[1] in GLOBAL_MOTO:
                return alljobs
            j = job_of_service(parts[1])
            if j:
                out.add(j)
        elif parts[0] == "tests":
            if len(parts) == 2:
                return alljobs
            svc = parts[1][len("test_"):] if parts[1].startswith("test_") else None
            if svc in ("core", "utilities"):
                return alljobs
            j = job_of_service(svc) if svc else None
            if j:
                out.add(j)
    return out


def main():
    rows = []
    for d in sorted(DATA.iterdir()):
        if not d.is_dir() or not (d / "plain-jobs.json").exists():
            continue
        i = int(d.name)
        demo, up = MIRROR[i - 1]
        qrun = json.loads((d / "quod-run.json").read_text())
        prun = json.loads((d / "plain-run.json").read_text())
        qj = {j["name"]: j for j in json.loads((d / "quod-jobs.json").read_text())}
        pj = {j["name"]: j for j in json.loads((d / "plain-jobs.json").read_text())}
        files = upstream_files(up)
        pf = path_filter(files)
        for name in sorted(qj):
            q, pl = qj[name], pj.get(name)
            ql = lines(d / f"quod-{name}.log.gz")
            pll = lines(d / f"plain-{name}.log.gz")
            ts = test_step(ql)
            rows.append({
                "i": i, "demo": demo, "upstream": up, "job": name,
                "quod_s": secs(q["started_at"], q["completed_at"]),
                "plain_s": secs(pl["started_at"], pl["completed_at"]) if pl else None,
                "quod_conclusion": q["conclusion"], "plain_conclusion": pl["conclusion"] if pl else None,
                "quod_test": ts[0] if ts else None,
                "quod_test_detail": list(ts[1:]) if ts else None,
                "quod_pytest": pytest_summaries(ql), "plain_pytest": pytest_summaries(pll),
                "quod_job_id": q["id"], "plain_job_id": pl["id"] if pl else None,
                "quod_run": qrun["id"], "plain_run": prun["id"],
                "quod_run_s": secs(qrun["run_started_at"], qrun["updated_at"]),
                "plain_run_s": secs(prun["run_started_at"], prun["updated_at"]),
                "files": files,
                "path_filter_runs": name in pf,
            })
    with open(DATA / "rows.jsonl", "w") as f:
        for r in rows:
            f.write(json.dumps(r) + "\n")
    print(len(rows), "rows")


if __name__ == "__main__":
    main()
