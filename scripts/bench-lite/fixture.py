#!/usr/bin/env python3
"""bench-lite's fixture: a small Go + Node + Python monorepo and its scripted
history, generated deterministically (fixed names, dates and contents, so
every commit hash is the same on every machine). No network.

    fixture.py DIR      writes the repo (DIR/src), the registry the network
                        step fetches from (DIR/registry) and DIR/jobs.json,
                        the jobs bench-lite runs: [{i, scenario, branch,
                        commit, note}]

Layout of the repo (no toolchain marker at the top, so every step names its
toolchain, as a monorepo's CI does):

  go/        module example.com/lite: shared/ (imported by alpha, beta,
             gamma), delta, eps (standalone), gitinfo (its test runs git in
             the checkout, as cli/cli's tests read .git/HEAD)
  node/      a workspace of four packages (core <- fmt, stats <- cli) and a
             vendored zero-dependency helper (vendor/fnv.js); `node --test`
  py/        package lite (geom, text, calc, store), unittest tests, a
             requirements.lock with hashes and the wheels it pins
             (py/wheels/fixture_dep-*.whl), installed with pip --no-index
  tools/     fetch.py, the network step: downloads index.json from the
             registry on 127.0.0.1 and checks it against registry.sha256
  docs/      read by nothing

Every source file carries `Rev = N`; a test prints a checksum that depends on
the Rev of everything it imports, so a stale replay prints something a plain
run does not.
"""
import base64
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import zipfile

PORT = 18765
DATE0 = 1793500000  # 2026-11-01, fixed: commit hashes do not depend on the clock

GO_SHARED = ["alpha", "beta", "gamma"]
GO_ALONE = ["delta", "eps"]
NODE_PKGS = ["core", "fmt", "stats", "cli"]
PY_MODS = ["geom", "text", "calc", "store"]


# ---- the wheel the Python packages depend on ---------------------------------

def wheel(version):
    """A pure-Python wheel, byte for byte the same on every run."""
    name, dist = "fixture_dep", f"fixture_dep-{version}.dist-info"
    major, minor, patch = (int(x) for x in version.split("."))
    files = {
        f"{name}/__init__.py": (
            f'VERSION = "{version}"\n\n\n'
            f"def mix(x):\n"
            f"    x ^= x >> 13\n"
            f"    x = (x * 0x5BD1E995 + {major * 100 + minor * 10 + patch}) & 0xFFFFFFFF\n"
            f"    return x ^ (x >> 15)\n"
        ),
        f"{dist}/METADATA": f"Metadata-Version: 2.1\nName: fixture-dep\nVersion: {version}\n",
        f"{dist}/WHEEL": "Wheel-Version: 1.0\nGenerator: quod-bench-lite\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
    }
    record = []
    for path, text in files.items():
        data = text.encode()
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
        record.append(f"{path},sha256={digest},{len(data)}")
    record.append(f"{dist}/RECORD,,")
    files[f"{dist}/RECORD"] = "\n".join(record) + "\n"
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        for path, text in files.items():
            info = zipfile.ZipInfo(path, date_time=(1980, 1, 1, 0, 0, 0))
            info.external_attr = 0o644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, text)
    return buf.getvalue()


def wheel_name(version):
    return f"fixture_dep-{version}-py3-none-any.whl"


# ---- sources -------------------------------------------------------------------

