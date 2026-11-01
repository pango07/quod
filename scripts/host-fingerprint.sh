#!/usr/bin/env bash
# 0a environment pin. Its output is hashed into the step key's `image` field,
# so anything outside the served tree that can change a step's result goes
# here. Quod runs it with a cleared env and every toolchain's bin dir on
# PATH (quod_key::profile::HOST_PATH), so it resolves the same binaries the
# steps do. A toolchain that is not installed is recorded as absent; a step
# needing it fails on its own.
set -eu
export PATH=/usr/local/go/bin:/usr/local/node/bin:/usr/local/python/bin:/usr/local/uv/bin:/usr/bin:/bin
export GOTOOLCHAIN=local HOME=/nonexistent GOENV=off
# The sections run concurrently (hashing the node binary alone takes ~0.2s,
# the whole script ~0.5s when sequential), each into its own file, and are
# printed in one fixed order: the output is the same bytes a sequential run
# prints. A section that fails fails the script.
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
pids=()
job() { local name=$1; shift; "$@" > "$out/$name" & pids+=($!); }
# Each toolchain: its version string and the hash of its binary (resolved
# through symlinks, so a venv-style launcher pins the real interpreter).
bin_hash() { sha256sum "$(readlink -f "$1")" | cut -c1-64; }
sec_go() {
  if go=$(command -v go); then
    echo "go $(go env GOVERSION) $(go env GOOS)/$(go env GOARCH) GOROOT=$(go env GOROOT)"
    echo "go binary $(bin_hash "$go")"
  else
    echo "go absent"
  fi
}
sec_node() {
  if node=$(command -v node); then
    echo "node $(node --version) npm $(npm --version 2>/dev/null || echo absent)"
    echo "node binary $(bin_hash "$node")"
  else
    echo "node absent"
  fi
}
sec_python() {
  if py=$(command -v python3); then
    # -B: this probe must not write stdlib pycs into the install it pins.
    echo "python $(python3 -I -B -c 'import sys,sysconfig; print(sys.version.split()[0], sys.implementation.cache_tag, sysconfig.get_platform())')"
    echo "python binary $(bin_hash "$py")"
  else
    echo "python absent"
  fi
}
sec_uv() {
  if uv=$(command -v uv); then
    echo "uv $(uv --version) binary $(bin_hash "$uv")"
  else
    echo "uv absent"
  fi
}
job 01-go sec_go
job 02-node sec_node
job 03-python sec_python
job 04-uv sec_uv
# A toolchain is more than its binary: npm is JavaScript under
# /usr/local/node/lib, Python's stdlib is .py files under /usr/local/python/lib,
# GOROOT holds the standard library's sources. None of it is in the served
# tree. Each install root's listing (path, type, size, mtime, mode, link
# target; for a directory only its path and mode, its entries are listed
# anyway) is hashed, so any change to an installed toolchain file changes
# every key. `__pycache__` directories are left out of the listing: the
# stripped Python build ships no stdlib pycs, so any root python run outside
# the sandbox writes some (fresh mtimes, and a directory mtime bump).
# Of those pycs, timestamp and checked-hash ones are validated against their
# (pinned) source on every import; an unchecked-hash pyc (PEP 552 flags=1) is
# trusted as is, so those are pinned by content. The same walk lists them,
# and Python starts only if there are any.
pycs=$out/pycs
sec_tree() {
  local t=$1
  if [ -d "$t" ]; then
    echo "tree $t $(find "$t" \( -type d -name __pycache__ \) \
      -o \( -path '*/__pycache__/*' -name '*.pyc' -type f -fprint0 "$pycs.${t##*/}" \) \
      -o -path '*/__pycache__/*' \
      -o -type d -printf '%P\td\t%m\n' \
      -o -printf '%P\t%y\t%s\t%T@\t%m\t%l\n' | LC_ALL=C sort | sha256sum | cut -c1-64)"
  fi
}
job 05-tree-go sec_tree /usr/local/go
job 06-tree-node sec_tree /usr/local/node
job 07-tree-python sec_tree /usr/local/python
job 08-tree-uv sec_tree /usr/local/uv
# Host binaries outside the served tree (ssh, git, coreutils) are inputs the
# observer cannot see, so every installed package version is pinned here.
sec_packages() {
  if command -v dpkg-query >/dev/null; then
    echo "packages $(dpkg-query -W -f '${Package}=${Version}\n' | sort | sha256sum | cut -c1-64)"
  fi
}
job 12-packages sec_packages
# /etc is bound read-only into the sandbox (gitconfig, ssl, alternatives).
# Files the container runtime rewrites per container are left out; the
# sandbox replaces passwd and group with its own. One line per entry,
# "path hash" for a file and "path -> target" for a symlink, sorted; the
# files go through one sha256sum (a process per file cost ~0.5s a run).
etc_find() {
  find /etc -readable \
    ! -path /etc/hostname ! -path /etc/hosts ! -path /etc/resolv.conf \
    ! -path /etc/mtab ! -path /etc/machine-id ! -path /etc/passwd ! -path /etc/group \
    ! -path '/etc/shadow*' ! -path '/etc/gshadow*' "$@" 2>/dev/null
}
sec_etc() {
  echo "etc $({ etc_find -type l -printf '%p -> %l\n'
    etc_find -type f -print0 | xargs -0 -r sha256sum | sed 's/^\([0-9a-f]\{64\}\)  \(.*\)$/\2 \1/'; } \
    | LC_ALL=C sort | sha256sum | cut -c1-64)"
}
job 13-etc sec_etc
for p in "${pids[@]}"; do wait "$p"; done
# After the tree walks, which list the pycs.
if [ -n "$(cat "$pycs".* 2>/dev/null | head -c1)" ]; then
  echo "unchecked-pycs $(cat "$pycs".* | /usr/local/python/bin/python3 -I -B -c '
import hashlib, sys
for p in sorted(sys.stdin.buffer.read().split(b"\0")):
    if p:
        with open(p, "rb") as f:
            head = f.read(16)
            if len(head) >= 8 and int.from_bytes(head[4:8], "little") & 3 == 1:
                print(p.decode(), hashlib.sha256(head + f.read()).hexdigest())
' | sha256sum | cut -c1-64)" > "$out/09-pycs"
else
  echo "unchecked-pycs $(printf '' | sha256sum | cut -c1-64)" > "$out/09-pycs"
fi
{
  echo "glibc $(getconf GNU_LIBC_VERSION 2>/dev/null || ldd --version 2>&1 | head -1)"
  echo "arch $(uname -m)"
  if [ -r /etc/os-release ]; then . /etc/os-release; echo "distro ${ID:-unknown} ${VERSION_ID:-unknown}"; fi
} > "$out/10-misc"
cat "$out"/[0-9][0-9]-*
