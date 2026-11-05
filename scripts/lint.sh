#!/usr/bin/env bash
# fmt, clippy -D warnings, unit tests. Fails loudly; run before every commit.
# Test output is kept in $QUOD_LINT_LOG (default target/lint-test.log) and its
# failures printed, so an intermittent failure names its test.
set -euo pipefail
cargo fmt --all --check
cargo clippy -q --workspace --all-targets -- -D warnings
cargo clippy -q --workspace --all-targets --all-features -- -D warnings
log=${QUOD_LINT_LOG:-target/lint-test.log}
mkdir -p "$(dirname "$log")"
# A test that stalls names itself: past 18 minutes the process tree (the
# test binary and what it started) goes to the gate's log every 5 minutes.
( sleep 1080; while :; do
    echo "lint watchdog: tests still running after $(( $(ps -o etimes= -p $$) / 60 )) min; the test processes:"
    ps -eo pid,ppid,etimes,stat,wchan:20,args --forest | grep -vE ' \[|ps -eo|grep ' | grep -A40 -E 'cargo test|/deps/' | cut -c1-220
    tail -5 "$log"
    sleep 300
  done ) &
watchdog=$!
trap 'kill $watchdog 2>/dev/null' EXIT
if ! cargo test -q --workspace >"$log" 2>&1; then
  grep -E "^---- |panicked at|^test .* FAILED|^error: test failed" "$log" || tail -40 "$log"
  echo "lint FAILED: unit tests (full output in $log)" >&2
  exit 1
fi
echo lint ok
