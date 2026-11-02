#!/usr/bin/env bash
# Smoke: step A (network) through quod with Ubuntu's symlinked resolv.conf,
# then one keyed step; in a scratch state dir, deleted after.
set -u
. /etc/profile.d/quod.sh
S=/work/smoke; rm -rf $S; mkdir -p $S
git clone -q /work/cli-src $S/cli && git -C $S/cli checkout -q -B s b1c8c0ae5aec75ec8caf6e537071efc15d274f45
echo .quod/ >> $S/cli/.git/info/exclude
Q=/opt/quod/target/release/quod
$Q --state $S/state run --repo $S/cli --allow-net --step "go mod download"; echo "STEP_A_EXIT=$?"
$Q --state $S/state run --repo $S/cli --mode replay --warm --step "go vet ./internal/text"; echo "VET_EXIT=$?"
tail -2 $S/state/events.jsonl | cut -c1-600
mount | grep -c -E 'fuse|overlay' || true
rm -rf $S
echo SMOKE_DONE
