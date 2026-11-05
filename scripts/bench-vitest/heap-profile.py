"""Same-runner ABBA diagnostic. Every sample executes the whole test suite.

This changes no product default and never uploads a reusable Quod ledger.
Plain and recorded modes run on separate runners: compare heap settings
within each mode, not the absolute times between those runners.
"""
import hashlib
import json
import os
import pathlib
import statistics
import subprocess
import sys
import time


def states(report, project):
    data = json.loads(report.read_text())
    files = data["testResults"]
    result = sorted(
        (str(pathlib.Path(f["name"]).relative_to(project)),
         a.get("fullName", a.get("title")), a["status"])
        for f in files for a in f["assertionResults"]
    )
    # This pinned c4 corpus must not silently select fewer tests/files.
    assert len(files) == 120, len(files)
    assert len(result) == 4129, len(result)
    assert all(s[2] in ("passed", "pending", "skipped", "todo") for s in result)
    return result


def main():
    project = pathlib.Path(sys.argv[1]).resolve()
    mode = sys.argv[2]
    assert mode in ("plain", "recorded")
    base = pathlib.Path("/tmp/quod-heap-profile")
    out = base / mode
    out.mkdir(parents=True, exist_ok=True)
    assert not os.environ.get("NODE_OPTIONS"), "Keep the diagnostic's heap settings unambiguous"
    assert not os.environ.get("QUOD_VITEST_JS_DIR"), "Use the pinned packaged recorder"
    environment = {
        "node": subprocess.check_output(["node", "--version"], text=True).strip(),
        "imageOS": os.environ.get("ImageOS"),
        "imageVersion": os.environ.get("ImageVersion"),
        "cpus": os.cpu_count(),
        "memory": pathlib.Path("/proc/meminfo").read_text().splitlines()[:3],
        "cpuModel": next(line.split(":", 1)[1].strip()
                         for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines()
                         if line.startswith("model name")),
        "projectCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=project, text=True).strip(),
        "engineCommit": "ae52750",
        "order": ["default", "64", "64", "default"],
        "reusableLedgerUploaded": False,
    }
    (out / "environment.json").write_text(json.dumps(environment, indent=2))
    rows = []
    expected = None
    for i, semi_space in enumerate(environment["order"]):
        tag = f"{i}-{semi_space}"
        report = out / f"{tag}-report.json"
        usage = out / f"{tag}-usage.json"
        env = dict(os.environ)
        if semi_space != "default":
            env["NODE_OPTIONS"] = f"--max-semi-space-size={semi_space}"
        if mode == "plain":
            cmd = ["node", "node_modules/vitest/vitest.mjs", "run",
                   "--reporter=default", "--reporter=json", f"--outputFile={report}"]
        else:
            env["QUOD_VITEST_NAMES_PROOF"] = "1"
            env["QUOD_VITEST_BOOLEAN_PROOF"] = "0"
            cmd = [str(base / "quod"), "vitest", "--full", "--dir", str(base / f"state-{i}"),
                   "--report", str(report), "--", "run"]
        # GNU time's RSS is a maximum for a process, not an aggregate tree peak.
        cmd = ["/usr/bin/time", "-f",
               '{"maxProcessRssKiB":%M,"userSeconds":%U,"systemSeconds":%S}',
               "-o", str(usage), *cmd]
        start = time.monotonic()
        with (out / f"{tag}.log").open("w") as log:
            result = subprocess.run(cmd, cwd=project, env=env, stdout=log, stderr=subprocess.STDOUT)
        row = {"mode": mode, "sample": i, "semiSpaceMiB": semi_space,
               "seconds": time.monotonic() - start, "exit": result.returncode}
        rows.append(row)
        (out / "times.json").write_text(json.dumps(rows, indent=2))
        assert result.returncode == 0, row
        row.update(json.loads(usage.read_text()))
        actual = states(report, project)
        if expected is None:
            expected = actual
        assert actual == expected, f"Per-test states changed in {tag}"
        row["testStates"] = len(actual)
        row["stateSha256"] = hashlib.sha256(json.dumps(actual).encode()).hexdigest()
        (out / "times.json").write_text(json.dumps(rows, indent=2))
        print(json.dumps(row), flush=True)
    averages = {setting: statistics.mean(r["seconds"] for r in rows if r["semiSpaceMiB"] == setting)
                for setting in ("default", "64")}
    summary = {"meanSeconds": averages, "ratio64ToDefault": averages["64"] / averages["default"],
               "testStates": len(expected), "wrong": 0,
               "scope": "4 full executions on one runner; no replay or job-cost claim"}
    (out / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary), flush=True)


if __name__ == "__main__":
    main()
