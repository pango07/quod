#!/usr/bin/env bash
# The trust v2 measurements on the Hetzner box, one after the other, each
# series under the box's shared heavy lock (other agents use it too, and may
# run between series). Fresh state dir per series. Logs in $ROOT/logs; rows
# in $ROOT/<series>/rows.jsonl.
#
#   ROOT=/work/tv2 QUOD=/opt/quod-v2/target/release/quod docs/bench/trust-v2/chain.sh
set -u
. /etc/profile.d/quod.sh
ROOT=${ROOT:-/work/tv2}
QUOD=${QUOD:-/opt/quod-v2/target/release/quod}
LOCK=${LOCK:-/work/heavy.lock}
HERE=$(cd "$(dirname "$0")" && pwd)
PY=/usr/local/python/bin/python3
L=$ROOT/logs
mkdir -p "$L"
stamp() { echo "== $(date -u +%FT%TZ) $* | $(cat /proc/loadavg)"; }
series() {  # name mode policy
  local name=$1 mode=$2 policy=$3
  if grep -q "EXIT=0" "$L/$name.log" 2>/dev/null; then echo "$name done already"; return; fi
  rm -rf "$ROOT/$name"; mkdir -p "$ROOT/$name"
  {
    stamp "$name waiting for $LOCK"
    flock "$LOCK" env MODE=$mode POLICY="$policy" OUT=$ROOT/$name QUOD=$QUOD N=${N:-40} \
      bash -c 'echo "== $(date -u +%FT%TZ) start (mode $MODE, policy '"'"'$POLICY'"'"') | $(cat /proc/loadavg)"; timeout 14400 '"$PY"' -I '"$HERE"'/traffic.py; echo "EXIT=$?"'
    stamp "$name end"
  } > "$L/$name.log" 2>&1
}
# 1. The 40-commit history under v2 defaults (0b measured v1: 2 replays of 120).
series history-v2 history ""
# 2. Realistic traffic (PR head, merge, 10% re-runs) under v2 defaults.
series traffic-v2 traffic ""
# 3. The same traffic under v1's policy, for the before/after.
series traffic-v1 traffic "--proofs 2 --shadow 0"
# 4. 0b's plain-warm baseline on this box, for like-with-like history numbers.
series plain-warm plain ""
"$PY" -I "$HERE/summary.py" "history v2=$ROOT/history-v2/rows.jsonl" "traffic v2=$ROOT/traffic-v2/rows.jsonl" "traffic v1=$ROOT/traffic-v1/rows.jsonl" "plain warm=$ROOT/plain-warm/rows.jsonl" > "$ROOT/summary.md" 2>&1
echo CHAIN_DONE > "$L/chain.done"
