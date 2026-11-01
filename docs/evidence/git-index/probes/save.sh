#!/usr/bin/env bash
# Gate logs into the evidence directory, host paths filtered, gzipped.
cd /work/tmp
out=/src/docs/evidence/git-index
for f in merge-gate acceptance-go hooks-acceptance lint; do
  sed -e 's#/Users/[^ ]*/agent-a592cef55b8f60b48#<repo>#g' -e 's#/Users/[a-z]*#<home>#g' "gidx-$f.log" | gzip -9 > "$out/$f.log.gz"
done
zgrep -c -E '/Users/|juliovaught' "$out"/*.gz
ls -la "$out"
