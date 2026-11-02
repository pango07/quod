#!/usr/bin/env bash
# bench-real-small: Quod against plain on three small real repositories
# (versions.toml [bench_real.*]; scripts/bench-real/real.py says how), and
# the publishable report (scripts/bench-real/report.py). The slow, credible
# tier beside bench-lite: 15-60 minutes, network for the clones and the
# prefetch only.
#
#   scripts/bench-real.sh                        all three repos, N=20
#   scripts/bench-real.sh --repos go --n 5       a subset
#   scripts/bench-real.sh --cpus 0-3             pin everything to these CPUs (taskset)
#
# On the host (macOS) it re-runs itself in the dev container under the shared
# heavy lock. Output: $BENCH_REAL_OUT (default /work/bench-real): clones,
# state, logs, rows.jsonl; report.md and summary.json also go to
# $BENCH_REAL_RESULTS (default .bench-real/ in the checkout). Exit 1 on any
# wrong replay.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if [ "$(uname -s)" != Linux ]; then
  q=; [ $# -eq 0 ] || q=$(printf ' %q' "$@")
  exec "$root/scripts/dev.sh" "QUOD_COMMIT=$(git -C "$root" rev-parse HEAD) flock /work/tmp/heavy.lock scripts/bench-real.sh$q"
fi

repos=go,node,python n=20 cpus=
while [ $# -gt 0 ]; do
  case "$1" in
    --repos) repos=$2; shift 2;;
    --n) n=$2; shift 2;;
    --cpus) cpus=$2; shift 2;;
    *) echo "bench-real: unknown argument $1" >&2; exit 2;;
  esac
done

py=/usr/local/python/bin/python3
out=${BENCH_REAL_OUT:-}
if [ -z "$out" ]; then
  if [ -d /work ] && [ -w /work ]; then out=/work/bench-real; else out=$root/target/bench-real; fi
fi
quod=${QUOD_BIN:-}
if [ -z "$quod" ]; then
  cargo build --release -q -p quod-cli
  quod=${CARGO_TARGET_DIR:-$root/target}/release/quod
fi
res=${BENCH_REAL_RESULTS:-$root/.bench-real}
mkdir -p "$out" "$res"
pin=(); [ -z "$cpus" ] || pin=(taskset -c "$cpus")
start=$(date +%s)
rc=0
"${pin[@]}" "$py" -I "$root/scripts/bench-real/real.py" --quod "$quod" --out "$out" --repos "$repos" --n "$n" || rc=$?
"$py" -I "$root/scripts/bench-real/report.py" "$out/rows.jsonl" --json "$res/summary.json" > "$res/report.md" || rc=1
cp "$out/rows.jsonl" "$res/rows.jsonl"
echo "bench-real: exit $rc after $(( $(date +%s) - start ))s; report in $res/report.md"
exit "$rc"
