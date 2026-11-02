#!/bin/bash
# exp/never-worse on the x86 box: build, then the 40-commit history
# (plain-warm vs quod, the 0b driver), all under the box's heavy lock.
set -u
. /etc/profile.d/quod.sh
L=/work/nw/logs
mkdir -p $L
exec >> $L/launch.log 2>&1
echo "== queued $(date -u +%FT%TZ) load $(cat /proc/loadavg)"
flock /work/heavy.lock bash -c '
  echo "== locked $(date -u +%FT%TZ) load $(cat /proc/loadavg)"
  cd /opt/quod-nw
  [ -d target/release ] || cp -a /opt/quod/target /opt/quod-nw/target
  timeout 1800 cargo build --release -q -p quod-cli || { echo BUILD_FAILED; exit 1; }
  echo "BUILD_OK $(cat /opt/quod-nw/COMMIT)"
  rm -rf /work/nw/hist
  N=40 COLD_EVERY=0 OUT=/work/nw/hist timeout 10800 /usr/local/python/bin/python3 -I /work/nw/history.py
  echo "HIST_EXIT=$? $(date -u +%FT%TZ)"
'
echo "== done $(date -u +%FT%TZ)"
