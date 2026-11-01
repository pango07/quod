#!/usr/bin/env bash
# The gate a branch passes on main before it is pushed: lint (fmt, clippy,
# every test CI runs), plus the io_uring suites, which need FUSE over
# io_uring. Run it inside the dev container, or through scripts/ci-gate.sh.
#   QUOD_GATE_SKIP_LINT=1  skip lint.sh (CI runs it as its own job)
#   QUOD_RING_OPTIONAL=1   on a host without the ring, skip the ring suites
#                          with a message instead of failing (GitHub's
#                          runners, when their kernel has no ring)
set -euo pipefail
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

if [ "${QUOD_GATE_SKIP_LINT:-}" = 1 ]; then
  echo "merge gate: lint.sh skipped (QUOD_GATE_SKIP_LINT=1; it runs as its own gate)"
else
  scripts/lint.sh
fi

ring=/sys/module/fuse/parameters/enable_uring
if [ "$(cat "$ring" 2>/dev/null)" != "Y" ]; then
  if [ "${QUOD_RING_OPTIONAL:-}" != 1 ]; then
    echo "merge gate: FUSE io_uring unavailable ($ring); enable it with 'echo Y > $ring' and re-run" >&2
    exit 1
  fi
  echo "merge gate: SKIPPED the ring suites (uring, ring_faults, the ring cases of faults and observer_faults, transports): FUSE io_uring unavailable ($ring: $(cat "$ring" 2>/dev/null || echo absent)), QUOD_RING_OPTIONAL=1"
  cargo test -q -p quod-fs --features fault-injection --test faults -- --test-threads=1
  cargo test -q -p quod-cli --features fault-injection --test observer_faults -- --test-threads=1
  echo "merge gate ok (classic transport only; ring suites skipped)"
  exit 0
fi
cargo test -q -p quod-fs --features fault-injection --test uring -- --ignored --test-threads=1
cargo test -q -p quod-cli --features fault-injection --test ring_faults -- --ignored --test-threads=1
# The observer's failure path (QUOD_FAULT): a panic while serving
# aborts the step and fails the run, classic and ring (--include-ignored).
cargo test -q -p quod-fs --features fault-injection --test faults -- --include-ignored --test-threads=1
cargo test -q -p quod-cli --features fault-injection --test observer_faults -- --include-ignored --test-threads=1
# transports_log_identically only: per_request_cost is a benchmark that needs QUOD_SPIKE_REPO
cargo test -q -p quod-cli --release --test transports transports_log_identically -- --ignored --test-threads=1
echo "merge gate ok"
