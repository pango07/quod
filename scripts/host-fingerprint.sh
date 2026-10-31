#!/usr/bin/env bash
# 0a environment pin. Its output is hashed into the step key's `image` field,
# so anything outside the served tree that can change a step's result goes
# here. Stillpass runs it with a cleared env and every toolchain's bin dir on
# PATH (sp_key::profile::HOST_PATH), so it resolves the same binaries the
# steps do. A toolchain that is not installed is recorded as absent; a step
# needing it fails on its own.
set -eu
export PATH=/usr/local/go/bin:/usr/local/node/bin:/usr/local/python/bin:/usr/local/uv/bin:/usr/bin:/bin
export GOTOOLCHAIN=local HOME=/nonexistent GOENV=off
# Each toolchain: its version string and the hash of its binary (resolved
# through symlinks, so a venv-style launcher pins the real interpreter).
bin_hash() { sha256sum "$(readlink -f "$1")" | cut -c1-64; }
if go=$(command -v go); then
  echo "go $(go env GOVERSION) $(go env GOOS)/$(go env GOARCH) GOROOT=$(go env GOROOT)"
  echo "go binary $(bin_hash "$go")"
else
  echo "go absent"
fi
if node=$(command -v node); then
  echo "node $(node --version) npm $(npm --version 2>/dev/null || echo absent)"
  echo "node binary $(bin_hash "$node")"
else
  echo "node absent"
fi
if py=$(command -v python3); then
  # -B: this probe must not write stdlib pycs into the install it pins.
  echo "python $(python3 -I -B -c 'import sys,sysconfig; print(sys.version.split()[0], sys.implementation.cache_tag, sysconfig.get_platform())')"
  echo "python binary $(bin_hash "$py")"
else
  echo "python absent"
fi
if uv=$(command -v uv); then
  echo "uv $(uv --version) binary $(bin_hash "$uv")"
else
  echo "uv absent"
fi
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
pycs=$(mktemp)
trap 'rm -f "$pycs" "$pycs".*' EXIT
for t in /usr/local/go /usr/local/node /usr/local/python /usr/local/uv; do
  if [ -d "$t" ]; then
    echo "tree $t $(find "$t" \( -type d -name __pycache__ \) \
      -o \( -path '*/__pycache__/*' -name '*.pyc' -type f -fprint0 "$pycs.${t##*/}" \) \
      -o -path '*/__pycache__/*' \
      -o -type d -printf '%P\td\t%m\n' \
      -o -printf '%P\t%y\t%s\t%T@\t%m\t%l\n' | LC_ALL=C sort | sha256sum | cut -c1-64)"
  fi
done
if [ -n "$(cat "$pycs".* 2>/dev/null | head -c1)" ]; then
  echo "unchecked-pycs $(cat "$pycs".* | /usr/local/python/bin/python3 -I -B -c '
import hashlib, sys
for p in sorted(sys.stdin.buffer.read().split(b"\0")):
    if p:
        with open(p, "rb") as f:
            head = f.read(16)
            if len(head) >= 8 and int.from_bytes(head[4:8], "little") & 3 == 1:
                print(p.decode(), hashlib.sha256(head + f.read()).hexdigest())
' | sha256sum | cut -c1-64)"
else
  echo "unchecked-pycs $(printf '' | sha256sum | cut -c1-64)"
fi
echo "glibc $(getconf GNU_LIBC_VERSION 2>/dev/null || ldd --version 2>&1 | head -1)"
echo "arch $(uname -m)"
if [ -r /etc/os-release ]; then . /etc/os-release; echo "distro ${ID:-unknown} ${VERSION_ID:-unknown}"; fi
# Host binaries outside the served tree (ssh, git, coreutils) are inputs the
# observer cannot see, so every installed package version is pinned here.
if command -v dpkg-query >/dev/null; then
  echo "packages $(dpkg-query -W -f '${Package}=${Version}\n' | sort | sha256sum | cut -c1-64)"
fi
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
echo "etc $({ etc_find -type l -printf '%p -> %l\n'
  etc_find -type f -print0 | xargs -0 -r sha256sum | sed 's/^\([0-9a-f]\{64\}\)  \(.*\)$/\2 \1/'; } \
  | LC_ALL=C sort | sha256sum | cut -c1-64)"
