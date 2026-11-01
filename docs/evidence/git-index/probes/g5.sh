#!/usr/bin/env bash
# Second round: baseline again (fixed CLI probe, G5 added) and the hooks test after.
flock /work/tmp/heavy.lock bash -c '
  bash /src/.probe/baseline.sh > /work/tmp/gidx-before2.log 2>&1
  cd /src && QUOD_INTEGRATION=1 QUOD_SPIKE_REPO=/work/cli-quod \
    cargo test -q --release -p quod-cli --test hooks_acceptance hooks_git_index -- --exact --nocapture 2>&1 \
    | grep -E "^G[0-9]|^  .*(differed|ran again|names)|hooks git|test result|panicked" > /work/tmp/gidx-after2.log
'
echo "== before"; cat /work/tmp/gidx-before2.log
echo "== after"; cat /work/tmp/gidx-after2.log
