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
if ! cargo test -q --workspace >"$log" 2>&1; then
  grep -E "^---- |panicked at|^test .* FAILED|^error: test failed" "$log" || tail -40 "$log"
  echo "lint FAILED: unit tests (full output in $log)" >&2
  exit 1
fi
echo lint ok
