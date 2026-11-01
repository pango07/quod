#!/usr/bin/env bash
# Before: main 2a24fd8 (an archive under .probe/main, with this branch's
# hooks test added in measure mode), on cli/cli 9490d204.
set -u
cd /src/.probe/main
export CARGO_TARGET_DIR=/src/target/gidx-main
spike=/work/cli-quod
echo "== main 2a24fd8, CLI: what quod run serves a go build on cli/cli"
cargo build -q --release -p quod-cli 2>&1 | tail -3
sp=$CARGO_TARGET_DIR/release/quod
st=$(mktemp -d)
cp -a "$spike" "$st/repo"
"$sp" --state "$st/state" run --repo "$st/repo" --toolchain go --host-root --step \
  'git status --porcelain 2>&1 | head -3; go build -o .quod/out/gh ./cmd/gh && echo "go version -m vcs lines: $(go version -m .quod/out/gh | grep -c vcs)"' 2>&1 \
  | grep -v '^  trust' | head -12
rm -rf "$st"
echo "== main 2a24fd8, hooks: hooks_git_index in measure mode (QUOD_GIT_INDEX_MEASURE=1)"
QUOD_INTEGRATION=1 QUOD_SPIKE_REPO=$spike QUOD_GIT_INDEX_MEASURE=1 \
  cargo test -q --release -p quod-cli --test hooks_acceptance hooks_git_index -- --exact --nocapture 2>&1 \
  | grep -E '^G[0-9]|^  .*(differed|ran again|names)|test result|panicked'
