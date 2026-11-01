#!/usr/bin/env bash
# The runner lab: one long-lived privileged container (the dev box image plus
# the runner's native dependencies) so a registered actions/runner can stay up
# across commands. Same volumes as scripts/dev.sh under QUOD_DEV_NS.
#   scripts/runner/lab.sh up            # start (or restart) the container
#   scripts/runner/lab.sh exec <cmd>    # run a command inside it
#   scripts/runner/lab.sh down          # stop and remove it
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$root/versions.toml"; }
tag=go$(v go version)-rust$(v rust toolchain)
base=quod-dev:$tag
img=quod-runner-dev:$tag
ns=${QUOD_DEV_NS:+-$QUOD_DEV_NS}
name=quod-lab$ns

ensure_image() {
  if ! docker image inspect "$base" >/dev/null 2>&1; then
    docker build -q -t "$base" -f "$root/Dockerfile.dev" \
      --build-arg GO_VERSION="$(v go version)" --build-arg RUST_TOOLCHAIN="$(v rust toolchain)" "$root" >/dev/null
  fi
  if ! docker image inspect "$img" >/dev/null 2>&1; then
    docker build -q -t "$img" -f "$root/Dockerfile.runner" --build-arg BASE="$base" "$root" >/dev/null
  fi
}

case "${1:-}" in
  up)
    ensure_image
    docker rm -f "$name" >/dev/null 2>&1 || true
    docker run -d --name "$name" --privileged --device /dev/fuse \
      -v "$root":/src -v stillpass-target$ns:/src/target -v stillpass-work$ns:/work \
      -e CARGO_TARGET_DIR=/src/target -e TMPDIR=/work/tmp -e QUOD_HOME=/work/state \
      -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" -e QUOD_INTEGRATION=1 -w /src \
      "$img" sleep infinity >/dev/null
    docker exec "$name" mkdir -p /work/tmp /work/state
    # Per-step cgroups (quod_sandbox::cgroup_root): cgroup v2 lets a parent
    # hand memory and pids to children only if it holds no processes, so the
    # container's own move to a leaf first. Steps then count their own OOM
    # kills and fork-limit hits instead of the machine's.
    docker exec "$name" sh -c 'mkdir -p /sys/fs/cgroup/init &&
      for p in $(cat /sys/fs/cgroup/cgroup.procs); do echo $p > /sys/fs/cgroup/init/cgroup.procs 2>/dev/null || true; done &&
      echo "+memory +pids" > /sys/fs/cgroup/cgroup.subtree_control &&
      mkdir -p /sys/fs/cgroup/quod &&
      echo "+memory +pids" > /sys/fs/cgroup/quod/cgroup.subtree_control' ||
      echo "note: no per-step cgroups; OOM kills are counted machine-wide" >&2
    echo "$name up"
    ;;
  exec)
    shift
    tty=; [ -t 0 ] && tty=-it
    exec docker exec $tty "$name" bash -c "$*"
    ;;
  down)
    docker rm -f "$name" >/dev/null 2>&1 && echo "$name removed" || echo "$name not running"
    ;;
  *)
    echo "usage: $0 up | exec <cmd> | down" >&2
    exit 2
    ;;
esac
