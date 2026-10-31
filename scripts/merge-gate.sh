#!/usr/bin/env bash
# The gate a branch passes on main before it is pushed: lint (fmt, clippy,
# every test CI runs), plus the io_uring suites that CI cannot run because
# GitHub's runners have no FUSE ring. Run it inside the dev container.
set -euo pipefail
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

scripts/lint.sh

ring=/sys/module/fuse/parameters/enable_uring
if [ "$(cat "$ring" 2>/dev/null)" != "Y" ]; then
  echo "merge gate: FUSE io_uring unavailable ($ring); enable it with 'echo Y > $ring' and re-run" >&2
  exit 1
fi
cargo test -q -p sp-fs --features fault-injection --test uring -- --ignored --test-threads=1
cargo test -q -p sp-cli --features fault-injection --test ring_faults -- --ignored --test-threads=1
# transports_log_identically only: per_request_cost is a benchmark that needs STILLPASS_SPIKE_REPO
cargo test -q -p sp-cli --release --test transports transports_log_identically -- --ignored --test-threads=1
echo "merge gate ok"
