#!/usr/bin/env bash
# The second host of the portability proof: runs a command in an Ubuntu 24.04
# dev container (Dockerfile.dev-ubuntu: other packages, other glibc, other
# /etc, no Go on the host) that shares exactly one thing with the first host
# (scripts/dev.sh): a state directory, mounted from the first host's work
# volume as /work/state (CAS, ledger, rootfs snapshots): the volume's
# `state` directory, or STILLPASS_SHARED=<subdir>. Its own build output,
# checkout and temp files live in volumes of its own.
#   STILLPASS_DEV_NS=rootfs scripts/dev-host2.sh '<command>'
# The first host must have created the shared directory.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$root/versions.toml"; }
img=stillpass-dev-ubuntu:rust$(v rust toolchain)
if ! docker image inspect "$img" >/dev/null 2>&1; then
  docker build -q -t "$img" -f "$root/Dockerfile.dev-ubuntu" \
    --build-arg RUST_TOOLCHAIN="$(v rust toolchain)" "$root" >/dev/null
fi
tty=; [ -t 0 ] && tty=-it
ns=${STILLPASS_DEV_NS:+-$STILLPASS_DEV_NS}
shared=${STILLPASS_SHARED:-state}
exec docker run --rm $tty --privileged --device /dev/fuse \
  -v "$root":/src -v "stillpass-target$ns-host2":/src/target -v "stillpass-work$ns-host2":/work \
  --mount "type=volume,src=stillpass-work$ns,dst=/work/state,volume-subpath=$shared" \
  -e CARGO_TARGET_DIR=/src/target -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" -e TMPDIR=/work/tmp \
  -e STILLPASS_HOME=/work/state -e STILLPASS_INTEGRATION="${STILLPASS_INTEGRATION:-1}" \
  "$img" bash -c "mkdir -p /work/tmp && $*"
