#!/usr/bin/env bash
# bench-lite: Quod against plain on a generated fixture (scripts/bench-lite/,
# docs/bench/lite-baseline.md), the fast benchmark every fix branch runs.
#
#   scripts/bench-lite.sh                   build quod (release), run, print the table
#   scripts/bench-lite.sh --against main    and the deltas against docs/bench/lite-baseline.json
#   scripts/bench-lite.sh --against FILE    against another run's summary JSON
#   scripts/bench-lite.sh --summarize ROWS  only re-summarize a rows.jsonl
#
# On the host (macOS) it re-runs itself in the dev container under the shared
# heavy lock: scripts/dev.sh 'flock /work/tmp/heavy.lock scripts/bench-lite.sh ...'.
# In the container or on a Linux CI runner it runs directly (as root: Quod's
# sandbox and FUSE need it, as for every gate). Output: $BENCH_LITE_OUT
# (default /work/bench-lite, or target/bench-lite without /work): rows.jsonl and
# logs; summary.json, table.md and rows.jsonl also go to $BENCH_LITE_RESULTS
# (default .bench-lite/ in the checkout). Exit 1 on any wrong replay.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if [ "$(uname -s)" != Linux ]; then
  q=; [ $# -eq 0 ] || q=$(printf ' %q' "$@")
  exec "$root/scripts/dev.sh" "flock /work/tmp/heavy.lock scripts/bench-lite.sh$q"
fi

against= summarize=
while [ $# -gt 0 ]; do
  case "$1" in
    --against) against=$2; shift 2;;
    --summarize) summarize=$2; shift 2;;
    *) echo "bench-lite: unknown argument $1" >&2; exit 2;;
  esac
done
case "$against" in
  '') ;;
  main) against=$root/docs/bench/lite-baseline.json;;
esac
if [ -n "$against" ] && [ ! -f "$against" ]; then
  echo "bench-lite: no baseline at $against; no deltas" >&2
  against=
fi

py=/usr/local/python/bin/python3
[ -x "$py" ] || py=python3
args=()
[ -z "$against" ] || args+=(--against "$against")

if [ -n "$summarize" ]; then
  exec "$py" -I "$root/scripts/bench-lite/driver.py" --summarize "$summarize" "${args[@]}"
fi

out=${BENCH_LITE_OUT:-}
if [ -z "$out" ]; then
  if [ -d /work ] && [ -w /work ]; then out=/work/bench-lite; else out=$root/target/bench-lite; fi
fi
quod=${QUOD_BIN:-}
if [ -z "$quod" ]; then
  cargo build --release -q -p quod-cli
  quod=${CARGO_TARGET_DIR:-$root/target}/release/quod
fi
mkdir -p "$(dirname "$out")"
res=${BENCH_LITE_RESULTS:-$root/.bench-lite}
mkdir -p "$res"
start=$(date +%s)
rc=0
"$py" -I "$root/scripts/bench-lite/driver.py" --quod "$quod" --out "$out" \
  --json "$res/summary.json" --md "$res/table.md" "${args[@]}" || rc=$?
cp "$out/rows.jsonl" "$res/rows.jsonl" 2>/dev/null || true
echo "bench-lite: exit $rc after $(( $(date +%s) - start ))s; summary in $res"
exit "$rc"
