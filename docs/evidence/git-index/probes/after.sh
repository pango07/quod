#!/usr/bin/env bash
# After: this branch, on cli/cli 9490d204. The CLI test (served vs plain
# checkout, replays) and the hooks test (G0-G4), both asserting.
set -u
cd /src
spike=/work/cli-quod
# The plain checkout's go build reads the same module cache, offline.
export GOMODCACHE=$spike/.quod/gomod GOPROXY=off
echo "== exp/git-index, CLI: tests/git_index.rs on the tiny module and cli/cli"
QUOD_INTEGRATION=1 QUOD_SPIKE_REPO=$spike \
  cargo test -q --release -p quod-cli --test git_index -- --nocapture 2>&1 \
  | grep -E 'identical|re-checked|REPLAYED$|EXECUTED \(|commit|test result|panicked|differed'
echo "== exp/git-index, hooks: hooks_git_index"
QUOD_INTEGRATION=1 QUOD_SPIKE_REPO=$spike \
  cargo test -q --release -p quod-cli --test hooks_acceptance hooks_git_index -- --exact --nocapture 2>&1 \
  | grep -E '^G[0-9]|^  .*(differed|ran again|names)|hooks git|test result|panicked'
