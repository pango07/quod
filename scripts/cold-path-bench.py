#!/usr/bin/env python3
"""Serial/overlapped preparation control; execute only in the reviewed CI job.

One binary, workload commit, runner and policy; four counterbalanced fresh
Quod states, with a second observed execution per state. These are command
diagnostics, not a paired whole-job saving or a fresh-runner replay proof.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import time


WORKLOAD = "23b3694b0d983d5398cceed4b5b4e25c5c450c51"
TIMINGS = (
    "setup_ms", "pin_ms", "environment_ms", "checkout_import_ms", "metadata_ms",
    "import_ms", "lookup_ms", "mount_ms", "seal_ms", "exec_ms", "record_ms",
    "end_to_end_ms",
)


def run(argv, cwd, env, log):
    start = time.monotonic()
    with log.open("wb") as stream:
        result = subprocess.run(argv, cwd=cwd, env=env, stdout=stream,
                                stderr=subprocess.STDOUT, timeout=300)
    elapsed = (time.monotonic() - start) * 1000
    if result.returncode:
        print(log.read_text(errors="replace")[-6000:])
        raise AssertionError(f"{log.name}: command exit {result.returncode}")
    return elapsed


def read_private(path, missing_ok=False):
    # The normal setup keeps /var/lib/quod private to root. Read only this
    # experiment's event data; do not relax state or sandbox permissions.
    result = subprocess.run(["sudo", "-n", "cat", str(path)], stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=10)
    if result.returncode and missing_ok and b"No such file or directory" in result.stderr:
        return b""
    assert result.returncode == 0, "could not read experiment events"
    assert len(result.stdout) <= 8 * 1024 * 1024, "unexpected diagnostic size"
    return result.stdout


def event_lines(state, missing_ok=False):
    data = read_private(state / "events.jsonl", missing_ok)
    if not data:
        return []
    lines = data.decode().splitlines()
    assert all(len(line) <= 256 * 1024 for line in lines), "unbounded event"
    return lines


def event(state, previous):
    lines = event_lines(state)
    events = [json.loads(line) for line in lines[previous:]]
    assert not any(e.get("message") == "quod server offline" for e in events), "Cloud offline"
    runs = [e for e in events if e.get("message") == "run" and e.get("step_key")]
    assert len(runs) == 1, f"expected one command event, got {len(runs)}"
    row = runs[0]
    assert row["exit_code"] == 0 and row["mode"] == "observe", "execution policy changed"
    assert row["outcome"] != "replayed" and row["exec_ms"] > 0, "command was skipped"
    for field in TIMINGS:
        assert isinstance(row.get(field), (float, int)), f"missing {field}"
        assert math.isfinite(row[field]) and row[field] >= 0, f"invalid {field}"
    pins = [e for e in events if e.get("message") == "host pin phases"]
    assert len(pins) == 1, "host pin instrumentation missing"
    return row, pins[0], len(lines)


def observed(quod, repo, out, state, mode, label, command):
    before = len(event_lines(state, missing_ok=True))
    env = os.environ.copy()
    env.update(QUOD_HOME=str(state), QUOD_MODE="observe")
    argv = [str(quod), "run", "--toolchain", "node", "--diagnose-inputs"]
    if mode == "serial":
        argv.append("--serial-preparation")
    argv.extend(["--step", command])
    log = out / f"{label}.log"
    wall = run(argv, repo / "cloud", env, log)
    row, pin, _ = event(state, before)
    run_id = row["run_id"]
    assert len(run_id) == 26 and run_id.isalnum(), "invalid diagnostic identity"
    diagnostic = json.loads(read_private(state / "diagnostics" / f"{run_id}.json"))
    assert row["end_to_end_ms"] <= wall + 50, "internal wall time exceeds process time"
    result = {
        "label": label, "preparation": mode, "wall_ms": wall,
        "overhead_ms": wall - row["exec_ms"],
        "step_key": row["step_key"], "image": row["image"],
        "manifest_id": row["manifest_id"], "outcome": row["outcome"],
        "candidates": row["candidates"],
        "input_tree": diagnostic["input_tree"],
        "manifest_matches_input": diagnostic["manifest_matches_input"],
        "stdout_hash": diagnostic["stdout"], "stderr_hash": diagnostic["stderr"],
        "outputs_hash": diagnostic["outputs"], "root_outputs_hash": diagnostic["root_outputs"],
        "diagnostics_ms": row["diagnostics_ms"],
        **{field: row[field] for field in TIMINGS},
        **{field: pin[field] for field in ("fingerprint_ms", "platform_ms")},
    }
    print(json.dumps(result), flush=True)
    result["_manifest"] = diagnostic["manifest"]
    return result, log


def validate_controls(rows):
    """Read sets may vary; every observation must still hold on the same input.

    Keep all negative observations in their manifests. Only the control's
    identity comparison permits different, validated absence probes.
    """
    assert len(rows) >= 2, "missing comparison controls"
    for field in ("input_tree", "stdout_hash", "stderr_hash", "outputs_hash",
                  "root_outputs_hash", "step_key", "image"):
        assert len({row[field] for row in rows}) == 1, f"{field} changed between controls"
    reference = rows[0]["_manifest"]
    frame = {key: value for key, value in reference.items() if key != "entries"}
    positive = [entry for entry in reference["entries"] if entry["k"] != "a"]
    for row in rows:
        assert row["manifest_matches_input"] is True, "manifest disagrees with actual input"
        manifest = row["_manifest"]
        assert {key: value for key, value in manifest.items() if key != "entries"} == frame, "manifest frame changed"
        assert manifest["step_key"] == row["step_key"], "manifest belongs to another step"
        assert [entry for entry in manifest["entries"] if entry["k"] != "a"] == positive, "positive reads changed"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--quod", type=Path, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    quod, repo, out = (p.resolve() for p in (args.quod, args.repo, args.out))
    client_hash = hashlib.sha256(args.client.read_bytes()).hexdigest()
    installed_hash = hashlib.sha256(Path("/usr/local/lib/quod/quod").read_bytes()).hexdigest()
    assert client_hash == installed_hash, "root wrapper would run a different client"
    assert not out.is_relative_to(repo), "diagnostics must stay outside the imported checkout"
    out.mkdir(parents=True, exist_ok=False)
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    assert source == WORKLOAD, "workload source changed"
    assert os.environ.get("QUOD_SERVER", "").startswith("https://"), "Cloud not configured"
    assert len(os.environ.get("QUOD_SERVER_KEY", "")) == 64, "server key not pinned"
    baseline = []
    baseline.append(run(["npm", "run", "lint"], repo / "cloud", os.environ.copy(),
                        out / "plain-before.log"))
    rows = []
    last_state = None
    run_id = str(int(os.environ["GITHUB_RUN_ID"]))
    # ABBA: neither mode always benefits from going second. Every case has
    # a new Quod store; this does not make the OS page cache cold again.
    for index, mode in enumerate(("parallel", "serial", "serial", "parallel")):
        state = Path("/var/lib/quod") / f"cold-path-{run_id}-{index}-{mode}"
        assert not event_lines(state, missing_ok=True), "Quod state was not fresh"
        for warmth in ("fresh-state", "reused-state"):
            row, _ = observed(quod, repo, out, state, mode,
                              f"{index}-{mode}-{warmth}", "npm run lint")
            row["state"] = warmth
            rows.append(row)
        last_state = state
    # Report read-set variation, including between serial controls. Extra
    # absence probes are permitted only when valid on the same input tree.
    reference = {(entry["p"], entry["k"]): entry for entry in rows[0]["_manifest"]["entries"]}
    for row in rows[1:]:
        current = {(entry["p"], entry["k"]): entry for entry in row["_manifest"]["entries"]}
        added, removed = current.keys() - reference.keys(), reference.keys() - current.keys()
        changed = [key for key in current.keys() & reference.keys() if current[key] != reference[key]]
        sample = [{"path": key[0][:200], "kind": key[1], "change": kind}
                  for kind, keys in (("added", sorted(added)), ("removed", sorted(removed)),
                                     ("changed", sorted(changed))) for key in keys][:16]
        print(json.dumps({"label": row["label"], "manifest_delta": {
            "added": len(added), "removed": len(removed), "changed": len(changed),
            "samples": sample}}), flush=True)
    validate_controls(rows)
    for row in rows:
        del row["_manifest"]
    baseline.append(run(["npm", "run", "lint"], repo / "cloud", os.environ.copy(),
                        out / "plain-after.log"))
    # The full check commands still execute, with route generation and its
    # consumer in one sandbox; the native build retains Google Fonts access.
    observed(quod, repo, out, last_state, "parallel", "complete-typecheck",
             "npx next typegen && npx tsc --noEmit")
    _, test_log = observed(quod, repo, out, last_state, "parallel", "complete-tests",
                           "node scripts/vitest-quod.mjs")
    counters = [line for line in test_log.read_text().splitlines()
                if line.startswith("test files:")]
    assert len(counters) == 1 and "tests: 496 passed, 0 failed, 0 skipped, 496 total" in counters[0], counters
    print(counters[0], flush=True)
    run(["npm", "run", "build"], repo / "cloud", os.environ.copy(), out / "native-build.log")
    subprocess.run(["git", "diff", "--exit-code"], cwd=repo, check=True)
    summary = {
        "workload": source, "binary_sha256": client_hash,
        "plain_lint_ms": baseline, "observations": rows,
        "full_tests": 496,
        "limits": ["one runner, four counterbalanced fresh Quod stores; OS page cache is shared",
                   "observe mode executes every command; no replay or job-level savings claim",
                   "phase timings overlap and must not be added"],
    }
    for warmth in ("fresh-state", "reused-state"):
        group = [row for row in rows if row["state"] == warmth]
        summary[warmth] = {
            mode: {field: statistics.median(row[field] for row in group if row["preparation"] == mode)
                   for field in ("wall_ms", "overhead_ms", "pin_ms", "metadata_ms", "checkout_import_ms")}
            for mode in ("serial", "parallel")
        }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({key: summary[key] for key in ("fresh-state", "reused-state", "plain_lint_ms", "full_tests")}), flush=True)


if __name__ == "__main__":
    main()
