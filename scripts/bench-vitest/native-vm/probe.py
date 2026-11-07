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
import time

SCHEMA = "quod-native-vm-repeat-v1-investigation"
CONTRACT = "diskless-canonical-linux-native-v1-investigation"
SOURCE = "5eaa7e5160736054d53f8b71e2c522835eb427e0"
UUID = "3a84431b-2034-4c41-8f61-157200000001"
CPU = "Nehalem,rdrand=off,rdseed=off,rdtscp=off,pmu=off"
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


def parse_envelope(serial):
    data = serial.read_bytes()
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


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("image", type=Path)
    ap.add_argument("out", type=Path)
    ap.add_argument("--watchdog", type=int, default=180)
    args = ap.parse_args()
    if sys.platform != "linux" or os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("VM execution is restricted to the authorized GitHub Linux gate")
    require(30 <= args.watchdog <= 180, "Finite per-VM watchdog must be between 30 and 180 seconds")
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
    (out / "image.json").write_bytes((image / "image.json").read_bytes())
    host_env = {"PATH": "/usr/bin:/bin", "TZ": "UTC", "LANG": "C.UTF-8"}
    policy = {"schema": SCHEMA, "contract": CONTRACT, "source": SOURCE, "receiptAdmission": False,
              "imageMetadataSha256": digest(image / "image.json"), "probeSha256": digest(Path(__file__)),
              "cpu": CPU, "memoryMiB": 4096, "vcpus": 1, "tcgThread": "single", "machine": qemu["machine"],
              "uuid": UUID, "icount": "shift=0,align=off,sleep=off", "seed": 42,
              "rtc": "base=2023-11-14T22:13:20,clock=vm,driftfix=none", "hostEnvironment": host_env,
              "watchdogSeconds": args.watchdog, "maxSerialBytes": MAX_SERIAL,
              "network": "no NIC; guest-owned loopback only", "blockDevices": [], "hostMounts": [],
              "inputChannels": [], "output": "output-only file chardev bound to a root-only guest UART",
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
            timeout = 60 if command in {"boundary", "background"} else args.watchdog
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
            verify_inputs()
            require(process.returncode == 0 and not result["timedOut"], "QEMU did not terminate successfully")
            value, raw, envelope = parse_envelope(serial)
            (case / "response.json").write_bytes(raw)
            validate_response(value, command)
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
