#!/usr/bin/env bash
# Mechanism, outside Quod: which git commands go's VCS stamping runs on
# cli/cli, and how many worktree files `git status --porcelain` opens with
# a GitHub-like index versus the index `git read-tree` leaves (zero stat
# data, what the hooks' checkout served until now).
set -eu
W=/work/tmp/gi-mech
rm -rf $W; mkdir -p $W
git clone -q --no-local /work/cli $W/cli
cd $W/cli
git checkout -q --detach 9490d204a2ba
echo "== git $(git --version | cut -d' ' -f3), go $(go env GOVERSION)"
echo "== tracked files: $(git ls-files | wc -l)"
echo "== go build -x ./cmd/gh: the git commands go runs (cwd elided)"
GOFLAGS=-mod=mod go build -x -o $W/gh ./cmd/gh 2>&1 | grep -E '^git |&& git |git -c' | sed 's/^cd [^;]*; //' | sort | uniq -c || true
echo "== go test -x -run XXX ./pkg/cmdutil: git commands (go test does not stamp)"
go test -x -count=1 -run XXX ./pkg/cmdutil 2>&1 | grep -E '^git |&& git |git -c' | sort | uniq -c || echo "   none"
command -v strace >/dev/null || { echo "no strace"; exit 0; }
count() { strace -f -e trace=openat -o $W/trace git status --porcelain >/dev/null 2>&1; grep -v -E '"/(etc|usr|lib|proc|dev)|\.git/' $W/trace | grep -c -E 'openat\(.*= [0-9]+$' || true; }
echo "== git status --porcelain, index from checkout (stat data matches): files opened: $(count)"
git read-tree HEAD
echo "== after git read-tree HEAD (stat data zero, as the hooks served): files opened: $(GIT_OPTIONAL_LOCKS=0 count)"
echo "   (GIT_OPTIONAL_LOCKS=0, as the hooks set it: git refreshes in memory only)"
git status --porcelain | head -3; echo "   porcelain lines: $(git status --porcelain | wc -l)"
