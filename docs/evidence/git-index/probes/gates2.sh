#!/usr/bin/env bash
# Review-fix gates under the shared lock, full logs in /work/tmp/gidx2-*.log:
# lint's steps with the test output kept, scripts/merge-gate.sh whole
# (traced), scripts/acceptance.sh go on cli/cli.
cd /src
flock /work/tmp/heavy.lock bash -c '
  echo Y > /sys/module/fuse/parameters/enable_uring 2>/dev/null
  {
    echo "# load $(cat /proc/loadavg)"
    set -o pipefail
    cargo fmt --all --check &&
    cargo clippy -q --workspace --all-targets -- -D warnings &&
    cargo clippy -q --workspace --all-targets --all-features -- -D warnings &&
    cargo test --workspace 2>&1 && echo "lint steps ok"
  } > /work/tmp/gidx2-lint-full.log 2>&1; echo "lint steps exit $?" | tee -a /work/tmp/gidx2-lint-full.log
  bash -x scripts/merge-gate.sh > /work/tmp/gidx2-merge-gate.log 2>&1; echo "merge gate exit $?" | tee -a /work/tmp/gidx2-merge-gate.log
  scripts/acceptance.sh go > /work/tmp/gidx2-acceptance-go.log 2>&1; echo "acceptance exit $?" | tee -a /work/tmp/gidx2-acceptance-go.log
'
