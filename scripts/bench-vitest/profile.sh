#!/usr/bin/env bash
# The recorder's cost, part by part: full runs at one commit, plain and
# `quod vitest --full` with parts of the recorder switched off
# (QUOD_VITEST_DISABLE), alternating, twice each. Then the observer
# backstop once.
#   profile.sh <project checkout> <vitest args...>   (QUOD=...)
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
proj=$(cd "$1" && pwd); shift
args=("$@")
out=${RUNNER_TEMP:-/tmp}/bench-vitest
state=/work/state/quod-vitest-profile
rm -rf "$out" "$state"; mkdir -p "$out/logs"
cd "$proj"
ms() { date +%s%3N; }
run() { # label, then the command
  local label=$1; shift
  local s; s=$(ms)
  "$@" >"$out/logs/$label.log" 2>&1
  local rc=$?
  local d=$(( $(ms) - s ))
  local extra; extra=$(grep -h 'workerSetupSum' "$state/out.json" 2>/dev/null | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{try{const t=JSON.parse(s).timings;console.log(`init ${t.init|0} ms, worker setup sum ${t.workerSetupSum|0} ms, dump sum ${t.workerDumpSum|0} ms`)}catch{console.log("")}})')
  echo "{\"label\":\"$label\",\"ms\":$d,\"exit\":$rc}" >> "$out/results.jsonl"
  printf '%-22s %7d ms  exit %d  %s\n' "$label" "$d" "$rc" "$extra"
  rm -f "$state/out.json"
}
q() { QUOD_VITEST_DISABLE=$1 "$QUOD" vitest --dir "$state" --full -- "${args[@]}"; }
for round in 1 2; do
  run "plain-$round" node_modules/.bin/vitest "${args[@]}"
  run "quod-all-$round" q ""
  run "plain-b-$round" node_modules/.bin/vitest "${args[@]}"
  run "quod-no-cov-$round" q cov
  run "quod-no-escapes-$round" q escapes
  run "quod-no-loader-$round" q loader
  run "quod-no-fs-env-$round" q fs,env
  run "quod-no-main-$round" q main
  run "quod-none-$round" q fs,env,loader,escapes,main
done
echo "::group::observer backstop"
"$QUOD" vitest --dir "$state" --full -- "${args[@]}" >/dev/null 2>&1
bash "$here/observe.sh" "$proj" "$state" "${args[@]}" 2>&1 | tail -70
echo "::endgroup::"
