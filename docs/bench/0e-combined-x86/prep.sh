#!/usr/bin/env bash
# Clone cli/cli and build quod at main (before measuring).
set -eux
. /etc/profile.d/quod.sh
snap refresh --hold || true
[ -d /work/cli-src ] || git clone -q https://github.com/cli/cli /work/cli-src
git -C /work/cli-src rev-parse 9490d204a2ba95434153dedce99898ce9f281426
git -C /work/cli-src log --first-parent --oneline -n 40 9490d204a2ba | tail -1
cd /opt/quod && time cargo build --release -q -p quod-cli
/opt/quod/target/release/quod --version
echo PREP_DONE
