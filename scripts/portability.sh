#!/usr/bin/env bash
# Portability proof. The same tree, run with the same rootfs on two different
# hosts, HITs; a different rootfs misses. Host 1 is scripts/dev.sh (Debian
# bookworm, Go on the host); host 2 is scripts/dev-host2.sh (Ubuntu 24.04,
# other packages and glibc, other bubblewrap, no Go on the host). They share
# exactly one directory, a fresh one per run (CAS, ledger, rootfs
# snapshots); each clones the spike repo and fills its own module cache with
# step A, and host 2 imports its own export of the image into a private
# store first, to show it computes the same digest without the shared one.
#
#   STILLPASS_DEV_NS=rootfs scripts/portability.sh [image-tag] [other-image-tag]
#
# Tags default to the pinned rootfs and its `--extra jq` variant built by
# scripts/build-rootfs.sh; their exports must be in host 1's work volume
# under /work/rootfs/<name>.tar.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$root/versions.toml"; }
url=$(v spike_repo url); commit=$(v spike_repo commit)
arch=$(docker version --format '{{.Server.Arch}}')
name=${1:-go$(v go version)-bookworm-$arch}
other=${2:-$name-jq}
step=${STILLPASS_STEP:-"go test ./..."}
ns=${STILLPASS_DEV_NS:+-$STILLPASS_DEV_NS}
shared=portability-$(date +%Y%m%dT%H%M%S)

echo "== host 2 gets its own export of stillpass-rootfs:$name"
cid=$(docker create "stillpass-rootfs:$name" true)
docker export "$cid" | docker run -i --rm -v "stillpass-work$ns-host2:/work" debian:bookworm-slim \
  sh -c "mkdir -p /work/rootfs && cat > /work/rootfs/$name.tar"
docker rm "$cid" >/dev/null

# What both hosts do: describe the host, build stillpass, clone the spike repo
# at the pinned commit, run step A in the rootfs (network, not memoized),
# then step B. $1: the state dir, $2: what step B should print.
host_script() {
  cat <<EOS
set -eu
export CARGO_BUILD_JOBS=\${CARGO_BUILD_JOBS:-4}
state=$1
echo "--- host: \$(. /etc/os-release && echo "\$PRETTY_NAME"), \$(ldd --version | head -1), \$(bwrap --version), \$(dpkg-query -W -f '\${Package}\n' | wc -l) packages, go on host: \$(command -v go || echo none), kernel \$(uname -r) \$(uname -m), load \$(cut -d' ' -f1-3 /proc/loadavg)"
cargo build -q --release -p sp-cli
sp() { /src/target/release/stillpass --state \$state "\$@"; }
EOS
}
step_script() {
  cat <<EOS
cd /work
if [ ! -d cli/.git ]; then git clone -q $url cli; fi
git -C cli fetch -q origin $commit 2>/dev/null || true
git -C cli checkout -q --detach $commit
grep -qx '.stillpass/' cli/.git/info/exclude || echo '.stillpass/' >> cli/.git/info/exclude
rm -rf cli/.stillpass
echo "--- step A in the rootfs (network, not memoized)"
sp run --repo /work/cli --commit $commit --allow-net --rootfs \$rootfs --step "go mod download" >/dev/null
rm -rf cli/.stillpass/gocache cli/.stillpass/home
echo "--- step B with the pinned rootfs ($1 expected)"
sp run --repo /work/cli --commit $commit --rootfs \$rootfs --step "$step" > /work/tmp/stepB.out 2> /work/tmp/stepB.err || true
grep -E '^STILLPASS|^  ' /work/tmp/stepB.err || true
echo "stdout: \$(wc -c < /work/tmp/stepB.out) bytes, sha256 \$(sha256sum < /work/tmp/stepB.out | cut -c1-16), \$(grep -c '^ok' /work/tmp/stepB.out) packages ok"
EOS
}

echo
echo "== host 1 (scripts/dev.sh), shared state /work/$shared"
"$root/scripts/dev.sh" "$(host_script /work/$shared)
mkdir -p \$state
rootfs=\$(sp rootfs import /work/rootfs/$name.tar --source stillpass-rootfs:$name)
otherfs=\$(sp rootfs import /work/rootfs/$other.tar --source stillpass-rootfs:$other)
echo \"rootfs \$rootfs, other \$otherfs\"
printf '%s\n%s\n' \$rootfs \$otherfs > \$state/digests
$(step_script EXECUTED)"

echo
echo "== host 2 (scripts/dev-host2.sh), sharing only /work/$shared"
STILLPASS_SHARED=$shared "$root/scripts/dev-host2.sh" "$(host_script /work/state)
rootfs=\$(sed -n 1p \$state/digests); otherfs=\$(sed -n 2p \$state/digests)
rm -rf /work/private
own=\$(/src/target/release/stillpass --state /work/private rootfs import /work/rootfs/$name.tar)
echo \"own import of its own export: \$own (\$( [ \"\$own\" = \"\$rootfs\" ] && echo same digest || echo DIFFERENT))\"
$(step_script REPLAYED)
echo '--- step B with a different rootfs (MISS expected)'
sp run --repo /work/cli --commit $commit --rootfs \$otherfs --step \"$step\" > /dev/null 2> /work/tmp/stepB2.err || true
grep -E '^STILLPASS|^  (image|no manifests|ran)' /work/tmp/stepB2.err || true
echo '--- step B on the host root (the fallback; this host has no Go)'
sp run --repo /work/cli --commit $commit --host-root --step \"$step\" > /dev/null 2> /work/tmp/stepB3.err || true
grep -E '^STILLPASS|^  (image|no manifests|ran)' /work/tmp/stepB3.err || true
"
