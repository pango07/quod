#!/usr/bin/env bash
# Before (main) and after (this branch) on cli/cli, under the shared lock.
flock /work/tmp/heavy.lock bash -c '
  bash /src/.probe/baseline.sh > /work/tmp/gidx-before.log 2>&1
  bash /src/.probe/after.sh > /work/tmp/gidx-after.log 2>&1
'
echo "== before"; cat /work/tmp/gidx-before.log
echo "== after"; cat /work/tmp/gidx-after.log
