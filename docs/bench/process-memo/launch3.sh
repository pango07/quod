#!/bin/bash
# Prototype v2 (hash cache for write-once inputs): memo-only history into a
# fresh store, then the unit-level mutation tests with their log fixed.
cd /work/pm
exec >> /work/pm/launch3.log 2>&1
echo "== queued $(date -u +%FT%TZ)"
flock /work/heavy.lock bash -c 'echo "== start $(date -u +%FT%TZ) load $(cat /proc/loadavg)"; rm -rf /work/pm/memo2; OUT=/work/pm/memo2 SERIES=memo timeout 5400 /usr/local/python/bin/python3 -I /work/pm/pm_memo_history.py; echo "MEMO_EXIT=$? $(date -u +%FT%TZ)"; OUT=/work/pm/memo2 timeout 3000 /usr/local/python/bin/python3 -I /work/pm/pm_mutate.py; echo "MUTATE_EXIT=$? $(date -u +%FT%TZ)"'
echo LAUNCH3_DONE
