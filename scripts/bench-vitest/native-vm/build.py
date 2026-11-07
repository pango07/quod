#!/usr/bin/env python3
"""Build an inert, canonical, diskless image on an authorized GitHub Linux runner.

This is an investigation, not a receipt issuer. Never export the checkout's .git,
runner environment, home directory, or npm cache into the guest.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys

SCHEMA = "quod-native-vm-image-v1-investigation"
SOURCE = "5eaa7e5160736054d53f8b71e2c522835eb427e0"
EPOCH = 1700000000
HERE = Path(__file__).resolve().parent
DENIED = {".git", ".ssh", ".aws", ".netrc", ".git-credentials", ".docker", ".kube"}


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def run(argv, *, cwd=None, data=None):
    # No shell, and no inherited credentials enter any child environment.
    return subprocess.run(argv, cwd=cwd, input=data, check=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "TZ": "UTC"})


def resolve_guest_path(root, guest):
    """Resolve each hop inside the guest; never follow a guest absolute link on
    the host. The root itself is trusted, while all components are inspected by
    lstat before use. POSIX-like parent traversal must remain inside that root.
    """
    if not guest.startswith("/"):
        raise RuntimeError("Guest resolution must start at an absolute path")
    pending, resolved, hops = guest.split("/"), [], 0
    while pending:
        part = pending.pop(0)
        if part in {"", "."}:
            continue
        if part == "..":
            if not resolved:
                raise RuntimeError("Guest symlink escapes the root")
            resolved.pop()
            continue
        candidate = root.joinpath(*resolved, part)
        try:
            info = candidate.lstat()
        except OSError as exc:
            raise RuntimeError("Missing guest symlink target: " + guest) from exc
        if stat.S_ISLNK(info.st_mode):
            hops += 1
            if hops > 40:
                raise RuntimeError("Guest symlink cycle or hop limit")
            target = os.readlink(candidate)
            if target.startswith("/"):
                resolved = []
            pending = target.split("/") + pending
        else:
            if pending and not stat.S_ISDIR(info.st_mode):
                raise RuntimeError("Guest symlink crosses a non-directory")
            resolved.append(part)
    return root.joinpath(*resolved)


def remote_symlink_fixtures(out):
    """Cheap actual-filesystem checks executed only by the GitHub builder."""
    root = out / "symlink-fixtures"
    for directory in ["usr/bin", "usr/lib", "etc/alternatives"]:
        (root / directory).mkdir(parents=True, exist_ok=True)
    (root / "usr/bin/mawk").write_bytes(b"fixture target\n")
    links = {"usr/bin/awk": "/etc/alternatives/awk", "etc/alternatives/awk": "/usr/bin/mawk",
             "usr/bin/relative": "../lib/tool", "usr/lib/tool": "../bin/mawk",
             "parent": "/usr/../usr/bin/mawk", "dangling": "/missing/target",
             "cycle-a": "cycle-b", "cycle-b": "cycle-a", "escape": "../outside"}
    for name, target in links.items():
        (root / name).symlink_to(target)
    rows = []
    for name, path, accepted in [("absolute-through-absolute", "/usr/bin/awk", True),
                                  ("relative-chain", "/usr/bin/relative", True),
                                  ("legitimate-parent", "/parent", True),
                                  ("dangling-rejected", "/dangling", False),
                                  ("cycle-rejected", "/cycle-a", False),
                                  ("root-escape-rejected", "/escape", False)]:
        result = None
        try:
            result = resolve_guest_path(root, path)
        except RuntimeError:
            if accepted:
                raise
        if accepted:
            if result != root / "usr/bin/mawk":
                raise RuntimeError("Guest symlink acceptance fixture resolved incorrectly: " + name)
        elif result is not None:
            raise RuntimeError("Guest symlink rejection fixture unexpectedly accepted: " + name)
        rows.append({"name": name, "expected": "accepted" if accepted else "rejected", "passed": True})
    shutil.rmtree(root)
    (out / "symlink-fixtures.json").write_bytes(canonical({"schema": "quod-native-vm-symlink-fixtures-v1", "checks": rows}) + b"\n")
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("project", type=Path)
    ap.add_argument("out", type=Path)
    args = ap.parse_args()
    if sys.platform != "linux" or os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Image preparation is restricted to the authorized GitHub Linux gate")
    os.umask(0o022)
    project, out = args.project.resolve(), args.out.resolve()
    if out.exists():
        raise SystemExit("Output must not already exist")
    if run(["git", "rev-parse", "HEAD"], cwd=project).stdout.decode().strip() != SOURCE:
        raise SystemExit("Expected the original c11 source commit")
    if run(["git", "diff", "--name-only", "HEAD", "--"], cwd=project).stdout:
        raise SystemExit("Tracked source must match the original c11 commit exactly")
    if not (project / "node_modules").is_dir():
        raise SystemExit("Install the original locked dependencies in the GitHub gate first")
    node = Path(shutil.which("node") or "").resolve()
    if not node.is_file():
        raise SystemExit("setup-node must supply the actual Node binary")
    npm_root = node.parent.parent / "lib/node_modules/npm"
    if not (npm_root / "bin/npm-cli.js").is_file():
        raise SystemExit("The actual setup-node npm tree is required")
    kernel_candidates = sorted(Path("/boot").glob("vmlinuz-*"))
    if not kernel_candidates:
        raise SystemExit("No installed Linux kernel; do not substitute a different executor")
    kernel = kernel_candidates[-1]
    qemu = Path(shutil.which("qemu-system-x86_64") or "").resolve()
    if not qemu.is_file():
        raise SystemExit("qemu-system-x86_64 is required")
    out.mkdir(parents=True)
    symlink_checks = remote_symlink_fixtures(out)
    stage = out / "rootfs"
    stage.mkdir()
    for alias, target in [("bin", "usr/bin"), ("sbin", "usr/sbin"), ("lib", "usr/lib"), ("lib64", "usr/lib64")]:
        (stage / target).mkdir(parents=True, exist_ok=True)
        (stage / alias).symlink_to(target)
    host_files, copied_inodes, elf_queue = {}, {}, []

    def inspect_private(relative, src):
        parts = Path(relative).parts
        if any(p in DENIED for p in parts):
            raise RuntimeError(f"Private metadata is forbidden in the image: {relative}")
        if Path(relative).name in {".npmrc", ".yarnrc", ".yarnrc.yml"} and src.is_file():
            content = src.read_bytes()
            if re.search(rb"(?i)(_auth|token|password|username|credential|certfile|keyfile)", content):
                raise RuntimeError(f"Authentication-bearing package configuration: {relative}")

    def copy_one(src, guest, *, private_check=False):
        src, guest = Path(src), Path(guest)
        if not guest.is_absolute() or ".." in guest.parts:
            raise RuntimeError("Non-canonical guest path")
        dst = stage / str(guest).lstrip("/")
        if private_check:
            inspect_private(str(guest), src)
        info = src.lstat()
        dst.parent.mkdir(parents=True, exist_ok=True)
        if stat.S_ISLNK(info.st_mode):
            target = os.readlink(src)
            if dst.exists() or dst.is_symlink():
                if not dst.is_symlink() or os.readlink(dst) != target:
                    raise RuntimeError(f"Conflicting guest symlink: {guest}")
            else:
                dst.symlink_to(target)
            return
        if stat.S_ISDIR(info.st_mode):
            dst.mkdir(exist_ok=True)
            for child in sorted(src.iterdir()):
                copy_one(child, guest / child.name, private_check=private_check)
            return
        if not stat.S_ISREG(info.st_mode):
            raise RuntimeError(f"Special input files are forbidden: {src}")
        if dst.exists():
            if dst.is_symlink() or digest(dst) != digest(src):
                raise RuntimeError(f"Conflicting guest file: {guest}")
            return
        inode = (info.st_dev, info.st_ino)
        previous = copied_inodes.get(inode)
        if previous is not None:
            os.link(previous, dst)
        else:
            shutil.copyfile(src, dst)
            copied_inodes[inode] = dst
        dst.chmod(0o755 if info.st_mode & 0o111 else 0o644)
        if dst.open("rb").read(4) == b"\x7fELF":
            elf_queue.append((src.resolve(), dst))

    def copy_system(src):
        """Keep interpreter/library absolute paths and every actual symlink hop."""
        src = Path(src)
        if not src.is_absolute():
            raise RuntimeError("System input must be absolute")
        src = Path(os.path.normpath(src))
        # Canonicalize parent aliases (/lib -> /usr/lib) without copying an
        # entire host directory or opening any runner-home/config tree.
        actual = src.parent.resolve() / src.name
        copy_one(actual, src)
        if actual.is_symlink():
            target = Path(os.readlink(actual))
            resolved_target = target if target.is_absolute() else actual.parent / target
            copy_system(resolved_target.absolute())
        elif actual.is_file():
            host_files[str(actual)] = {"sha256": digest(actual), "bytes": actual.stat().st_size}

    tracked = run(["git", "ls-files", "-z"], cwd=project).stdout.split(b"\0")
    source_rows = []
    for raw in tracked:
        if not raw:
            continue
        name = os.fsdecode(raw)
        rel = Path(name)
        if rel.is_absolute() or ".." in rel.parts or rel.parts[0] == "dist":
            raise RuntimeError(f"Unexpected tracked input: {name}")
        src = project / rel
        if src.is_dir() and not src.is_symlink():
            raise RuntimeError(f"Git submodules require a separate closed input policy: {name}")
        copy_one(src, Path("/project") / rel, private_check=True)
        source_rows.append({"path": name, "type": "symlink" if src.is_symlink() else "file",
                            "sha256": hashlib.sha256(os.readlink(src).encode()).hexdigest() if src.is_symlink() else digest(src)})
    copy_one(project / "node_modules", Path("/project/node_modules"), private_check=True)
    copy_one(node, Path("/tools/node/bin/node"))
    copy_one(npm_root, Path("/tools/node/lib/node_modules/npm"), private_check=True)
    for name, target in [("npm", "../lib/node_modules/npm/bin/npm-cli.js"),
                         ("npx", "../lib/node_modules/npm/bin/npx-cli.js")]:
        (stage / "tools/node/bin" / name).symlink_to(target)
    tools = ["/bin/sh", "/bin/bash", "/usr/bin/env", "/usr/bin/git", "/usr/bin/cat", "/usr/bin/cp",
             "/usr/bin/mv", "/usr/bin/rm", "/usr/bin/mkdir", "/usr/bin/rmdir", "/usr/bin/chmod",
             "/usr/bin/dirname", "/usr/bin/basename", "/usr/bin/readlink", "/usr/bin/sleep",
             "/usr/bin/printf", "/usr/bin/true", "/usr/bin/false", "/usr/bin/uname", "/usr/bin/which"]
    tools += [f"/usr/bin/{name}" for name in ["sed", "grep", "cut", "tr", "sort", "head", "tail", "wc", "touch",
                                             "ln", "find", "tee", "test", "expr", "awk", "dd", "date", "tar", "gzip", "ls"]]
    for tool in tools:
        copy_system(tool)
    copy_system("/usr/lib/git-core")
    if Path("/usr/share/git-core/templates").exists():
        copy_one(Path("/usr/share/git-core/templates"), Path("/usr/share/git-core/templates"))
    # NSS is dynamically loaded by glibc; name every allowed implementation.
    for pattern in ["libnss_files.so*", "libnss_dns.so*", "libresolv.so*"]:
        for lib in sorted(Path("/lib/x86_64-linux-gnu").glob(pattern)):
            copy_system(lib)
    ldconfig = run(["/sbin/ldconfig", "-p"]).stdout.decode()
    library_map = {}
    for line in ldconfig.splitlines():
        match = re.match(r"\s*(\S+)\s+\(.*x86-64.*\)\s+=>\s+(\S+)", line)
        if match:
            library_map.setdefault(match[1], Path(match[2]))
    seen_elf = set()
    while elf_queue:
        src, dst = elf_queue.pop(0)
        if str(src) in seen_elf:
            continue
        seen_elf.add(str(src))
        headers = run(["readelf", "-lW", str(src)]).stdout.decode()
        interp = re.search(r"Requesting program interpreter:\s*([^\]]+)\]", headers)
        if interp:
            copy_system(interp[1])
        dynamic = run(["readelf", "-dW", str(src)]).stdout.decode()
        needed = re.findall(r"\(NEEDED\).*\[([^\]]+)\]", dynamic)
        search = []
        for entry in re.findall(r"\((?:RUNPATH|RPATH)\).*\[([^\]]+)\]", dynamic):
            search.extend(Path(p.replace("${ORIGIN}", str(src.parent)).replace("$ORIGIN", str(src.parent))) for p in entry.split(":"))
        for name in needed:
            candidates = [folder / name for folder in search] + ([library_map[name]] if name in library_map else [])
            found = next((p for p in candidates if p.is_file()), None)
            if found is None:
                raise RuntimeError(f"Unresolved actual ELF input {name} for {src}")
            resolved = found.resolve()
            if resolved.is_relative_to(project):
                copy_one(resolved, Path("/project") / resolved.relative_to(project), private_check=True)
            elif resolved.is_relative_to(node.parent.parent):
                copy_one(resolved, Path("/tools/node") / resolved.relative_to(node.parent.parent), private_check=True)
            else:
                copy_system(found)

    for directory in ["dev", "proc", "sys", "tmp", "run", "project/dist", "trusted", "etc", "root"]:
        (stage / directory).mkdir(parents=True, exist_ok=True)
    (stage / "etc/passwd").write_text("root:x:0:0:root:/root:/bin/sh\nquod:x:1000:1000:quod:/tmp/home:/bin/sh\n")
    (stage / "etc/group").write_text("root:x:0:\nquod:x:1000:\n")
    (stage / "etc/nsswitch.conf").write_text("passwd: files\ngroup: files\nhosts: files\n")
    (stage / "etc/hosts").write_text("127.0.0.1 localhost quod-native-vm\n::1 localhost\n")
    (stage / "etc/hostname").write_text("quod-native-vm\n")
    (stage / "etc/gitconfig").write_text("[safe]\n\tdirectory = /project\n")
    compile_records = []
    for source, target in [("guest-init.c", stage / "init"), ("guest-negative.c", stage / "bin/quod-negative")]:
        target.parent.mkdir(parents=True, exist_ok=True)
        argv = ["gcc", "-std=c11", "-O2", "-static", "-Wall", "-Wextra", "-Werror", "-o", str(target), str(HERE / source)]
        compiled = run(argv)
        if source == "guest-init.c":
            run([str(target), "--self-test"])
        compile_records.append({"source": source, "sourceSha256": digest(HERE / source),
                                "outputSha256": digest(target), "argv": argv,
                                "stderr": compiled.stderr.decode(errors="replace"),
                                "sha256SelfTestPassed": source == "guest-init.c"})
    (stage / "init").chmod(0o700)
    (stage / "trusted").chmod(0o700)
    (stage / "root").chmod(0o700)
    guest_policy = {"schema": SCHEMA, "source": SOURCE, "epoch": EPOCH,
                    "contract": "diskless-canonical-linux-native-v1-investigation",
                    "project": "read-only input with initially-empty writable dist tmpfs",
                    "scratch": "ephemeral /tmp /run /dev/shm; no replay or receipt admission",
                    "commands": {"build": ["npm", "run", "build"], "release": ["npm", "run", "test:release"]},
                    "sourceInventorySha256": hashlib.sha256(canonical(source_rows)).hexdigest()}
    (stage / "trusted/policy.json").write_bytes(canonical(guest_policy))
    (stage / "trusted/policy.json").chmod(0o600)

    inventory = []
    for path in sorted(stage.rglob("*")):
        rel = path.relative_to(stage).as_posix()
        info = path.lstat()
        if path.is_symlink():
            target = os.readlink(path)
            resolve_guest_path(stage, "/" + rel)
            row = {"path": rel, "type": "symlink", "target": target, "mode": 0o777}
        elif path.is_dir():
            row = {"path": rel, "type": "directory", "mode": stat.S_IMODE(info.st_mode)}
        else:
            row = {"path": rel, "type": "file", "mode": stat.S_IMODE(info.st_mode), "bytes": info.st_size, "sha256": digest(path)}
        row.update(uid=0, gid=0, mtime=EPOCH)
        inventory.append(row)
        os.utime(path, (EPOCH, EPOCH), follow_symlinks=False)
    # Root ownership in the archive does not require host sudo. Deterministic
    # sorted names, inode renumbering and gzip mtime avoid staging metadata.
    names = b".\0" + b"".join(row["path"].encode() + b"\0" for row in inventory)
    stage.chmod(0o755)
    os.utime(stage, (EPOCH, EPOCH))
    cpio = run(["cpio", "--null", "-o", "--format=newc", "--reproducible", "--owner=0:0", "--quiet"], cwd=stage, data=names).stdout
    with (out / "initramfs.cpio.gz").open("wb") as raw:
        with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0, compresslevel=6) as compressed:
            compressed.write(cpio)
    shutil.copyfile(kernel, out / "kernel")

    # Retain and key the actual host emulator, data files, BIOS and module tree.
    # QEMU is run from its fixed installed path; all observed inputs are checked
    # unchanged immediately before and after every execution by probe.py.
    host_runtime = {str(qemu): {"sha256": digest(qemu), "bytes": qemu.stat().st_size}}
    roots = [Path("/usr/share/qemu"), Path("/usr/share/seabios"), Path("/usr/lib/x86_64-linux-gnu/qemu")]
    qemu_elf = [qemu]
    for root in roots:
        if root.exists():
            for path in sorted(root.rglob("*")):
                if path.is_file():
                    host_runtime[str(path)] = {"sha256": digest(path), "bytes": path.stat().st_size}
                    if path.open("rb").read(4) == b"\x7fELF":
                        qemu_elf.append(path)
    # ldd is used only on administrator-installed QEMU/modules, never project
    # executables. Its complete text is retained along with every named file.
    ldd_rows = []
    for executable in qemu_elf:
        result = run(["ldd", str(executable)]).stdout.decode()
        ldd_rows.append({"path": str(executable), "output": result})
        for name in re.findall(r"(?:=>\s+)?(/[\w./+:-]+)\s+\(", result):
            path = Path(name)
            host_runtime[str(path)] = {"sha256": digest(path), "bytes": path.stat().st_size}
    for name in ["/etc/ld.so.cache", "/etc/ld.so.preload"]:
        path = Path(name)
        host_runtime[name] = ({"sha256": digest(path), "bytes": path.stat().st_size}
                              if path.exists() else {"absent": True})
    machines = run([str(qemu), "-machine", "help"]).stdout.decode()
    candidates = re.findall(r"^(pc-i440fx-[0-9]+\.[0-9]+)\s", machines, re.M)
    if not candidates:
        raise RuntimeError("An explicit versioned i440fx machine is required")
    machine = sorted(candidates, key=lambda s: tuple(map(int, s.rsplit("-", 1)[1].split("."))))[-1]
    bios = Path("/usr/share/seabios/bios-256k.bin")
    if not bios.is_file() or str(bios) not in host_runtime:
        raise RuntimeError("The explicit hashed SeaBIOS image is required")
    metadata = {"schema": SCHEMA, "contract": guest_policy["contract"], "source": SOURCE,
                "rootMetadata": {"mode": 0o755, "uid": 0, "gid": 0, "mtime": EPOCH},
                "symlinkChecks": symlink_checks,
                "sourceInventory": source_rows, "guestPolicy": guest_policy, "inventory": inventory,
                "inventorySha256": hashlib.sha256(canonical(inventory)).hexdigest(),
                "kernel": {"origin": str(kernel), "sha256": digest(out / "kernel"), "bytes": (out / "kernel").stat().st_size},
                "initramfs": {"sha256": digest(out / "initramfs.cpio.gz"), "bytes": (out / "initramfs.cpio.gz").stat().st_size},
                "qemu": {"path": str(qemu), "version": run([str(qemu), "--version"]).stdout.decode(), "machine": machine,
                         "bios": str(bios), "dataDirectory": "/usr/share/qemu", "hostFiles": host_runtime, "ldd": ldd_rows},
                "hostInputFiles": host_files, "compile": compile_records,
                "node": {"origin": str(node), "sha256": digest(node), "version": run([str(node), "--version"]).stdout.decode().strip()},
                "builderSha256": digest(Path(__file__)), "guestInitSourceSha256": digest(HERE / "guest-init.c"),
                "negativeSourceSha256": digest(HERE / "guest-negative.c"),
                "packages": run(["dpkg-query", "-W", "-f=${Package}\t${Version}\n", "qemu-system-x86", "linux-image-virtual", "gcc", "libc6", "cpio"]).stdout.decode(),
                "receiptAdmission": False}
    (out / "image.json").write_bytes(canonical(metadata) + b"\n")
    # Keep the actual image and entry inventory, not a redundant unpacked tree.
    shutil.rmtree(stage)
    print(json.dumps({"image": str(out), "kernelSha256": metadata["kernel"]["sha256"],
                      "initramfsSha256": metadata["initramfs"]["sha256"], "entries": len(inventory), "receiptAdmission": False}))


if __name__ == "__main__":
    main()
