#!/usr/bin/env bash
# Review-fix gate logs into the evidence directory, host paths filtered.
cd /work/tmp
out=/src/docs/evidence/git-index
for f in lint-full merge-gate acceptance-go; do
  sed -e 's#/Users/[^ ]*/agent-a592cef55b8f60b48#<repo>#g' -e 's#/Users/[a-z]*#<home>#g' "gidx2-$f.log" | gzip -9 > "$out/review-$f.log.gz"
done
zgrep -c -E '/Users/|juliovaught' "$out"/review-*.gz
grep -E 'test result' gidx2-lint-full.log | awk '{p+=$4; f+=$6; i+=$8} END {print "lint tests: " p " passed, " f " failed, " i " ignored"}'
tail -2 gidx2-merge-gate.log
grep -E '^(A[0-9]|W[0-9]|T[0-9]|A7)|false hits|acceptance exit' gidx2-acceptance-go.log | grep -E 'A3|A7:|10|exit|T7|W4 ok' | cut -c1-200