def go_files():
    f = {"go/go.mod": "module example.com/lite\n\ngo 1.24\n"}
    f["go/shared/shared.go"] = """package shared

// Rev changes when the shared library does.
const Rev = 1

// Mix is a 64-bit finalizer.
func Mix(x uint64) uint64 {
	x ^= x >> 33
	x *= 0xff51afd7ed558ccd
	x ^= x >> 33
	return x + Rev
}

// Work chains Mix n times: real CPU work for the tests.
func Work(seed uint64, n int) uint64 {
	h := seed
	for i := 0; i < n; i++ {
		h = Mix(h ^ uint64(i))
	}
	return h
}
"""
    f["go/shared/shared_test.go"] = """package shared

import "testing"

func TestWork(t *testing.T) {
	v := Work(1, 60_000_000)
	t.Logf("shared rev %d checksum %x", Rev, v)
	if v == 0 {
		t.Fatal("zero")
	}
}
"""
    for k, p in enumerate(GO_SHARED):
        f[f"go/{p}/{p}.go"] = f"""package {p}

import "example.com/lite/shared"

const Rev = 1

func Compute(n int) uint64 {{ return shared.Work({k + 2}, n) + Rev }}
"""
        f[f"go/{p}/{p}_test.go"] = f"""package {p}

import (
	"testing"

	"example.com/lite/shared"
)

func TestCompute(t *testing.T) {{
	v := Compute(120_000_000)
	t.Logf("{p} rev %d shared rev %d checksum %x", Rev, shared.Rev, v)
	if v == 0 {{
		t.Fatal("zero")
	}}
}}
"""
    f["go/delta/delta.go"] = """package delta

const Rev = 1

// Primes counts the primes below n with a sieve.
func Primes(n int) int {
	composite := make([]bool, n)
	c := 0
	for i := 2; i < n; i++ {
		if composite[i] {
			continue
		}
		c++
		for j := i * i; j < n; j += i {
			composite[j] = true
		}
	}
	return c + Rev - 1
}
"""
    f["go/delta/delta_test.go"] = """package delta

import "testing"

func TestPrimes(t *testing.T) {
	c := Primes(40_000_000)
	t.Logf("delta rev %d primes %d", Rev, c)
	if c < 1_000_000 {
		t.Fatal("too few")
	}
}
"""
    f["go/eps/eps.go"] = """package eps

import "crypto/sha256"

const Rev = 1

// Chain hashes its own output n times.
func Chain(n int) [32]byte {
	h := sha256.Sum256([]byte{Rev})
	for i := 0; i < n; i++ {
		h = sha256.Sum256(h[:])
	}
	return h
}
"""
    f["go/eps/eps_test.go"] = """package eps

import "testing"

func TestChain(t *testing.T) {
	h := Chain(2_000_000)
	t.Logf("eps rev %d digest %x", Rev, h[:8])
}
"""
    f["go/gitinfo/gitinfo.go"] = """package gitinfo

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

const Rev = 1

// Branch is the checkout's current branch ref: git finds the repository,
// then .git/HEAD names the branch (as cli/cli's tests read it).
func Branch() (string, error) {
	out, err := exec.Command("git", "rev-parse", "--git-dir").Output()
	if err != nil {
		return "", err
	}
	head, err := os.ReadFile(filepath.Join(strings.TrimSpace(string(out)), "HEAD"))
	return strings.TrimPrefix(strings.TrimSpace(string(head)), "ref: "), err
}
"""
    f["go/gitinfo/gitinfo_test.go"] = """package gitinfo

import (
	"strings"
	"testing"
)

// Runs git in the checkout: the step reads .git/HEAD, which differs between
// a pull request's head (branch pr) and its merge (branch main).
func TestBranch(t *testing.T) {
	b, err := Branch()
	if err != nil {
		t.Fatalf("git: %v", err)
	}
	if !strings.HasPrefix(b, "refs/heads/") {
		t.Fatalf("not on a branch: %q", b)
	}
	t.Logf("gitinfo rev %d: on a branch", Rev)
}
"""
    return f


def node_files():
    f = {
        "node/package.json": json.dumps({"name": "lite-node", "private": True, "type": "module",
                                         "workspaces": [f"packages/{p}" for p in NODE_PKGS],
                                         "scripts": {"test": "node --test"}}, indent=2) + "\n",
        "node/package-lock.json": json.dumps({"name": "lite-node", "lockfileVersion": 3, "requires": True,
                                              "packages": {"": {"name": "lite-node", "workspaces": [f"packages/{p}" for p in NODE_PKGS]}}},
                                             indent=2) + "\n",
        "node/vendor/fnv.js": """// FNV-1a, 32-bit (vendored, zero dependencies).
export function fnv1a(n) {
  let h = 0x811c9dc5;
  for (let i = 0; i < 4; i++) {
    h ^= (n >>> (i * 8)) & 0xff;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h;
}
""",
    }
    deps = {"core": [], "fmt": ["core"], "stats": ["core"], "cli": ["fmt", "stats"]}
    for k, p in enumerate(NODE_PKGS):
        imports = "".join(f"import * as {d} from '../../{d}/src/index.js';\n" for d in deps[p])
        if p == "core":
            imports = "import { fnv1a } from '../../../vendor/fnv.js';\n"
            body = "  let h = seed >>> 0;\n  for (let i = 0; i < n; i++) h = fnv1a(h ^ i) + REV >>> 0;\n  return h;"
        else:
            body = "  let h = seed >>> 0;\n" + "".join(f"  h = {d}.work(h + {k}, n) ^ {d}.REV;\n" for d in deps[p]) + "  return (h + REV) >>> 0;"
        f[f"node/packages/{p}/package.json"] = json.dumps({"name": f"@lite/{p}", "version": "1.0.0", "type": "module",
                                                           "main": "src/index.js"}, indent=2) + "\n"
        f[f"node/packages/{p}/src/index.js"] = f"{imports}\nexport const REV = 1;\n\nexport function work(seed, n) {{\n{body}\n}}\n"
        f[f"node/packages/{p}/test/{p}.test.js"] = f"""import {{ test }} from 'node:test';
import assert from 'node:assert/strict';
import {{ work, REV }} from '../src/index.js';

test('{p} work', () => {{
  const v = work({k + 7}, 16_000_000);
  console.log(`{p} rev ${{REV}} checksum ${{v.toString(16)}}`);
  assert.ok(Number.isInteger(v));
}});
"""
    return f


