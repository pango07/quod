"""Profile the shipped recorder, bracketed by plain runs on the same runner.

This uses a disposable cold ledger and never publishes reusable records.
Profiling overhead is included; these are diagnostics, not job savings.
"""
import collections
import hashlib
import json
import os
import pathlib
import shlex
import shutil
import subprocess
import sys
import time

project, binary, base = map(lambda x: pathlib.Path(x).resolve(), sys.argv[1:])
assert not base.exists(), "profile output must be fresh"
out = base / "evidence"
cpu = out / "cpu"
cpu.mkdir(parents=True)
node = shutil.which("node")
assert node
wrapper = base / "profile-node"
wrapper.write_text("#!/bin/sh\nexec " + shlex.quote(node) +
                   " --cpu-prof --cpu-prof-dir=" + shlex.quote(str(cpu)) + ' "$@"\n')
wrapper.chmod(0o755)


def states(file):
    report = json.loads(file.read_text())
    assert isinstance(report.get("testResults"), list) and report["testResults"]
    rows = []
    statuses = collections.Counter()
    for module in report["testResults"]:
        tests = []
        for test in module["assertionResults"]:
            name, status = test["fullName"], test["status"]
            assert isinstance(name, str) and isinstance(status, str)
            tests.append([name, status])
            statuses[status] += 1
        rows.append([os.path.relpath(module["name"], project), module["status"], sorted(tests)])
    rows.sort()
    return rows, dict(statuses)


results = []
reference = None
for phase in ["plain-before", "recorded", "plain-after"]:
    report = out / (phase + "-report.json")
    env = dict(os.environ)
    # Measure the exact embedded implementation rather than a development override.
    assert not env.get("QUOD_VITEST_JS_DIR") and not env.get("QUOD_VITEST_DISABLE")
    args = ["run", "--reporter=default", "--reporter=json", "--outputFile.json=" + str(report)]
    if phase == "recorded":
        env.update(QUOD_VITEST_LINEAGE="1", QUOD_VITEST_TIMING="off")
        command = [str(binary), "vitest", "--node", str(wrapper), "--dir", str(base / "state"),
                   "--summary", str(out / "quod-summary.json"), "--full", "--", *args]
    else:
        command = [node, "node_modules/vitest/vitest.mjs", *args]
    start = time.monotonic()
    with (out / (phase + ".log")).open("w") as log:
        result = subprocess.run(command, cwd=project, env=env, stdout=log,
                                stderr=subprocess.STDOUT, timeout=300)
    elapsed = time.monotonic() - start
    assert result.returncode == 0, f"{phase} failed; see its log"
    rows, statuses = states(report)
    assert len(rows) == 120 and sum(len(row[2]) for row in rows) == 4131
    assert statuses == {"passed": 4130, "skipped": 1}, statuses
    if reference is None:
        reference = rows
    assert rows == reference, f"{phase} test states differ"
    row = {"phase": phase, "seconds": elapsed, "files": len(rows),
           "states": sum(len(r[2]) for r in rows), "statuses": statuses,
           "reportSha256": hashlib.sha256(report.read_bytes()).hexdigest()}
    results.append(row)
    (out / "times.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(row), flush=True)

summary = json.loads((out / "quod-summary.json").read_text())
assert summary["replayed"] == 0
profiles = sorted(cpu.glob("*.cpuprofile"))
assert profiles, "no CPU samples were collected"
ledger = json.loads((base / "state/ledger.json").read_text())
result = {
    "scope": "Vitest only; shipped cold recorder with CPU profiling between two unprofiled plain runs",
    "binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    "projectCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=project, text=True).strip(),
    "node": subprocess.check_output([node, "--version"], text=True).strip(),
    "image": os.environ.get("ImageOS", "") + "/" + os.environ.get("ImageVersion", ""),
    "phases": results, "semanticMatches": True, "replayed": 0,
    "metadataKeys": {kind: sum(k.startswith(kind + ":") for k in ledger["keys"])
                     for kind in ["sm", "sl", "st"]},
    "profiles": [{"file": p.name, "bytes": p.stat().st_size,
                  "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for p in profiles],
    "limitation": "One runner, fixed phase order and profiler overhead; no causal speedup or full-job savings claim."
}
(out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps({k: v for k, v in result.items() if k != "profiles"}), flush=True)
