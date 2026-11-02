#!/usr/bin/env bash
# 0e on a dedicated box (nothing else runs; no shared lock): build quod at
# main, then (1) the 40-commit history, plain-warm and quod interleaved per
# commit (0c's driver), (2) realistic traffic, plain-warm and quod
# interleaved per job (trust-v2's driver, MODE=both), (3) the ledgers'
# audit, trust and suspect tables, (4) every replay re-run plainly.
#   /work/0e/chain.sh   (logs in /work/0e/logs)
set -u
. /etc/profile.d/quod.sh
R=/work/0e
L=$R/logs
PY=/usr/local/python/bin/python3
Q=/opt/quod/target/release/quod
mkdir -p $L
stamp() { echo "== $(date -u +%FT%TZ) $* | $(cat /proc/loadavg)"; }
{
  stamp build start
  cd /opt/quod && timeout 2400 cargo build --release -q -p quod-cli; echo "BUILD_EXIT=$? $(cat /opt/quod/COMMIT)"
  $Q --version
  stamp build end
} > $L/build.log 2>&1
grep -q BUILD_EXIT=0 $L/build.log || exit 1
if ! grep -q HIST_EXIT=0 $L/history.log 2>/dev/null; then
  rm -rf $R/hist
  {
    stamp history start
    N=40 COLD_EVERY=0 OUT=$R/hist QUOD=$Q timeout 14400 $PY -I $R/history.py; echo "HIST_EXIT=$?"
    stamp history end
  } > $L/history.log 2>&1
fi
if ! grep -q TRAFFIC_EXIT=0 $L/traffic.log 2>/dev/null; then
  rm -rf $R/traffic
  {
    stamp traffic start
    MODE=both N=40 OUT=$R/traffic QUOD=$Q timeout 21600 $PY -I $R/traffic.py; echo "TRAFFIC_EXIT=$?"
    stamp traffic end
  } > $L/traffic.log 2>&1
fi
for s in hist/q/state traffic/state; do
  db=$R/$s/ledger.sqlite
  echo "### $db"
  sqlite3 -header -column $db "SELECT outcome, count(*) n, sum(suspect IS NOT NULL) suspect FROM runs GROUP BY outcome;"
  sqlite3 -header -column $db "SELECT substr(step_key,1,14) key, cmd, state, eligible, replays, audits, divergences, quarantined_at FROM trust;"
  sqlite3 -header -column $db "SELECT substr(step_key,1,14) key, substr(manifest_id,1,14) manifest, at, phase, diverged, detail FROM audits ORDER BY id;"
  sqlite3 -header -column $db "SELECT id, outcome, suspect FROM runs WHERE suspect IS NOT NULL;"
done > $L/ledgers.txt 2>&1
{
  stamp spotcheck start
  rm -rf $R/spot
  $PY -I $R/spotcheck.py $R/spot $R/hist/rows.jsonl $R/traffic/rows.jsonl; echo "SPOT_EXIT=$?"
  stamp spotcheck end
} > $L/spotcheck.log 2>&1
stamp chain done > $L/chain.done
