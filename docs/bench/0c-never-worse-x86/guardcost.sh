#!/bin/bash
# What the guard costs go test by itself (no Quod, no FUSE): warm each flavor's
# cache at commit 6, then time commit 7 (a dependency bump) with and without it.
set -u
W=/work/nw/exp
Q=/opt/quod-nw/target/release/quod
C6=401854ece8; C7=1531837d85
rm -rf $W; mkdir -p $W; chown 1001:1001 $W
git clone -q /work/cli-src $W/cli && git -c safe.directory=* -C $W/cli checkout -q -B x $C6
echo .quod/ >> $W/cli/.git/info/exclude
# The guard's files, from a kept Quod run (the overlay names sandbox paths).
$Q --state $W/qs run --repo $W/cli --keep --warm --step "go test ./internal/text" > /dev/null 2>&1
g=$(ls -d $W/qs/runs/*/go-test-guard | head -1)
mkdir -p $W/guard && cp $g/* $W/guard/
sed -i "s#/quod/go-test-guard/v1#$W/guard#g" $W/guard/overlay.json
chown -R 1001:1001 $W
run() { # flavor gocache flags label
  setpriv --reuid=1001 --regid=1001 --clear-groups env -i PATH=/usr/local/go/bin:/usr/bin:/bin \
    CGO_ENABLED=0 GOTOOLCHAIN=local LANG=C.UTF-8 TZ=UTC GOTELEMETRY=off GOPROXY=off GODEBUG=goindex=0 \
    GOMODCACHE=/work/nw/hist/plain/gomod GOPATH=$W/gopath HOME=$W/home-$1 GOCACHE=$W/cache-$1 \
    GOFLAGS="-mod=mod $2" sh -c "cd $W/cli && go test ./..." > $W/$1-$3.out 2>&1
}
for f in plain guard; do
  flags=; [ $f = guard ] && flags="-overlay=$W/guard/overlay.json"
  git -c safe.directory=* -C $W/cli checkout -q -f $C6
  run $f "$flags" c6
  git -c safe.directory=* -C $W/cli checkout -q -f $C7
  t0=$(date +%s.%N); run $f "$flags" c7; t1=$(date +%s.%N)
  echo "$f c7: $(python3 -c "print(round($t1-$t0,1))") s, cached $(grep -c '(cached)' $W/$f-c7.out), ran $(grep -c '^ok' $W/$f-c7.out), fail $(grep -c '^FAIL' $W/$f-c7.out)"
done
