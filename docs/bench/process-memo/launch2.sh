#!/bin/bash
# Queued behind launch.sh on the shared heavy lock: the prototype on the
# 40-commit history paired with plain-warm, then the unit-level mutation tests.
cd /work/pm
exec >> /work/pm/launch2.log 2>&1
echo "== queued $(date -u +%FT%TZ)"
flock /work/heavy.lock bash -c 'echo "== start $(date -u +%FT%TZ) load $(cat /proc/loadavg)"; rm -rf /work/pm/memo; timeout 6000 /usr/local/python/bin/python3 -I /work/pm/pm_memo_history.py; echo "MEMO_EXIT=$? $(date -u +%FT%TZ)"; timeout 3000 /usr/local/python/bin/python3 -I /work/pm/pm_mutate.py; echo "MUTATE_EXIT=$? $(date -u +%FT%TZ)"'
echo LAUNCH2_DONE
