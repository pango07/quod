#!/usr/bin/env bash
# Runs a command in the privileged Linux dev container (0a needs FUSE,
# overlayfs and user namespaces). Build output and the spike repo live in
# named volumes so they stay off the slow host bind mount.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$root/versions.toml"; }
img=quod-dev:go$(v go version)-rust$(v rust toolchain)-node$(v node version)-py$(v python version)-uv$(v uv version)
if ! docker image inspect "$img" >/dev/null 2>&1; then
  docker build -q -t "$img" -f "$root/Dockerfile.dev" \
    --build-arg GO_VERSION="$(v go version)" --build-arg RUST_TOOLCHAIN="$(v rust toolchain)" \
    --build-arg NODE_VERSION="$(v node version)" --build-arg PYTHON_VERSION="$(v python version)" \
    --build-arg PBS_RELEASE="$(v python pbs_release)" --build-arg UV_VERSION="$(v uv version)" "$root" >/dev/null
fi
tty=; [ -t 0 ] && tty=-it
# QUOD_DEV_NS gives parallel worktrees their own build and work volumes.
# The volumes keep their names from before the rename to Quod (stillpass-*),
# so build caches, corpora and state carry over; every script that mounts
# them (dev-host2, build-rootfs, portability, runner/lab) uses the same names.
ns=${QUOD_DEV_NS:+-$QUOD_DEV_NS}
exec docker run --rm $tty --privileged --device /dev/fuse \
  -v "$root":/src -v stillpass-target$ns:/src/target -v stillpass-work$ns:/work \
  -e CARGO_TARGET_DIR=/src/target -e TMPDIR=/work/tmp -e QUOD_HOME=/work/state -e QUOD_INTEGRATION="${QUOD_INTEGRATION:-1}" \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" \
  "$img" bash -c "mkdir -p /work/tmp && $*"
