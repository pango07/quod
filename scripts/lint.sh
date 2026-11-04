#!/usr/bin/env bash
# fmt, clippy -D warnings, unit tests. Fails loudly; run before every commit.
# Test output is kept in $QUOD_LINT_LOG (default target/lint-test.log) and its
# failures printed, so an intermittent failure names its test. Each phase
# prints its time ("lint: <phase> <s>s"), so a slow CI run says where it went.
set -euo pipefail
t=$(date +%s)
phase() { local now; now=$(date +%s); echo "lint: $1 $((now - t))s"; t=$now; }
cargo fmt --all --check
phase fmt
cargo clippy -q --workspace --all-targets -- -D warnings
phase clippy
cargo clippy -q --workspace --all-targets --all-features -- -D warnings
phase clippy-all-features
log=${QUOD_LINT_LOG:-target/lint-test.log}
mkdir -p "$(dirname "$log")"
# Built first, then run: the same work as one `cargo test`, timed apart.
if ! cargo test -q --workspace --no-run >"$log" 2>&1; then
  tail -40 "$log"
  echo "lint FAILED: building the tests (full output in $log)" >&2
  exit 1
fi
phase test-build
if ! cargo test -q --workspace >>"$log" 2>&1; then
  grep -E "^---- |panicked at|^test .* FAILED|^error: test failed" "$log" || tail -40 "$log"
  echo "lint FAILED: unit tests (full output in $log)" >&2
  exit 1
fi
phase test-run
echo lint ok
