#!/usr/bin/env bash
# The gates, under the shared lock: merge gate whole, CLI acceptance on
# cli/cli, hooks acceptance gate (b) on cli/cli. Logs in /work/tmp/gidx-*.log.
cd /src
flock /work/tmp/heavy.lock bash -c '
  echo Y > /sys/module/fuse/parameters/enable_uring 2>/dev/null
  echo "# merge gate: load $(cat /proc/loadavg)"
  scripts/merge-gate.sh > /work/tmp/gidx-merge-gate.log 2>&1; echo "merge gate exit $?" | tee -a /work/tmp/gidx-merge-gate.log
  scripts/acceptance.sh go > /work/tmp/gidx-acceptance-go.log 2>&1; echo "acceptance exit $?" | tee -a /work/tmp/gidx-acceptance-go.log
  QUOD_INTEGRATION=1 QUOD_SPIKE_REPO=/work/cli-quod QUOD_A7B_SAMPLES=10 \
    cargo test -q --release -p quod-cli --test hooks_acceptance hooks_acceptance -- --exact --nocapture \
    > /work/tmp/gidx-hooks-acceptance.log 2>&1; echo "hooks acceptance exit $?" | tee -a /work/tmp/gidx-hooks-acceptance.log
'