def py_files(dep_version):
    f = {
        "py/pyproject.toml": '[project]\nname = "lite"\nversion = "1.0.0"\nrequires-python = ">=3.12"\ndependencies = ["fixture-dep"]\n',
        "py/lite/__init__.py": "",
        "py/tests/__init__.py": "",
    }
    for k, m in enumerate(PY_MODS):
        f[f"py/lite/{m}.py"] = f"""import fixture_dep

REV = 1


def work(seed, n):
    h = seed
    for i in range(n):
        h = fixture_dep.mix(h ^ i) + REV
    return h & 0xFFFFFFFF
"""
        f[f"py/tests/test_{m}.py"] = f"""import unittest

import fixture_dep

from lite import {m}


class Test{m.capitalize()}(unittest.TestCase):
    def test_work(self):
        v = {m}.work({k + 3}, 1_000_000)
        print(f"{m} rev {{{m}.REV}} dep {{fixture_dep.VERSION}} checksum {{v:x}}", flush=True)
        self.assertGreaterEqual(v, 0)
"""
    return f


def lock(version):
    digest = hashlib.sha256(wheel(version)).hexdigest()
    return f"fixture-dep=={version} \\\n    --hash=sha256:{digest}\n"


FETCH = f'''"""The network step: fetch the registry index from the loopback server
bench-lite runs and check it against the pinned digest."""
import hashlib
import pathlib
import urllib.request

URL = "http://127.0.0.1:{PORT}/index.json"
want = pathlib.Path("tools/registry.sha256").read_text().split()[0]
opener = urllib.request.build_opener(urllib.request.ProxyHandler({{}}))
data = opener.open(URL, timeout=10).read()
got = hashlib.sha256(data).hexdigest()
if got != want:
    raise SystemExit(f"registry index sha256 {{got}}, pinned {{want}}")
out = pathlib.Path(".quod/fetched")
out.mkdir(parents=True, exist_ok=True)
(out / "index.json").write_bytes(data)
print(f"fetched {{len(data)}} bytes, sha256 {{got[:16]}}")
'''


def registry_index():
    entries = [{"name": f"pkg{i}", "version": f"1.{i % 7}.{i % 3}",
                "sha256": hashlib.sha256(f"pkg{i}".encode()).hexdigest()} for i in range(400)]
    return (json.dumps({"entries": entries}, indent=1, sort_keys=True) + "\n").encode()


# ---- the repo and its history ---------------------------------------------------

