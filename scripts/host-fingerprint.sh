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
  echo "python $(python3 -c 'import sys,sysconfig; print(sys.version.split()[0], sys.implementation.cache_tag, sysconfig.get_platform())')"
  echo "python binary $(bin_hash "$py")"
else
  echo "python absent"
fi
if uv=$(command -v uv); then
  echo "uv $(uv --version) binary $(bin_hash "$uv")"
else
  echo "uv absent"
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
# sandbox replaces passwd and group with its own.
echo "etc $(find /etc \( -type f -o -type l \) -readable \
  ! -path /etc/hostname ! -path /etc/hosts ! -path /etc/resolv.conf \
  ! -path /etc/mtab ! -path /etc/machine-id ! -path /etc/passwd ! -path /etc/group \
  ! -path '/etc/shadow*' ! -path '/etc/gshadow*' 2>/dev/null | LC_ALL=C sort \
  | while read -r f; do if [ -L "$f" ]; then echo "$f -> $(readlink "$f")"; else echo "$f $(sha256sum < "$f" | cut -c1-64)"; fi; done \
  | sha256sum | cut -c1-64)"
