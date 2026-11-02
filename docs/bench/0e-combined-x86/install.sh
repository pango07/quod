#!/usr/bin/env bash
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get install -y -q --no-install-recommends bubblewrap ca-certificates curl fuse3 git build-essential pkg-config \
  sqlite3 util-linux procps time openssh-client jq linux-tools-common hyperfine 2>&1 | tail -3 || \
apt-get install -y -q --no-install-recommends bubblewrap ca-certificates curl fuse3 git build-essential pkg-config sqlite3 util-linux procps time openssh-client jq
GO=1.27.3; NODE=24.22.2; PY=3.12.16; PBS=20261026; UV=0.13.6
curl -fsSL "https://go.dev/dl/go${GO}.linux-amd64.tar.gz" -o /tmp/go.tgz
echo "65efbc80fdc1fa2cd2631cad5b481777a68e8725f34e89f50fc6ac714e379b40  /tmp/go.tgz" | sha256sum -c
rm -rf /usr/local/go && tar -C /usr/local -xzf /tmp/go.tgz
curl -fsSL "https://nodejs.org/dist/v${NODE}/node-v${NODE}-linux-x64.tar.gz" | tar -C /usr/local -xz
rm -rf /usr/local/node; mv /usr/local/node-v${NODE}-linux-x64 /usr/local/node
curl -fsSL "https://github.com/astral-sh/python-build-standalone/releases/download/${PBS}/cpython-${PY}+${PBS}-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz" | tar -C /usr/local -xz
mkdir -p /usr/local/uv/bin
curl -fsSL "https://github.com/astral-sh/uv/releases/download/${UV}/uv-x86_64-unknown-linux-gnu.tar.gz" | tar -C /usr/local/uv/bin -xz --strip-components=1
export RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.98.0 -c rustfmt -c clippy
grep -q '^user_allow_other' /etc/fuse.conf || echo user_allow_other >> /etc/fuse.conf
cat > /etc/profile.d/quod.sh <<'P'
export PATH=/usr/local/cargo/bin:/usr/local/go/bin:/usr/local/node/bin:/usr/local/python/bin:/usr/local/uv/bin:$PATH
export RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
P
. /etc/profile.d/quod.sh
go version; node --version; python3 --version; uv --version; cargo --version; bwrap --version; fusermount3 -V
echo INSTALL_DONE
