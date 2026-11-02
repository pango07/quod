#!/usr/bin/env bash
# Smoke test of the vitest driver alone (no Rust): a full recorded run, then
# a run that replays every recorded file, then the two JSON reports compared.
#   smoke.sh <project checkout> <vitest args...>
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
src=$(cd "$here/../../crates/quod-cli/src/vitest" && pwd)
proj=$1; shift
state=${RUNNER_TEMP:-/tmp}/quod-vitest-smoke
rm -rf "$state"; mkdir -p "$state"
cp "$src"/*.mjs "$state/"
cd "$proj"
echo '{"stateDir":"'"$state"'","replay":{}}' > "$state/plan0.json"
t() { local s=$(date +%s.%N); "$@"; local rc=$?; echo "WALL $(echo "$(date +%s.%N) - $s" | bc) rc=$rc" >&2; return 0; }

echo "::group::plain"
t node_modules/.bin/vitest run "$@" --reporter=dot --reporter=json --outputFile.json="$state/plain.json" 2>&1 | tail -15
echo "::endgroup::"
echo "::group::record (driver, empty plan)"
t node "$state/driver.mjs" "$state/plan0.json" "$state/out1.json" run "$@" --reporter=dot --reporter=json --outputFile.json="$state/j1.json" 2>&1 | tail -25
echo "::endgroup::"
node "$here/smoke-plan.mjs" "$state/out1.json" "$state" "$state/plan2.json"
echo "::group::replay all"
t node "$state/driver.mjs" "$state/plan2.json" "$state/out2.json" run "$@" --reporter=default --reporter=json --outputFile.json="$state/j2.json" 2>&1 | tail -40
echo "::endgroup::"
node -e 'const o=require(process.argv[1]);console.log(JSON.stringify({ran:o.ran.length,replayed:o.replayed.length,auditMismatch:o.auditMismatch,exit:o.exitCode,timings:o.timings}))' "$state/out2.json"
node "$here/compare.mjs" "$state/plain.json" "$state/j1.json" plain record || true
node "$here/compare.mjs" "$state/plain.json" "$state/j2.json" plain replay || true
