#!/usr/bin/env bash
# Where a small vitest run's time goes, at one commit: plain vitest on 1, 4
# and 10 test files (the critical path of a typical commit runs that many),
# with a timing reporter (phases in ms since process start, per-module
# diagnostics), variants (pool, workers, vitest's fs module cache), and CPU
# profiles of the main process and the workers.
#   breakdown.sh <project checkout> <vitest args...>
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
proj=$(cd "$1" && pwd); shift
args=("$@")
out=${RUNNER_TEMP:-/tmp}/bench-vitest
rm -rf "$out"; mkdir -p "$out/logs" "$out/prof"
cd "$proj"
ms() { local up _; read -r up _ </proc/uptime; echo $(( 10#${up/./} * 10 )); }
mapfile -t all < <(git ls-files '*.spec.ts' '*.test.ts' | grep -v -e e2e/ -e '\.test-d\.' | sort)
echo "${#all[@]} test files"
one=${BREAKDOWN_ONE:-${all[$(( ${#all[@]} / 2 ))]}}
four=("${all[@]:3:4}")
ten=(); for ((k = 0; k < ${#all[@]} && ${#ten[@]} < 10; k += ${#all[@]} / 10)); do ten+=("${all[$k]}"); done
rep=${BREAKDOWN_REPS:-3}
run() { # label, then vitest arguments
  local label=$1; shift
  for r in $(seq 1 "$rep"); do
    local s; s=$(ms)
    node_modules/.bin/vitest "${args[@]}" --reporter=default --reporter="$here/timing-reporter.mjs" "$@" >"$out/logs/$label-$r.log" 2>&1
    local rc=$? d=$(( $(ms) - s ))
    echo "{\"label\":\"$label\",\"rep\":$r,\"ms\":$d,\"exit\":$rc}" >> "$out/results.jsonl"
    printf '%-26s rep %d %6d ms exit %d\n' "$label" "$r" "$d" "$rc"
  done
  sed 's/\x1b\[[0-9;]*m//g' "$out/logs/$label-$rep.log" | grep -E '^(T |   import|   Duration| Test Files)' | sed 's/^/    /'
}
s=$(ms); node -e '' ; echo "node -e '': $(( $(ms) - s )) ms"
s=$(ms); node -e 'await import("vitest/node")' --input-type=module 2>/dev/null; echo "import vitest/node: $(( $(ms) - s )) ms"
echo "one: $one"; echo "four: ${four[*]}"; echo "ten: ${ten[*]}"
if [ "${BREAKDOWN_SET:-}" = cc ]; then
  # node's compile cache (V8 code for node-loaded modules, main and workers), workers = CPUs
  run one "$one"
  rm -rf /work/state/cc; NODE_COMPILE_CACHE=/work/state/cc rep=1 run one-cc-cold "$one"
  echo "compile cache: $(du -sh /work/state/cc | cut -f1), $(find /work/state/cc -type f | wc -l) files"
  NODE_COMPILE_CACHE=/work/state/cc run one-cc-warm "$one"
  run ten "${ten[@]}"
  NODE_COMPILE_CACHE=/work/state/cc run ten-cc-warm "${ten[@]}"
  run ten-w4 --maxWorkers=4 "${ten[@]}"
  NODE_COMPILE_CACHE=/work/state/cc run ten-cc-w4 --maxWorkers=4 "${ten[@]}"
  run full
  run full-w4 --maxWorkers=4
  NODE_COMPILE_CACHE=/work/state/cc run full-cc-w4 --maxWorkers=4
  echo "compile cache after: $(du -sh /work/state/cc | cut -f1)"
  exit 0
fi
if [ -n "${QUOD:-}" ]; then
  # The critical path itself: record everything once, then --no-record with
  # nothing changed (all replay) and with one test file touched (one runs).
  st=/work/state/breakdown-quod
  rm -rf "$st"
  "$QUOD" vitest --dir "$st" -- "${args[@]}" >"$out/logs/quod-record.log" 2>&1
  echo "recorded: $(grep -a 'quod vitest: ran' "$out/logs/quod-record.log")"
  crit() { # label
    for r in $(seq 1 "$rep"); do
      local s; s=$(ms)
      "$QUOD" vitest --no-record --dir "$st" -- "${args[@]}" --reporter=default --reporter="$here/timing-reporter.mjs" >"$out/logs/$1-$r.log" 2>&1
      local rc=$? d=$(( $(ms) - s ))
      echo "{\"label\":\"$1\",\"rep\":$r,\"ms\":$d,\"exit\":$rc}" >> "$out/results.jsonl"
      printf '%-26s rep %d %6d ms exit %d\n' "$1" "$r" "$d" "$rc"
    done
    sed 's/\x1b\[[0-9;]*m//g' "$out/logs/$1-$rep.log" | grep -E '^(T |   Duration| Test Files|quod vitest)' | grep -v -E '^T [0-9]+ (queued|collected|start) ' | sed 's/^/    /'
  }
  crit quod-allreplay
  echo "// quod breakdown" >> "$one"
  crit quod-one
  NODE_OPTIONS="--cpu-prof --cpu-prof-dir=$out/profq" "$QUOD" vitest --no-record --dir "$st" -- "${args[@]}" >"$out/logs/prof-quod-one.log" 2>&1
  echo "::group::cpu profiles, quod one"
  node "$here/cpuprof-summary.mjs" "$out/profq" 16
  echo "::endgroup::"
  git checkout -q -- "$one"
  [ "${BREAKDOWN_PLAIN:-0}" = 1 ] || exit 0
fi
run one "$one"
run one-threads --pool=threads "$one"
run four "${four[@]}"
run ten "${ten[@]}"
run ten-threads --pool=threads "${ten[@]}"
run ten-w2 --maxWorkers=2 "${ten[@]}"
run ten-w8 --maxWorkers=8 "${ten[@]}"
rm -rf node_modules/.experimental-vitest-cache
rep=1 run ten-fscache-cold --experimental.fsModuleCache "${ten[@]}"
run ten-fscache-warm --experimental.fsModuleCache "${ten[@]}"
run one-fscache-warm --experimental.fsModuleCache "$one"
rm -rf node_modules/.experimental-vitest-cache
run full
echo "::group::cpu profiles, one file (main process and the worker)"
NODE_OPTIONS="--cpu-prof --cpu-prof-dir=$out/prof" node_modules/.bin/vitest "${args[@]}" "$one" >"$out/logs/prof-one.log" 2>&1
node "$here/cpuprof-summary.mjs" "$out/prof" 16
echo "::endgroup::"
mkdir -p "$out/prof10"
echo "::group::cpu profiles, ten files"
NODE_OPTIONS="--cpu-prof --cpu-prof-dir=$out/prof10" node_modules/.bin/vitest "${args[@]}" "${ten[@]}" >"$out/logs/prof-ten.log" 2>&1
node "$here/cpuprof-summary.mjs" "$out/prof10" 12
echo "::endgroup::"
