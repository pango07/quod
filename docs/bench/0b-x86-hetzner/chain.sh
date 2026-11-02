#!/usr/bin/env bash
# Runs after scripts/acceptance.sh go: everything measured, one at a time.
set -u
. /etc/profile.d/quod.sh
cd /opt/quod
L=/work/logs
until grep -q ACC_EXIT $L/acceptance-go.log; do sleep 5; done
stamp() { echo "== $(date -u +%FT%TZ) $* | $(cat /proc/loadavg)"; }
grep -q HOOKS_EXIT=0 $L/hooks-acceptance.log 2>/dev/null || {
stamp hooks acceptance start
QUOD_INTEGRATION=1 timeout 1200 cargo test -q --release -p quod-cli --test hooks_acceptance -- --nocapture --test-threads=1
echo HOOKS_EXIT=$?
stamp hooks acceptance end
} > $L/hooks-acceptance.log 2>&1
{
stamp history start N=${N:-30} COLD_EVERY=${COLD_EVERY:-5}
N=${N:-30} COLD_EVERY=${COLD_EVERY:-5} OUT=/work/hist /usr/local/python/bin/python3 -I /work/history.py
echo HIST_EXIT=$?
stamp history end
} > $L/history.log 2>&1
{
stamp overhead bench start
rm -rf /work/bench; mkdir -p /work/bench
git clone -q /work/cli-src /work/bench/cli && git -C /work/bench/cli checkout -q -B bench 9490d204a2ba95434153dedce99898ce9f281426
echo '.quod/' >> /work/bench/cli/.git/info/exclude
timeout 5400 target/release/quod --state /work/bench/state bench --repo /work/bench/cli --runs 10 --no-hit-rate --interleave --out /work/bench/report.md
echo BENCH_EXIT=$?
stamp overhead bench end
} > $L/overhead-bench.log 2>&1
{
stamp replay start
ROUNDS=${ROUNDS:-5} OUT=/work/rs /usr/local/python/bin/python3 -I /work/replay.py
echo REPLAY_EXIT=$?
stamp replay end
} > $L/replay.log 2>&1
echo CHAIN_DONE > $L/chain.done