class Repo:
    def __init__(self, path):
        self.path = path
        self.n = 0

    def git(self, *args):
        env = dict(os.environ, GIT_AUTHOR_NAME="bench-lite", GIT_AUTHOR_EMAIL="bench-lite@quod.invalid",
                   GIT_COMMITTER_NAME="bench-lite", GIT_COMMITTER_EMAIL="bench-lite@quod.invalid",
                   GIT_AUTHOR_DATE=f"{DATE0 + self.n * 60} +0000", GIT_COMMITTER_DATE=f"{DATE0 + self.n * 60} +0000",
                   GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null")
        return subprocess.run(["git", "-C", self.path, *args], check=True, text=True,
                              capture_output=True, env=env).stdout.strip()

    def write(self, rel, content):
        p = os.path.join(self.path, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "wb") as fh:
            fh.write(content if isinstance(content, bytes) else content.encode())

    def read(self, rel):
        with open(os.path.join(self.path, rel)) as fh:
            return fh.read()

    def bump(self, rel, pattern=r"(Rev|REV) = (\d+)"):
        text = self.read(rel)
        new, k = re.subn(pattern, lambda m: f"{m.group(1)} = {int(m.group(2)) + 1}", text, count=1)
        assert k == 1, rel
        self.write(rel, new)

    def commit(self, msg):
        self.n += 1
        self.git("add", "-A")
        self.git("commit", "-q", "-m", msg)
        return self.git("rev-parse", "HEAD")


def build(out):
    src = os.path.join(out, "src")
    if os.path.exists(src):
        shutil.rmtree(src)
    os.makedirs(src)
    r = Repo(src)
    r.git("init", "-q", "-b", "main")
    files = {"README.md": "# lite\n\nbench-lite's fixture monorepo (scripts/bench-lite/fixture.py).\n",
             "docs/guide.md": "# Guide\n\nNothing reads this file.\n",
             "tools/fetch.py": FETCH,
             "tools/registry.sha256": hashlib.sha256(registry_index()).hexdigest() + "  index.json\n",
             "py/requirements.lock": lock("1.0.0"),
             f"py/wheels/{wheel_name('1.0.0')}": wheel("1.0.0")}
    files.update(go_files())
    files.update(node_files())
    files.update(py_files("1.0.0"))
    for rel, content in files.items():
        r.write(rel, content)
    jobs = []

    def job(scenario, note, branch="main", commit=None):
        jobs.append(dict(i=len(jobs), scenario=scenario, branch=branch,
                         commit=commit or r.git("rev-parse", "HEAD"), note=note))

    job("init", "warm-up: the first commit, every step runs", commit=r.commit("init"))
    r.bump("go/alpha/alpha.go"); r.commit("go: alpha")
    job("go-pkg", "one Go package changed (alpha)")
    r.bump("go/shared/shared.go"); r.commit("go: shared lib")
    job("go-shared", "the shared Go lib changed (imported by alpha, beta, gamma)")
    r.write("docs/guide.md", r.read("docs/guide.md") + "\nA docs-only change.\n"); r.commit("docs")
    job("docs-only", "docs/ only")
    r.bump("node/packages/stats/src/index.js"); r.commit("node: stats")
    job("node-pkg", "one Node package changed (stats)")
    r.bump("py/lite/geom.py"); r.commit("py: geom")
    job("py-src", "Python source changed (geom)")
    r.write(f"py/wheels/{wheel_name('1.1.0')}", wheel("1.1.0"))
    r.write("py/requirements.lock", lock("1.1.0")); r.commit("py: fixture-dep 1.1.0")
    job("dep-bump", "lockfile bump: fixture-dep 1.0.0 -> 1.1.0 (the install step installs another wheel)")
    r.git("checkout", "-q", "-b", "pr")
    r.bump("go/beta/beta.go"); r.bump("node/packages/fmt/src/index.js")
    head = r.commit("pr: beta and fmt")
    job("pr-head", "pull request head on branch pr (Go beta, Node fmt)", branch="pr", commit=head)
    r.git("checkout", "-q", "main")
    r.n += 1
    r.git("merge", "-q", "--no-ff", "-m", "Merge branch pr", "pr")
    assert r.git("rev-parse", "HEAD^{tree}") == r.git("rev-parse", "pr^{tree}")
    job("merge", "--no-ff merge of pr into main: the same tree as the PR head, on main")
    job("rerun", "the merge job re-run: an identical commit")
    r.bump("py/lite/text.py"); r.commit("py: text")
    job("py-src-2", "Python source changed (text)")
    r.n += 1
    r.git("revert", "--no-edit", "HEAD")
    job("revert", "revert of py-src-2: the tree of the merge again")
    r.write("misc/notes.txt", "Nothing reads this directory.\n"); r.commit("misc: notes")
    job("unread", "a new file under misc/, which no step reads")
    r.bump("go/delta/delta.go"); r.commit("go: delta")
    job("go-pkg-2", "one standalone Go package changed (delta)")
    r.bump("node/packages/core/src/index.js"); r.commit("node: core")
    job("node-shared", "the shared Node package changed (core; fmt, stats, cli import it)")
    r.write("py/tests/test_extra.py", "import unittest\n\nfrom lite import calc\n\n\nclass TestExtra(unittest.TestCase):\n"
            "    def test_calc_small(self):\n        self.assertEqual(calc.work(1, 0), 1)\n")
    r.commit("py: a new test")
    job("py-test-add", "a Python test added")
    r.bump("go/eps/eps.go"); r.bump("py/lite/calc.py"); r.commit("go and py: eps, calc")
    job("mixed", "Go (eps) and Python (calc) in one commit")
    r.bump("go/shared/shared.go"); r.commit("go: shared lib again")
    job("go-shared-2", "the shared Go lib changed again")
    reg = os.path.join(out, "registry")
    os.makedirs(reg, exist_ok=True)
    with open(os.path.join(reg, "index.json"), "wb") as fh:
        fh.write(registry_index())
    with open(os.path.join(out, "jobs.json"), "w") as fh:
        json.dump(jobs, fh, indent=1)
    return jobs


if __name__ == "__main__":
    for j in build(sys.argv[1]):
        print(f"{j['i']:2} {j['scenario']:12} {j['branch']:4} {j['commit'][:10]}  {j['note']}")
