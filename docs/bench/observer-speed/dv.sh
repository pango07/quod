#!/usr/bin/env bash
# scripts/dev.sh's container with a private target volume, the shared work
# volume (corpus, heavy lock) and this directory at /s. DOCKER_EXTRA adds
# docker flags: the measurements ran with --cpuset-cpus 0-3 (the Hetzner
# box's 4 CPUs), the perf sidecar with --pid=host --cpuset-cpus 4-5.
set -eu
root=$(cd "$(dirname "$0")/../../.." && pwd)
s=$(cd "$(dirname "$0")" && pwd)
img=quod-dev:go1.27.3-rust1.98.0-node24.22.2-py3.12.16-uv0.13.6
exec docker run --rm ${DOCKER_EXTRA:-} --privileged --device /dev/fuse \
  -v "$root":/src -v stillpass-target-obsspeed:/src/target -v stillpass-work:/work -v "$s":/s \
  -e CARGO_TARGET_DIR=/src/target -e TMPDIR=/work/tmp -e QUOD_HOME=/work/state -e QUOD_INTEGRATION="${QUOD_INTEGRATION:-1}" \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" \
  "$img" bash -c "mkdir -p /work/tmp /work/obsspeed && $*"
