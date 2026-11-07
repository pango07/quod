#!/usr/bin/env python3
"""Independent data-only auditor. Never imports or executes artifact code."""
import argparse
import base64
import gzip
import hashlib
import json
from pathlib import Path
import re
import stat
import sys

HERE = Path(__file__).resolve().parent
CONTRACT = "diskless-canonical-linux-native-v1-investigation"
SOURCE = "5eaa7e5160736054d53f8b71e2c522835eb427e0"
ORDER = ["boundary", "background", "build-1", "build-2", "release-1", "release-2"]
NAMES = {"uid-gid-1000", "uart-read-denied", "uart-write-denied", "trusted-init-read-denied",
         "trusted-policy-read-denied", "pid1-memory-read-denied", "pid1-fd-read-denied", "pid1-kill-denied",
         "pid1-ptrace-denied", "mount-denied", "user-namespace-denied", "source-write-denied", "no-host-mount",
         "owned-loopback-real", "external-network-absent"}
SYMLINK_CHECKS = {"absolute-through-absolute": "accepted", "relative-chain": "accepted", "legitimate-parent": "accepted",
                  "dangling-rejected": "rejected", "cycle-rejected": "rejected", "root-escape-rejected": "rejected"}
PARSER_CHECKS = {"valid-progress-and-final", "progress-is-not-completion", "truncated-progress-header", "truncated-progress-body",
                 "forged-progress-digest", "wrong-progress-sequence", "progress-after-completion", "extra-completion",
                 "malformed-completion-header", "truncated-final", "missing-final-terminator", "extra-progress-terminator"}


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def sha(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for part in iter(lambda: f.read(1024 * 1024), b""):
            h.update(part)
    return h.hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load(path):
    require(path.is_file() and not path.is_symlink(), f"Missing regular evidence file: {path}")
    require(path.stat().st_size <= 160 * 1024 * 1024, f"JSON bound: {path}")
    return json.loads(path.read_bytes())


def decode(value):
    return base64.b64decode(value, validate=True)


def audit_progress(data, command):
    """Independently decode diagnostics without granting completion authority."""
    out = {"schema": "quod-native-vm-progress-report-v1", "completion": False, "receiptAdmission": False,
           "records": [], "frames": [], "bodyBytes": 0, "errors": []}
    require(len(data) <= 100 * 1024**2, "Serial bound")
    headers = list(re.finditer(rb"(?:^|\n)QUOD_NATIVE_VM_PROGRESS_V1 ([0-9]+) ([0-9a-f]{64})\n", data))
    if len(headers) > 64 or data.count(b"QUOD_NATIVE_VM_PROGRESS_V1") != len(headers):
        out["errors"].append("excess or malformed progress header")
    if data.count(b"\nQUOD_NATIVE_VM_PROGRESS_END\n") != len(headers):
        out["errors"].append("missing or extra progress terminator")
    for line in data.splitlines():
        if line.startswith(b"QUOD_NATIVE_VM_") and not (line.startswith(b"QUOD_NATIVE_VM_PROGRESS_V1 ") or
                line.startswith(b"QUOD_NATIVE_VM_RESULT_V1 ") or line in {b"QUOD_NATIVE_VM_PROGRESS_END", b"QUOD_NATIVE_VM_END"}):
            out["errors"].append("unknown or truncated frame marker")
    last_end, last_clock, captured = 0, (-1, -1), (0, 0)
    for header in headers[:64]:
        try:
            size = int(header[1]);start = header.end();end = start + size
            require(0 < size <= 16384 and header.start() >= last_end, "progress bound/overlap")
            raw = data[start:end]
            require(len(raw) == size and hashlib.sha256(raw).hexdigest().encode() == header[2] and
                    data[end:].startswith(b"\nQUOD_NATIVE_VM_PROGRESS_END\n"), "progress truncated/digest/terminator")
            record = json.loads(raw)
            clock = (record["guestMonotonicSeconds"], record["guestMonotonicNanoseconds"])
            require(record["schema"] == "quod-native-vm-progress-v1" and record["completion"] is False and
                    record["receiptAdmission"] is False and record["command"] == command and record["sequence"] == len(out["records"]) + 1,
                    "progress identity/sequence")
            require(clock >= last_clock and clock[0] >= 0 and 0 <= clock[1] < 1000000000 and
                    record["stdoutBytes"] >= captured[0] and record["stderrBytes"] >= captured[1], "progress monotonic counters")
            require(record["stage"] in {"child-started", "running", "child-exited-quiescent"} and type(record["mainExited"]) is bool and
                    type(record["mainPid"]) is int and record["mainPid"] > 1, "progress process metadata")
            for stream in ["stdout", "stderr"]:
                for suffix in ["PrefixBase64", "TailBase64"]:
                    require(len(decode(record[stream + suffix])) <= min(512, record[stream + "Bytes"]), "progress output bound")
            require(len(decode(record["mainProcStatPrefixBase64"])) <= 256 and len(record["processes"]) <= 8 and
                    0 <= record["processEntriesScanned"] <= 256 and type(record["processSamplesTruncated"]) is bool, "progress scan bound")
            for process in record["processes"]:
                require(type(process["pid"]) is int and process["pid"] > 1 and len(decode(process["statPrefixBase64"])) <= 256 and
                        len(decode(process["cmdlinePrefixBase64"])) <= 256, "progress sample bound")
            out["bodyBytes"] += size
            require(out["bodyBytes"] <= 1048576, "progress total bound")
            last_end = end + len(b"\nQUOD_NATIVE_VM_PROGRESS_END\n")
            out["records"].append(record)
            out["frames"].append({"offset": header.start(), "end": last_end, "bodyBytes": size, "sha256": header[2].decode()})
            last_clock, captured = clock, (record["stdoutBytes"], record["stderrBytes"])
        except Exception as exc:
            out["errors"].append(str(exc))
    finals = list(re.finditer(rb"(?:^|\n)QUOD_NATIVE_VM_RESULT_V1 ([0-9]+) ([0-9a-f]{64})\n", data))
    if len(finals) > 1 or data.count(b"QUOD_NATIVE_VM_RESULT_V1") != len(finals):
        out["errors"].append("extra or malformed completion header")
    if data.count(b"\nQUOD_NATIVE_VM_END\n") != len(finals):
        out["errors"].append("missing or extra completion terminator")
    if finals:
        header = finals[0];size = int(header[1]);raw = data[header.end():header.end()+size]
        if not (0 < size <= 96 * 1024**2 and len(raw) == size and hashlib.sha256(raw).hexdigest().encode() == header[2] and
                header.start() >= last_end and data[header.end()+size:].startswith(b"\nQUOD_NATIVE_VM_END\n")):
            out["errors"].append("completion truncated/digest/order/terminator")
    return out


def audit_symlink_graph(entries):
    """Resolve the archived graph as data. Host filesystem resolution is never
    used for a guest target, including at subsequent absolute symlink hops.
    """
    for origin, original in entries.items():
        if original["type"] != "symlink":
            continue
        todo, parents, hops = origin.split("/"), [], 0
        while todo:
            piece = todo.pop(0)
            if piece in {"", "."}:
                continue
            if piece == "..":
                require(parents, "Archived symlink escapes root: " + origin)
                parents.pop()
                continue
            key = "/".join(parents + [piece])
            require(key in entries, "Archived symlink target missing: " + origin)
            row = entries[key]
            if row["type"] == "symlink":
                hops += 1
                require(hops <= 40, "Archived symlink cycle/hop limit: " + origin)
                target = row["target"]
                if target.startswith("/"):
                    parents = []
                todo = target.split("/") + todo
            else:
                require(not todo or row["type"] == "directory", "Archived symlink crosses a non-directory: " + origin)
                parents.append(piece)


def audit_archive(path, inventory):
    expected = {row["path"]: row for row in inventory}
    require(len(expected) == len(inventory) <= 500000, "Duplicate/excess image entries")
    found, hardlinks = {}, {}
    total = 0

    def exact(f, n):
        data = f.read(n)
        require(len(data) == n, "Truncated initramfs")
        return data

    with gzip.open(path, "rb") as f:
        while True:
            header = exact(f, 110)
            require(header[:6] == b"070701", "Expected canonical newc archive")
            fields = [int(header[6 + i*8:14 + i*8], 16) for i in range(13)]
            ino, mode, uid, gid, nlink, mtime, size, major, minor, rmajor, rminor, namesize, crc = fields
            require(0 < namesize <= 4096 and size <= 2 * 1024**3, "Archive entry bound")
            raw_name = exact(f, namesize)
            require(raw_name[-1:] == b"\0" and b"\0" not in raw_name[:-1], "Invalid archive name")
            name = raw_name[:-1].decode("utf-8", errors="strict")
            exact(f, (-(110 + namesize)) % 4)
            if name == "TRAILER!!!":
                require(size == 0, "Nonempty cpio trailer")
                tail = f.read(1024 * 1024)
                require(not tail.strip(b"\0") and not f.read(1), "Unexpected initramfs payload after trailer")
                break
            require(name not in found and not name.startswith("/") and ".." not in Path(name).parts, "Invalid/duplicate image path")
            require(uid == gid == 0 and mtime == 1700000000 and rmajor == rminor == crc == 0,
                    f"Noncanonical image metadata: {name}")
            require(not any(p in {".git", ".ssh", ".aws", ".netrc", ".git-credentials", ".docker", ".kube"} for p in Path(name).parts),
                    "Private metadata in image")
            total += size
            require(total <= 3 * 1024**3, "Uncompressed image exceeds declared gate budget")
            h = hashlib.sha256()
            small = bytearray()
            remaining = size
            while remaining:
                part = exact(f, min(1024 * 1024, remaining))
                h.update(part)
                if stat.S_ISLNK(mode) or Path(name).name in {".npmrc", ".yarnrc", ".yarnrc.yml"}:
                    require(len(small) + len(part) <= 1024 * 1024, "Configuration/symlink bound")
                    small.extend(part)
                remaining -= len(part)
            exact(f, (-size) % 4)
            row = {"mode": stat.S_IMODE(mode), "size": size, "sha256": h.hexdigest(), "kind": stat.S_IFMT(mode),
                   "linkGroup": (major, minor, ino) if nlink > 1 and stat.S_ISREG(mode) else None}
            if stat.S_ISLNK(mode):
                row["target"] = small.decode("utf-8", errors="strict")
            if Path(name).name in {".npmrc", ".yarnrc", ".yarnrc.yml"}:
                require(re.search(rb"(?i)(_auth|token|password|username|credential|certfile|keyfile)", small) is None,
                        "Authentication-bearing image config")
            if row["linkGroup"] and size:
                previous = hardlinks.setdefault(row["linkGroup"], (size, row["sha256"]))
                require(previous == (size, row["sha256"]), "Conflicting hard-link content")
            found[name] = row
    require(set(found) == set(expected) | {"."}, "Archive/inventory path mismatch")
    require(found["."]["mode"] == 0o755 and found["."]["kind"] == stat.S_IFDIR, "Noncanonical archive root")
    for name, expected_row in expected.items():
        row = found[name]
        require(row["mode"] == expected_row["mode"] and expected_row["uid"] == expected_row["gid"] == 0 and
                expected_row["mtime"] == 1700000000, f"Inventory metadata mismatch: {name}")
        kind = expected_row["type"]
        require(row["kind"] == {"file": stat.S_IFREG, "directory": stat.S_IFDIR, "symlink": stat.S_IFLNK}[kind],
                f"Inventory type mismatch: {name}")
        if kind == "file":
            actual = hardlinks.get(row["linkGroup"], (row["size"], row["sha256"]))
            require(actual == (expected_row["bytes"], expected_row["sha256"]), f"Inventory content mismatch: {name}")
        elif kind == "symlink":
            require(row["target"] == expected_row["target"], f"Inventory symlink mismatch: {name}")
    require(expected["init"]["mode"] == 0o700 and expected["trusted"]["mode"] == 0o700 and
            expected["trusted/policy.json"]["mode"] == 0o600, "Trusted image files are not private")
    audit_symlink_graph(expected)
    return {"entries": len(expected), "storedDataBytes": total}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("image", type=Path)
    ap.add_argument("evidence", type=Path)
    args = ap.parse_args()
    image, evidence = args.image.resolve(), args.evidence.resolve()
    report = {"schema": "quod-native-vm-audit-v1-investigation", "ok": False, "receiptAdmission": False, "issues": []}
    try:
        metadata = load(image / "image.json")
        require(metadata["schema"] == "quod-native-vm-image-v1-investigation" and metadata["contract"] == CONTRACT and
                metadata["source"] == SOURCE and metadata["receiptAdmission"] is False, "Wrong image contract/source")
        require((image / "image.json").read_bytes() == (evidence / "image.json").read_bytes(), "Retained image metadata differs")
        for name, key in [("kernel", "kernel"), ("initramfs.cpio.gz", "initramfs")]:
            require((image / name).is_file() and (image / name).stat().st_size == metadata[key]["bytes"] and
                    sha(image / name) == metadata[key]["sha256"], f"Actual {name} bytes differ")
        for name, field in [("build.py", "builderSha256"), ("guest-init.c", "guestInitSourceSha256"), ("guest-negative.c", "negativeSourceSha256")]:
            require(sha(HERE / name) == metadata[field], f"Frozen source differs: {name}")
        require(hashlib.sha256(canonical(metadata["inventory"])).hexdigest() == metadata["inventorySha256"], "Inventory hash differs")
        require(metadata["rootMetadata"] == {"mode": 0o755, "uid": 0, "gid": 0, "mtime": 1700000000}, "Wrong declared image-root metadata")
        symlinks = load(image / "symlink-fixtures.json")
        checks = symlinks["checks"]
        require(symlinks["schema"] == "quod-native-vm-symlink-fixtures-v1" and checks == metadata["symlinkChecks"] and
                len(checks) == len(SYMLINK_CHECKS) and {r["name"]: r["expected"] for r in checks} == SYMLINK_CHECKS and
                all(r["passed"] is True for r in checks), "Remote guest symlink resolution fixtures missing/failed")
        require(hashlib.sha256(canonical(metadata["sourceInventory"])).hexdigest() == metadata["guestPolicy"]["sourceInventorySha256"],
                "Tracked source inventory hash differs")
        report["archive"] = audit_archive(image / "initramfs.cpio.gz", metadata["inventory"])
        inv = {r["path"]: r for r in metadata["inventory"]}
        isa = load(image / "guest-isa-notes.json")
        isa_rows = isa["records"]
        require(isa["schema"] == "quod-native-vm-guest-isa-notes-v1" and isa_rows == metadata["guestIsaNotes"] and
                len(isa_rows) == 6 and {r["path"] for r in isa_rows} ==
                {"/init", "/usr/bin/env", "/lib/x86_64-linux-gnu/libc.so.6", "/lib64/ld-linux-x86-64.so.2", "/usr/bin/git", "/tools/node/bin/node"},
                "Missing actual guest ISA-note records")
        for row in isa_rows:
            require(row["resolvedPath"].startswith("/") and row["resolvedPath"][1:] in inv and
                    row["sha256"] == inv[row["resolvedPath"][1:]]["sha256"] and
                    hashlib.sha256(row["readelfNotes"].encode()).hexdigest() == row["readelfNotesSha256"],
                    "Guest ISA-note identity differs from actual archived bytes")
        require(inv["tools/node/bin/node"]["sha256"] == metadata["node"]["sha256"], "Actual guest Node differs")
        builds = {r["source"]: r for r in metadata["compile"]}
        require(set(builds) == {"guest-init.c", "guest-negative.c"} and builds["guest-init.c"]["sha256SelfTestPassed"] is True,
                "Trusted supervisor self-test/compile records absent")
        for source, binary in [("guest-init.c", "init"), ("guest-negative.c", "usr/bin/quod-negative")]:
            require(builds[source]["sourceSha256"] == sha(HERE / source) and builds[source]["outputSha256"] == inv[binary]["sha256"],
                    "Actual compiled trusted entry differs")
        policy = load(evidence / "policy.json")
        key = policy.pop("executionClassSha256")
        require(key == hashlib.sha256(b"quod-native-vm-class-v1\n" + canonical(policy)).hexdigest(), "Execution class key differs")
        policy["executionClassSha256"] = key
        require(policy["contract"] == CONTRACT and policy["source"] == SOURCE and policy["receiptAdmission"] is False and
                policy["probeSha256"] == sha(HERE / "probe.py") and policy["imageMetadataSha256"] == sha(image / "image.json"),
                "Wrong keyed supervisor or image")
        require(policy["cpu"] == "Haswell-noTSX,rdrand=off,rdseed=off,rdtscp=off,pmu=off" and policy["vcpus"] == 1 and
                policy["memoryMiB"] == 4096 and policy["tcgThread"] == "single" and
                policy["icount"] == "shift=0,align=off,sleep=off" and policy["seed"] == 42 and
                policy["rtc"] == "base=2023-11-14T22:13:20,clock=vm,driftfix=none" and
                policy["uuid"] == "3a84431b-2034-4c41-8f61-157200000001" and policy["hostMounts"] == policy["blockDevices"] == policy["inputChannels"] == [],
                "Profile configuration differs")
        require(policy["hostEnvironment"] == {"PATH": "/usr/bin:/bin", "TZ": "UTC", "LANG": "C.UTF-8"}, "Host environment leaks inputs")
        require(policy["diagnostics"] == {"schema": "quod-native-vm-progress-v1", "completionAuthority": False,
                "maxRecords": 64, "maxBodyBytes": 16384, "maxTotalBodyBytes": 1048576, "intervalGuestNanoseconds": 1000000000,
                "captureIntervalGuestNanoseconds": 250000000, "outputSampleBytes": 512, "maxProcEntriesScanned": 256,
                "maxProcSamples": 8, "procSampleBytes": 256}, "Unbounded or authoritative diagnostics policy")
        parser = load(evidence / "parser-fixtures.json")
        require(parser["schema"] == "quod-native-vm-parser-fixtures-v1" and parser["syntheticParserFixturesOnly"] is True and
                parser["completionAuthority"] is False and len(parser["checks"]) == len(PARSER_CHECKS) and
                {r["name"] for r in parser["checks"]} == PARSER_CHECKS and all(r["passed"] is True for r in parser["checks"]) and
                all(r["accepted"] is (r["name"] == "valid-progress-and-final") for r in parser["checks"]), "Remote parser fixtures missing/failed")
        require(type(policy["watchdogSeconds"]) is int and 30 <= policy["watchdogSeconds"] <= 180 and
                type(policy["releaseWatchdogSeconds"]) is int and 30 <= policy["releaseWatchdogSeconds"] <= 360 and
                policy["maxSerialBytes"] == 100 * 1024**2, "Unbounded supervisor")
        for path in [metadata["qemu"]["path"], metadata["qemu"]["bios"], "/etc/ld.so.cache", "/etc/ld.so.preload"]:
            require(path in metadata["qemu"]["hostFiles"], "Incomplete actual emulator/loader identity")
        scans = metadata["qemu"]["ldd"]
        require(len(scans) == len({r["path"] for r in scans}) and
                any(r["path"] == metadata["qemu"]["path"] and r["hostDependencyScan"] is True for r in scans),
                "Actual emulator dynamic dependency scan missing")
        for scan in scans:
            expected_scan = scan["elfClass"] == 2 and scan["encoding"] == 1 and scan["machine"] == 62 and bool(scan["dynamicNeeded"])
            require(scan["path"] in metadata["qemu"]["hostFiles"] and scan["hostDependencyScan"] is expected_scan,
                    "Host ELF classification differs")
            if expected_scan:
                require(scan["output"] and "not found" not in scan["output"], "Host dynamic dependencies unresolved")
                for name in re.findall(r"(?:=>\s+)?(/[\w./+:-]+)\s+\(", scan["output"]):
                    require(name in metadata["qemu"]["hostFiles"], "Host dynamic dependency hash missing")
            else:
                require(scan["output"] == "", "Foreign/static firmware was treated as host dynamic code")
        summary = load(evidence / "result.json")
        report["diagnostics"] = {}
        for name in ORDER:
            case = evidence / name
            if not (case / "serial.raw").is_file():
                continue
            diagnostics = audit_progress((case / "serial.raw").read_bytes(), name.split("-", 1)[0])
            report["diagnostics"][name] = diagnostics
            require(diagnostics == load(case / "progress.json"), "Independent progress parse differs: " + name)
            require(not diagnostics["errors"], "Malformed/truncated progress evidence: " + name)
        require(summary["ok"] is True and summary["receiptAdmission"] is False and summary["policy"] == policy and
                summary["fixedOrder"] == ORDER and [r["name"] for r in summary["runs"]] == ORDER,
                "Investigation did not finish every fixed phase")
        require(summary["agreement"] == {"build": True, "release": True, "kernelGetrandom": True}, "Missing agreement")
        results, responses = {}, {}
        for name in ORDER:
            case, command = evidence / name, name.split("-", 1)[0]
            watchdog = (60 if command in {"boundary", "background"} else
                        policy["releaseWatchdogSeconds"] if command == "release" else policy["watchdogSeconds"])
            result = load(case / "result.json")
            require(result == summary["runs"][ORDER.index(name)] and result["ok"] is True and result["timedOut"] is False and
                    result["qemuExitCode"] == 0 and 0 < result["seconds"] <= watchdog + 1,
                    "Bad actual VM completion/timing")
            serial = (case / "serial.raw").read_bytes()
            require(len(serial) <= policy["maxSerialBytes"] and hashlib.sha256(serial).hexdigest() == result["serialSha256"], "Serial hash differs")
            matches = list(re.finditer(rb"(?:^|\n)QUOD_NATIVE_VM_RESULT_V1 ([0-9]+) ([0-9a-f]{64})\n", serial))
            require(len(matches) == 1, "Not exactly one root envelope")
            match = matches[0];size = int(match[1]);body = serial[match.end():match.end()+size]
            require(size == len(body) <= 96 * 1024**2 and hashlib.sha256(body).hexdigest().encode() == match[2] and
                    serial[match.end()+size:].startswith(b"\nQUOD_NATIVE_VM_END\n") and body == (case / "response.json").read_bytes() and
                    hashlib.sha256(body).hexdigest() == result["responseSha256"] == result["envelope"]["sha256"], "Raw envelope differs")
            response = json.loads(body)
            diagnostics = report["diagnostics"][name]
            require(len(diagnostics["records"]) >= 1 and response["diagnostics"]["progressCount"] == len(diagnostics["records"]) and
                    response["diagnostics"]["progressBodyBytes"] == diagnostics["bodyBytes"] and
                    type(response["diagnostics"]["progressBudgetExhausted"]) is bool and
                    result["diagnostics"]["sha256"] == sha(case / "progress.json"), "Completion/diagnostic evidence differs")
            require(response["schema"] == "quod-native-vm-response-v1-investigation" and response["contract"] == CONTRACT and
                    response["command"] == command and response["receiptAdmission"] is False, "Response contract differs")
            proof = response["proof"]
            require(all(proof[k] is True for k in ["entropySeededBeforeChild", "loopbackEnabled", "sourceReadOnly", "captureComplete"]) and
                    proof["remainingProcesses"] == 0 and len(decode(proof["entropyProbeBase64"])) == 32,
                    "Boundary/completion proof missing")
            if command == "background":
                require(response["ok"] is False and response["complete"] is False and response["exitCode"] == 0 and
                        response["signal"] == 0 and response["leftoversDetected"] > 0 and response["killedProcesses"] > 0 and
                        "processes remained" in response["failure"] and result["expectedRejection"] is True, "Background was admitted")
            else:
                require(response["ok"] is True and response["complete"] is True and response["exitCode"] == response["signal"] ==
                        response["leftoversDetected"] == response["killedProcesses"] == 0 and response["failure"] == "" and
                        result["expectedRejection"] is False, "Command failed or had leftover work")
            effects = response["effects"]
            require(hashlib.sha256(canonical(effects)).hexdigest() == result["effectsSha256"], "Effects digest differs")
            for field, filename in [("stdoutBase64", "command.stdout"), ("stderrBase64", "command.stderr")]:
                require(decode(effects[field]) == (case / filename).read_bytes() and len(decode(effects[field])) <= 16 * 1024**2,
                        "Raw command output differs")
            paths, size_sum = [], 0
            for row in effects["dist"]:
                path = decode(row["pathBase64"])
                require((path or row["type"] == "directory") and not path.startswith(b"/") and b".." not in path.split(b"/") and b"\0" not in path,
                        "Invalid output path")
                paths.append(path)
                require(row["uid"] == row["gid"] == 1000 and 0 <= row["mode"] <= 0o7777, "Wrong output metadata")
                if row["type"] == "file":
                    data = decode(row["dataBase64"]);size_sum += len(data)
                    require(len(data) == row["bytes"] and hashlib.sha256(data).hexdigest() == row["sha256"], "Output content differs")
                elif row["type"] == "symlink":
                    require(b"\0" not in decode(row["targetBase64"]), "Invalid symlink target")
                else:
                    require(row["type"] == "directory", "Special output file")
            require(len(paths) == len(set(paths)) == effects["distEntries"] == result["distEntries"] <= 50000 and
                    size_sum == effects["distBytes"] == result["distBytes"] <= 48 * 1024**2, "Output tree count differs")
            if command == "boundary":
                native = json.loads(decode(effects["stdoutBase64"]))
                require(native["schema"] == "quod-native-vm-boundary-v1" and native["count"] == len(NAMES) and native["failed"] == 0 and
                        len(native["checks"]) == len(NAMES) and {r["name"] for r in native["checks"]} == NAMES and
                        all(r["pass"] is True for r in native["checks"]), "Native boundary controls differ")
            if command == "build":
                require(b"bin.cjs" in paths, "Actual build output missing")
            if command == "release":
                stdout = decode(effects["stdoutBase64"]).decode("utf-8", errors="strict")
                require(all(re.search(r"(?:# |ℹ )" + counter + r"(?:\s|$)", stdout) for counter in ["tests 20", "pass 20", "fail 0"]),
                        "Actual release terminal counters missing")
            invocation = load(case / "invocation.json")
            argv = invocation["argv"]
            kernel_path = argv[argv.index("-kernel")+1]
            serial_arg = argv[argv.index("-chardev")+1]
            require(Path(kernel_path).name == "kernel" and serial_arg.startswith("file,id=trusted-output,path=") and
                    serial_arg.endswith("/" + name + "/serial.raw"), "Wrong fixed input/output channel")
            remote_image = str(Path(kernel_path).parent)
            expected_argv = [metadata["qemu"]["path"], "-no-user-config", "-nodefaults", "-display", "none", "-monitor", "none",
                "-nic", "none", "-accel", "tcg,thread=single", "-machine", policy["machine"], "-cpu", policy["cpu"], "-smp", "1", "-m", "4096",
                "-uuid", policy["uuid"], "-seed", "42", "-icount", policy["icount"], "-rtc", policy["rtc"], "-L", metadata["qemu"]["dataDirectory"],
                "-bios", metadata["qemu"]["bios"], "-kernel", remote_image + "/kernel", "-initrd", remote_image + "/initramfs.cpio.gz",
                "-append", "init=/init rdinit=/init console=ttyS0,115200n8 quiet loglevel=0 nokaslr random.trust_cpu=off random.trust_bootloader=off panic=-1 oops=panic quod.command=" + command,
                "-chardev", serial_arg, "-device", "isa-serial,chardev=trusted-output,index=0", "-no-reboot"]
            require(argv == expected_argv and invocation["environment"] == policy["hostEnvironment"] and invocation["stdin"] == "DEVNULL" and
                    invocation["watchdogSeconds"] == watchdog and
                    invocation["imageMetadataSha256"] == sha(image / "image.json") and invocation["policySha256"] == sha(evidence / "policy.json"),
                    "Actual QEMU invocation differs from the fixed boundary")
            results[name], responses[name] = result, response
        for command in ["build", "release"]:
            a, b = command + "-1", command + "-2"
            require(responses[a]["effects"] == responses[b]["effects"] and
                    responses[a]["proof"]["entropyProbeBase64"] == responses[b]["proof"]["entropyProbeBase64"],
                    "Repeated actual command/entropy effects differ")
        retained = load(evidence / "files.json")
        actual_files = {p.relative_to(evidence).as_posix(): {"sha256": sha(p), "bytes": p.stat().st_size}
                        for p in sorted(evidence.rglob("*")) if p.is_file() and p.name != "files.json"}
        require(retained == actual_files and not any(p.is_symlink() for p in evidence.rglob("*")), "Retained evidence file manifest differs")
        report.update(ok=True, executionClassSha256=key, boundaryChecks=len(NAMES), expectedBackgroundRejections=1,
                      actualBuildExecutions=2, actualReleaseExecutions=2,
                      phaseSeconds={name: result["seconds"] for name, result in results.items()},
                      agreement={"build": True, "release": True, "kernelGetrandom": True},
                      files=actual_files, kernelSha256=metadata["kernel"]["sha256"], initramfsSha256=metadata["initramfs"]["sha256"])
    except Exception as exc:
        report["issues"].append(str(exc))
    print(json.dumps(report, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
