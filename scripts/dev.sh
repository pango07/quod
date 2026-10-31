#!/usr/bin/env bash
# Runs a command in the privileged Linux dev container (0a needs FUSE,
# overlayfs and user namespaces). Build output and the spike repo live in
# named volumes so they stay off the slow host bind mount.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$root/versions.toml"; }
img=stillpass-dev:go$(v go version)-rust$(v rust toolchain)
if ! docker image inspect "$img" >/dev/null 2>&1; then
  docker build -q -t "$img" -f "$root/Dockerfile.dev" \
    --build-arg GO_VERSION="$(v go version)" --build-arg RUST_TOOLCHAIN="$(v rust toolchain)" "$root" >/dev/null
fi
tty=; [ -t 0 ] && tty=-it
# STILLPASS_DEV_NS gives parallel worktrees their own build and work volumes.
ns=${STILLPASS_DEV_NS:+-$STILLPASS_DEV_NS}
exec docker run --rm $tty --privileged --device /dev/fuse \
  -v "$root":/src -v stillpass-target$ns:/src/target -v stillpass-work$ns:/work \
  -e CARGO_TARGET_DIR=/src/target -e TMPDIR=/work/tmp -e STILLPASS_HOME=/work/state -e STILLPASS_INTEGRATION="${STILLPASS_INTEGRATION:-1}" \
  "$img" bash -c "mkdir -p /work/tmp && $*"
