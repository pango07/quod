#!/usr/bin/env python3
"""Run six bounded, diskless native VMs. No receipts or cache admission."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time

SCHEMA = "quod-native-vm-repeat-v1-investigation"
CONTRACT = "diskless-canonical-linux-native-v1-investigation"
SOURCE = "5eaa7e5160736054d53f8b71e2c522835eb427e0"
UUID = "3a84431b-2034-4c41-8f61-157200000001"
CPU = "Haswell-noTSX,rdrand=off,rdseed=off,rdtscp=off,pmu=off"
MAX_SERIAL = 100 * 1024 * 1024
BOUNDARY_NAMES = {
    "uid-gid-1000", "uart-read-denied", "uart-write-denied", "trusted-init-read-denied",
    "trusted-policy-read-denied", "pid1-memory-read-denied", "pid1-fd-read-denied",
    "pid1-kill-denied", "pid1-ptrace-denied", "mount-denied", "user-namespace-denied",
    "source-write-denied", "no-host-mount", "owned-loopback-real", "external-network-absent",
}


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def write_json(path, value):
    path.write_bytes(canonical(value) + b"\n")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def check_file(path, record):
    if record.get("absent") is True:
        require(not path.exists() and not path.is_symlink(), f"Negative input changed: {path}")
    else:
        require(path.is_file() and path.stat().st_size == record["bytes"] and digest(path) == record["sha256"],
                f"Actual input changed: {path}")


def decode(value):
    require(isinstance(value, str), "Base64 output must be a string")
    return base64.b64decode(value, validate=True)


def inspect_progress(serial, command):
    """Diagnostics never establish completion. Preserve valid records even if
    a watchdog interrupts a later frame; strict success rejects every error.
    """
    result = {"schema": "quod-native-vm-progress-report-v1", "completion": False,
              "receiptAdmission": False, "records": [], "frames": [], "bodyBytes": 0, "errors": []}
    data = serial.read_bytes()
    if len(data) > MAX_SERIAL:
        result["errors"].append("serial policy bound exceeded")
        return result
    matches = list(re.finditer(rb"(?:^|\n)QUOD_NATIVE_VM_PROGRESS_V1 ([0-9]+) ([0-9a-f]{64})\n", data))
    for line in data.splitlines():
        if line.startswith(b"QUOD_NATIVE_VM_") and not (line.startswith(b"QUOD_NATIVE_VM_PROGRESS_V1 ") or
                line.startswith(b"QUOD_NATIVE_VM_RESULT_V1 ") or line in {b"QUOD_NATIVE_VM_PROGRESS_END", b"QUOD_NATIVE_VM_END"}):
            result["errors"].append("unknown or truncated frame marker")
    if len(matches) > 64 or data.count(b"QUOD_NATIVE_VM_PROGRESS_V1") != len(matches):
        result["errors"].append("excess or malformed progress header")
    if data.count(b"\nQUOD_NATIVE_VM_PROGRESS_END\n") != len(matches):
        result["errors"].append("missing or extra progress terminator")
    last_clock, last_stdout, last_stderr, last_end = (-1, -1), 0, 0, 0
    for match in matches[:64]:
        try:
            size = int(match[1])
            require(0 < size <= 16384 and match.start() >= last_end, "progress bound/overlap")
            body = data[match.end():match.end() + size]
            end = match.end() + size
            require(len(body) == size and hashlib.sha256(body).hexdigest().encode() == match[2] and
                    data[end:].startswith(b"\nQUOD_NATIVE_VM_PROGRESS_END\n"), "progress truncated/digest/terminator")
            value = json.loads(body)
            require(value["schema"] == "quod-native-vm-progress-v1" and value["completion"] is False and
                    value["receiptAdmission"] is False and value["command"] == command and
                    value["sequence"] == len(result["records"]) + 1, "progress identity/sequence")
            clock = (value["guestMonotonicSeconds"], value["guestMonotonicNanoseconds"])
            require(clock >= last_clock and clock[0] >= 0 and 0 <= clock[1] < 1000000000, "progress guest clock")
            require(value["stdoutBytes"] >= last_stdout and value["stderrBytes"] >= last_stderr and
                    value["stage"] in {"child-started", "running", "child-exited-quiescent"} and
                    type(value["mainExited"]) is bool and type(value["mainPid"]) is int and value["mainPid"] > 1,
                    "progress process/output counters")
            for stream in ["stdout", "stderr"]:
                require(all(len(decode(value[stream + field])) <= min(512, value[stream + "Bytes"])
                            for field in ["PrefixBase64", "TailBase64"]), "progress output sample bound")
            require(len(decode(value["mainProcStatPrefixBase64"])) <= 256 and len(value["processes"]) <= 8 and
                    0 <= value["processEntriesScanned"] <= 256 and type(value["processSamplesTruncated"]) is bool,
                    "progress process scan bound")
            for process in value["processes"]:
                require(type(process["pid"]) is int and process["pid"] > 1 and
                        len(decode(process["statPrefixBase64"])) <= 256 and len(decode(process["cmdlinePrefixBase64"])) <= 256,
                        "progress process sample bound")
            result["bodyBytes"] += size
            require(result["bodyBytes"] <= 1024 * 1024, "progress total byte bound")
            last_end = end + len(b"\nQUOD_NATIVE_VM_PROGRESS_END\n")
            result["records"].append(value)
            result["frames"].append({"offset": match.start(), "end": last_end, "bodyBytes": size, "sha256": match[2].decode()})
            last_clock, last_stdout, last_stderr = clock, value["stdoutBytes"], value["stderrBytes"]
        except Exception as exc:
            result["errors"].append(str(exc))
    completions = list(re.finditer(rb"(?:^|\n)QUOD_NATIVE_VM_RESULT_V1 ([0-9]+) ([0-9a-f]{64})\n", data))
    if len(completions) > 1 or data.count(b"QUOD_NATIVE_VM_RESULT_V1") != len(completions):
        result["errors"].append("extra or malformed completion header")
    if data.count(b"\nQUOD_NATIVE_VM_END\n") != len(completions):
        result["errors"].append("missing or extra completion terminator")
    if completions:
        match = completions[0]
        size = int(match[1])
        body = data[match.end():match.end() + size]
        if not (0 < size <= 96 * 1024 * 1024 and len(body) == size and
                hashlib.sha256(body).hexdigest().encode() == match[2] and match.start() >= last_end and
                data[match.end() + size:].startswith(b"\nQUOD_NATIVE_VM_END\n")):
            result["errors"].append("completion truncated/digest/order/terminator")
    return result


def validate_response(value, command):
    require(value.get("schema") == "quod-native-vm-response-v1-investigation" and value.get("contract") == CONTRACT,
            "Wrong response contract")
    require(value.get("command") == command and value.get("receiptAdmission") is False, "Wrong command or admission flag")
    proof = value["proof"]
    require(all(proof.get(k) is True for k in ["entropySeededBeforeChild", "loopbackEnabled", "sourceReadOnly"]),
            "Missing enforced guest boundary proof")
    require(len(decode(proof["entropyProbeBase64"])) == 32, "Missing actual getrandom sample")
    effects = value["effects"]
    require(len(decode(effects["stdoutBase64"])) <= 16 * 1024 * 1024 and
            len(decode(effects["stderrBase64"])) <= 16 * 1024 * 1024, "Capture exceeds policy")
    rows = effects["dist"]
    require(isinstance(rows, list) and len(rows) == effects["distEntries"] <= 50000, "Wrong output entry count")
    seen, file_bytes = set(), 0
    for row in rows:
        path = decode(row["pathBase64"])
        require((path or row["type"] == "directory") and not path.startswith(b"/") and b".." not in path.split(b"/") and b"\0" not in path,
                "Invalid output path")
        require(path not in seen, "Duplicate output path")
        seen.add(path)
        require(isinstance(row["mode"], int) and 0 <= row["mode"] <= 0o7777 and row["uid"] == 1000 and row["gid"] == 1000,
                "Unexpected output mode/owner")
        if row["type"] == "file":
            data = decode(row["dataBase64"])
            require(len(data) == row["bytes"] and hashlib.sha256(data).hexdigest() == row["sha256"], "Output file digest mismatch")
            file_bytes += len(data)
        elif row["type"] == "symlink":
            require(b"\0" not in decode(row["targetBase64"]), "Invalid output symlink")
        else:
            require(row["type"] == "directory", "Special output rejected")
    require(file_bytes == effects["distBytes"] <= 48 * 1024 * 1024, "Wrong output byte count")
    if command == "background":
        require(value.get("ok") is False and value.get("complete") is False and value["exitCode"] == 0 and
                value["signal"] == 0 and value["leftoversDetected"] > 0 and value["killedProcesses"] > 0 and
                "processes remained" in value["failure"] and proof.get("captureComplete") is True and proof["remainingProcesses"] == 0,
                "A background descendant was not rejected and reaped")
    else:
        require(value.get("ok") is True and value.get("complete") is True and value["exitCode"] == 0 and
                value["signal"] == 0 and value["leftoversDetected"] == 0 and value["killedProcesses"] == 0 and
                value["failure"] == "" and proof.get("captureComplete") is True and proof["remainingProcesses"] == 0,
                "Command did not complete successfully and quiescently")
    if command == "boundary":
        boundary = json.loads(decode(effects["stdoutBase64"]))
        checks = boundary["checks"]
        require(boundary["schema"] == "quod-native-vm-boundary-v1" and boundary["count"] == len(BOUNDARY_NAMES) and
                len(checks) == len(BOUNDARY_NAMES) and boundary["failed"] == 0 and
                {c["name"] for c in checks} == BOUNDARY_NAMES and all(c["pass"] is True for c in checks),
                "Native adversarial boundary fixture did not pass every fixed check")
    if command == "build":
        require(rows and any(decode(r["pathBase64"]) == b"bin.cjs" and r["type"] == "file" for r in rows),
                "Actual build output bin.cjs is absent")
    if command == "release":
        # The actual original package command includes both release test files.
        # Require the native node:test terminal counters, not a mocked sentinel.
        stdout = decode(effects["stdoutBase64"]).decode("utf-8", errors="strict")
        require(re.search(r"(?:# |ℹ )tests 20(?:\s|$)", stdout) is not None and
                re.search(r"(?:# |ℹ )pass 20(?:\s|$)", stdout) is not None and
                re.search(r"(?:# |ℹ )fail 0(?:\s|$)", stdout) is not None,
                "Actual 20-test release tail did not finish")


def parse_envelope(serial, progress):
    data = serial.read_bytes()
    require(not progress["errors"], "Malformed or truncated diagnostic frames")
    require(len(data) <= MAX_SERIAL, "Serial output exceeded bounded policy")
    pattern = rb"(?:^|\n)QUOD_NATIVE_VM_RESULT_V1 ([0-9]+) ([0-9a-f]{64})\n"
    matches = list(re.finditer(pattern, data))
    require(len(matches) == 1, "Expected exactly one trusted envelope")
    match = matches[0]
    size = int(match[1])
    require(0 < size <= 96 * 1024 * 1024, "Envelope size outside policy")
    body = data[match.end():match.end() + size]
    require(len(body) == size and hashlib.sha256(body).hexdigest().encode() == match[2], "Envelope bytes/digest mismatch")
    suffix = data[match.end() + size:]
    require(suffix.startswith(b"\nQUOD_NATIVE_VM_END\n"), "Envelope terminator missing")
    require(b"QUOD_NATIVE_VM_RESULT_V1" not in suffix, "Unexpected second envelope")
    return json.loads(body), body, {"bytes": size, "sha256": match[2].decode(),
                                    "preambleBytes": match.start(), "trailingBytes": len(suffix) - 20}


def parser_fixtures(out):
    """Remote-only bounded parser checks; synthetic bytes never grant authority."""
    def frame(marker, value, end):
        body = canonical(value)
        return b"\n" + marker + b" " + str(len(body)).encode() + b" " + hashlib.sha256(body).hexdigest().encode() + b"\n" + body + end
    sample = {"schema": "quod-native-vm-progress-v1", "completion": False, "receiptAdmission": False,
              "command": "boundary", "sequence": 1, "guestMonotonicSeconds": 1, "guestMonotonicNanoseconds": 0,
              "mainPid": 2, "mainExited": False, "stage": "running", "stdoutBytes": 0, "stderrBytes": 0,
              "stdoutPrefixBase64": "", "stdoutTailBase64": "", "stderrPrefixBase64": "", "stderrTailBase64": "",
              "mainProcStatPrefixBase64": "", "processes": [], "processEntriesScanned": 0, "processSamplesTruncated": False}
    progress = frame(b"QUOD_NATIVE_VM_PROGRESS_V1", sample, b"\nQUOD_NATIVE_VM_PROGRESS_END\n")
    final = frame(b"QUOD_NATIVE_VM_RESULT_V1", {"syntheticParserFixture": True}, b"\nQUOD_NATIVE_VM_END\n")
    wrong_sequence = frame(b"QUOD_NATIVE_VM_PROGRESS_V1", {**sample, "sequence": 2}, b"\nQUOD_NATIVE_VM_PROGRESS_END\n")
    cases = [("valid-progress-and-final", progress + final, True), ("progress-is-not-completion", progress, False),
             ("truncated-progress-header", b"\nQUOD_NATIVE_VM_PROG" + final, False),
             ("truncated-progress-body", progress[:-40] + final, False),
             ("forged-progress-digest", progress.replace(b'"running"', b'"Running"') + final, False),
             ("wrong-progress-sequence", wrong_sequence + final, False),
             ("progress-after-completion", final + progress, False), ("extra-completion", progress + final + final, False),
             ("malformed-completion-header", progress + final.replace(b"RESULT_V1 ", b"RESULT_V1 x", 1), False),
             ("truncated-final", progress + final[:-8], False),
             ("missing-final-terminator", progress + final[:-20], False),
             ("extra-progress-terminator", progress + b"\nQUOD_NATIVE_VM_PROGRESS_END\n" + final, False)]
    checks = []
    with tempfile.TemporaryDirectory(prefix="parser-fixtures-", dir=out) as folder:
        serial = Path(folder) / "serial.raw"
        for name, data, accepted in cases:
            serial.write_bytes(data)
            observed = False
            try:
                diagnostics = inspect_progress(serial, "boundary")
                parse_envelope(serial, diagnostics)
                observed = True
            except (ValueError, KeyError, TypeError, json.JSONDecodeError):
                pass
            require(observed is accepted, "Remote frame parser fixture failed: " + name)
            checks.append({"name": name, "passed": True, "accepted": accepted})
    write_json(out / "parser-fixtures.json", {"schema": "quod-native-vm-parser-fixtures-v1", "checks": checks,
                                               "syntheticParserFixturesOnly": True, "completionAuthority": False})


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("image", type=Path)
    ap.add_argument("out", type=Path)
    ap.add_argument("--watchdog", type=int, default=180)
    ap.add_argument("--release-watchdog", type=int, default=None)
    args = ap.parse_args()
    if sys.platform != "linux" or os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("VM execution is restricted to the authorized GitHub Linux gate")
    require(30 <= args.watchdog <= 180, "Finite per-VM watchdog must be between 30 and 180 seconds")
    release_watchdog = args.watchdog if args.release_watchdog is None else args.release_watchdog
    require(30 <= release_watchdog <= 720, "Finite release watchdog must be between 30 and 720 seconds")
    image, out = args.image.resolve(), args.out.resolve()
    require(not out.exists(), "Evidence directory must not already exist")
    require("," not in str(image) and "," not in str(out), "QEMU file paths must not contain option separators")
    metadata = json.loads((image / "image.json").read_bytes())
    require(metadata["schema"] == "quod-native-vm-image-v1-investigation" and metadata["contract"] == CONTRACT and
            metadata["source"] == SOURCE and metadata["receiptAdmission"] is False, "Wrong image policy")
    require(metadata["inventorySha256"] == hashlib.sha256(canonical(metadata["inventory"])).hexdigest(), "Image inventory digest mismatch")
    qemu = metadata["qemu"]
    require(re.fullmatch(r"pc-i440fx-[0-9]+\.[0-9]+", qemu["machine"]) is not None, "Unversioned QEMU machine")
    require(qemu["bios"] == "/usr/share/seabios/bios-256k.bin" and qemu["dataDirectory"] == "/usr/share/qemu", "Unexpected firmware inputs")

    def verify_inputs():
        check_file(image / "kernel", metadata["kernel"])
        check_file(image / "initramfs.cpio.gz", metadata["initramfs"])
        for name, record in qemu["hostFiles"].items():
            check_file(Path(name), record)

    verify_inputs()
    out.mkdir(parents=True)
    parser_fixtures(out)
    (out / "image.json").write_bytes((image / "image.json").read_bytes())
    host_env = {"PATH": "/usr/bin:/bin", "TZ": "UTC", "LANG": "C.UTF-8"}
    policy = {"schema": SCHEMA, "contract": CONTRACT, "source": SOURCE, "receiptAdmission": False,
              "imageMetadataSha256": digest(image / "image.json"), "probeSha256": digest(Path(__file__)),
              "cpu": CPU, "memoryMiB": 4096, "vcpus": 1, "tcgThread": "single", "machine": qemu["machine"],
              "uuid": UUID, "icount": "shift=0,align=off,sleep=off", "seed": 42,
              "rtc": "base=2023-11-14T22:13:20,clock=vm,driftfix=none", "hostEnvironment": host_env,
              "watchdogSeconds": args.watchdog, "releaseWatchdogSeconds": release_watchdog, "maxSerialBytes": MAX_SERIAL,
              "network": "no NIC; guest-owned loopback only", "blockDevices": [], "hostMounts": [],
              "inputChannels": [], "output": "output-only file chardev bound to a root-only guest UART",
              "diagnostics": {"schema": "quod-native-vm-progress-v1", "completionAuthority": False,
                              "maxRecords": 64, "maxBodyBytes": 16384, "maxTotalBodyBytes": 1048576,
                              "intervalGuestNanoseconds": 1000000000, "captureIntervalGuestNanoseconds": 250000000,
                              "outputSampleBytes": 512, "maxProcEntriesScanned": 256, "maxProcSamples": 8, "procSampleBytes": 256},
              "limitations": ["No receipt or replay admission", "No exhaustive host-open or negative-lookup closure",
                              "RNDADDENTROPY mixes a seed; actual repeated-command getrandom agreement is required"],
              "scope": "actual build/release repeat-image investigation; no ordinary-equivalence or full-window claim"}
    policy["executionClassSha256"] = hashlib.sha256(b"quod-native-vm-class-v1\n" + canonical(policy)).hexdigest()
    write_json(out / "policy.json", policy)
    summary = {"schema": SCHEMA, "policy": policy, "runs": [], "ok": False, "receiptAdmission": False,
               "fixedOrder": ["boundary", "background", "build-1", "build-2", "release-1", "release-2"]}
    write_json(out / "result.json", summary)
    try:
        for name in summary["fixedOrder"]:
            command = name.split("-", 1)[0]
            case = out / name
            case.mkdir()
            serial = case / "serial.raw"
            require(not serial.exists(), "Output channel must be a fresh regular file")
            serial.touch(mode=0o600)
            # file chardev has no input source. There is no stdio/pty/socket
            # backend, monitor, QMP, guest agent, disk, shared mount or NIC.
            argv = [qemu["path"], "-no-user-config", "-nodefaults", "-display", "none", "-monitor", "none",
                    "-nic", "none", "-accel", "tcg,thread=single", "-machine", qemu["machine"], "-cpu", CPU,
                    "-smp", "1", "-m", "4096", "-uuid", UUID, "-seed", "42", "-icount", policy["icount"],
                    "-rtc", policy["rtc"], "-L", qemu["dataDirectory"], "-bios", qemu["bios"],
                    "-kernel", str(image / "kernel"), "-initrd", str(image / "initramfs.cpio.gz"),
                    "-append", "init=/init rdinit=/init console=ttyS0,115200n8 quiet loglevel=0 nokaslr "
                    "random.trust_cpu=off random.trust_bootloader=off panic=-1 oops=panic quod.command=" + command,
                    "-chardev", f"file,id=trusted-output,path={serial}", "-device", "isa-serial,chardev=trusted-output,index=0",
                    "-no-reboot"]
            timeout = (60 if command in {"boundary", "background"} else
                       release_watchdog if command == "release" else args.watchdog)
            invocation = {"argv": argv, "environment": host_env, "stdin": "DEVNULL", "watchdogSeconds": timeout,
                          "imageMetadataSha256": policy["imageMetadataSha256"], "policySha256": digest(out / "policy.json")}
            write_json(case / "invocation.json", invocation)
            verify_inputs()
            started = time.monotonic()
            result = {"name": name, "command": command, "timedOut": False, "ok": False}
            with (case / "qemu.stdout").open("wb") as stdout, (case / "qemu.stderr").open("wb") as stderr:
                process = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr,
                                           env=host_env, start_new_session=True)
                try:
                    # Polling bounds serial disk use while retaining every byte.
                    while process.poll() is None:
                        if time.monotonic() - started > timeout:
                            result["timedOut"] = True
                            raise TimeoutError("QEMU watchdog expired; completion is incomplete")
                        if serial.stat().st_size > MAX_SERIAL:
                            raise ValueError("Serial policy bound exceeded")
                        time.sleep(0.05)
                except BaseException:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait()
                    raise
                finally:
                    result["seconds"] = time.monotonic() - started
                    result["qemuExitCode"] = process.returncode
                    progress = inspect_progress(serial, command)
                    write_json(case / "progress.json", progress)
                    result["diagnostics"] = {"records": len(progress["records"]), "bodyBytes": progress["bodyBytes"],
                                             "errors": progress["errors"], "sha256": digest(case / "progress.json")}
            verify_inputs()
            require(process.returncode == 0 and not result["timedOut"], "QEMU did not terminate successfully")
            value, raw, envelope = parse_envelope(serial, progress)
            (case / "response.json").write_bytes(raw)
            validate_response(value, command)
            require(value["diagnostics"]["progressCount"] == len(progress["records"]) and
                    len(progress["records"]) >= 1 and
                    value["diagnostics"]["progressBodyBytes"] == progress["bodyBytes"] and
                    type(value["diagnostics"]["progressBudgetExhausted"]) is bool, "Guest diagnostic count differs")
            (case / "command.stdout").write_bytes(decode(value["effects"]["stdoutBase64"]))
            (case / "command.stderr").write_bytes(decode(value["effects"]["stderrBase64"]))
            result.update(ok=True, envelope=envelope, responseSha256=hashlib.sha256(raw).hexdigest(),
                          serialSha256=digest(serial), effectsSha256=hashlib.sha256(canonical(value["effects"])).hexdigest(),
                          entropyProbeBase64=value["proof"]["entropyProbeBase64"],
                          expectedRejection=command == "background", distEntries=value["effects"]["distEntries"],
                          distBytes=value["effects"]["distBytes"])
            write_json(case / "result.json", result)
            summary["runs"].append(result)
            write_json(out / "result.json", summary)
            print(json.dumps({"phase": name, "ok": True, "seconds": result["seconds"], "effectsSha256": result["effectsSha256"]}), flush=True)
        by_name = {row["name"]: row for row in summary["runs"]}
        for command in ["build", "release"]:
            require(by_name[command + "-1"]["entropyProbeBase64"] == by_name[command + "-2"]["entropyProbeBase64"],
                    f"Actual kernel getrandom differs across identical {command} boot inputs")
            require(by_name[command + "-1"]["effectsSha256"] == by_name[command + "-2"]["effectsSha256"],
                    f"Actual {command} raw output/files/metadata differed across identical images")
        summary["ok"] = True
        summary["agreement"] = {"build": True, "release": True, "kernelGetrandom": True}
    except Exception as exc:
        summary["failure"] = str(exc)
        # Preserve per-case invocation/logs/raw serial even if spawn, boot,
        # envelope verification or the watchdog fails. Never fabricate success.
        if "result" in locals() and result not in summary["runs"]:
            result["failure"] = str(exc)
            result.setdefault("seconds", time.monotonic() - started)
            write_json(case / "result.json", result)
            summary["runs"].append(result)
    write_json(out / "result.json", summary)
    hashes = {p.relative_to(out).as_posix(): {"sha256": digest(p), "bytes": p.stat().st_size}
              for p in sorted(out.rglob("*")) if p.is_file()}
    write_json(out / "files.json", hashes)
    print(json.dumps({"ok": summary["ok"], "runs": len(summary["runs"]), "receiptAdmission": False,
                      "failure": summary.get("failure")}), flush=True)
    return 0 if summary["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
