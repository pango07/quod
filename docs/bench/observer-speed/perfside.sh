#!/bin/bash
# Sidecar: install perf here (not in the measured container: it would change Quod's host key),
# record system-wide on cpus 0-3 whenever /work/obsspeed/prof/ctl exists. Exits after N profiles.
apt-get update -qq >/dev/null 2>&1; apt-get install -y -qq linux-perf >/dev/null 2>&1
C=/work/obsspeed/prof/ctl
rm -f $C $C.on $C.done
n=${1:-2}
while [ $n -gt 0 ]; do
  until [ -e $C ]; do sleep 0.05; done
  name=$(cat $C)
  perf record -q -e cpu-clock -F 997 -a -C 0-3 -g -o /work/obsspeed/prof/$name.data >/dev/null 2>&1 &
  pid=$!
  sleep 0.3; touch $C.on
  while [ -e $C ]; do sleep 0.05; done
  kill -INT $pid; wait $pid
  echo "== $name"
  perf report -i /work/obsspeed/prof/$name.data --no-children --sort comm --stdio -g none 2>/dev/null | grep -v '^#' | grep -v '^$' | head -20
  perf report -i /work/obsspeed/prof/$name.data --no-children --sort sym --stdio -g none 2>/dev/null | grep -v '^#' | grep -v '^$' | head -40
  rm -f $C.on
  n=$((n-1))
done
