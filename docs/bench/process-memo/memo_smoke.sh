#!/bin/bash
# Three runs of four packages through pmemo in replay mode on the smoke tree:
# first records, second proves, third replays. Prints each run's outcomes.
set -u
export CGO_ENABLED=0 GOTOOLCHAIN=local LANG=C.UTF-8 TZ=UTC PATH=/usr/local/go/bin:/usr/bin:/bin
export GOMODCACHE=/work/hist/plain/gomod GOPATH=/work/hist/plain/gopath GOTELEMETRY=off GOPROXY=off GODEBUG=goindex=0
export GOCACHE=/work/pm/smoke/gocache HOME=/work/pm/smoke/home PMEMO_TMP=/work/pm/smoke/tmp
export GOFLAGS="-mod=mod -count=1 -toolexec=/work/pm/pmemo -exec=/work/pm/pmemo"
export PMEMO_STORE=/work/pm/smoke/store PMEMO_MODE=replay PMEMO_PROOFS=2 PMEMO_WS=/work/pm/smoke/cli
cd /work/pm/smoke/cli
PK="./internal/text/ ./pkg/cmd/ruleset/... ./pkg/cmd/repo/create/ ./pkg/cmd/extension/"
for n in 1 2 3; do
  export PMEMO_LOG=/work/pm/smoke/memo$n.jsonl; rm -f $PMEMO_LOG
  s=$(date +%s.%N); go test $PK > /work/pm/smoke/memo$n.out 2>&1; rc=$?; e=$(date +%s.%N)
  echo "== run $n exit=$rc wall=$(echo "$e - $s" | bc)"
  grep -E "^(ok|FAIL|---)" /work/pm/smoke/memo$n.out | sed 's/\t[0-9.]*s$//'
  python3 -c "
import json,collections
c=collections.Counter()
for l in open('$PMEMO_LOG'):
    r=json.loads(l); c[(r['kind'],r['outcome'])]+=1
    if r['outcome']=='opaque': print('  opaque', r['unit'], r.get('why'))
print(' ', dict(c))
"
done
