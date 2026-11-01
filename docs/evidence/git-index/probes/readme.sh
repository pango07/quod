#!/usr/bin/env bash
# Which cli/cli test package reads README.md once a .git is served:
# record `go test <pkg>`, commit a README.md edit, look at the miss.
cd /src
sp=/src/target/release/quod
cargo build -q --release -p quod-cli
st=$(mktemp -d)
cp -a /work/cli-quod "$st/repo"
cd "$st/repo"
git checkout -q -B quod-readme
for pkg in ./internal/skills/... ./pkg/cmd/skills/...; do
  for i in 1 2; do
    "$sp" --state "$st/state" run --repo . --toolchain go --host-root --mode replay --proofs 1 --step "go test $pkg" 2>&1 | grep -E '^QUOD  (EXECUTED|REPLAYED)'
  done
done
echo "readme $RANDOM" >> README.md
git -c user.email=t@t -c user.name=t commit -qam "README only"
for pkg in ./internal/skills/... ./pkg/cmd/skills/...; do
  echo "== $pkg after a README.md commit"
  "$sp" --state "$st/state" run --repo . --toolchain go --host-root --mode replay --proofs 1 --step "go test $pkg" 2>&1 | grep -E '^QUOD  (EXECUTED|REPLAYED)|differed'
done
rm -rf "$st"
