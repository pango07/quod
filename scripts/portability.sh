#!/usr/bin/env bash
# Portability proof. Host 1 is scripts/dev.sh (Debian bookworm, glibc 2.36,
# bubblewrap 0.8, Go on the host); host 2 is scripts/dev-host2.sh (Ubuntu
# 24.04, glibc 2.39, bubblewrap 0.9, no Go on the host). Both are containers
# on one VM, so kernel, CPU, RAM, sysctls and seccomp are shared: this shows
# that the key ignores the host's userland and that the result does not
# depend on it, not that a different kernel or machine would hit (it would
# not: those are in the key).
#
#   STILLPASS_DEV_NS=rootfs scripts/portability.sh [image-tag] [other-image-tag]
#
# 1. Host 1, fresh shared state: imports the pinned rootfs and its `--extra
#    jq` variant, clones cli/cli, fills the module cache with step A, runs
#    step B (executes and records).
# 2. Host 2 imports its own `docker export` of the image into a private store
#    on its own loop-mounted ext4 (another filesystem, another directory hash
#    seed): the digest must be the same.
# 3. Host 2 runs step B fresh on the private store (its own bubblewrap 0.9,
#    an empty ledger, run directories on the other filesystem): the host
#    class differs only in the sandbox's version, so the key differs and it
#    executes; the two executions are compared (key documents, manifest
#    entries, exit code, stdout with go test durations normalized, outputs).
# 4. Host 2 with Debian's bubblewrap 0.8.0 first on PATH is host 1's class:
#    against the shared ledger it must replay host 1's proof.
# 5. The jq rootfs misses; the host-root fallback on host 2 (no Go) refuses.
#
# Tags default to the pinned rootfs and its jq variant built by
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
echo "== host 2 also gets host 1's bubblewrap binary (Debian 0.8.0), for step 4"
dev_img=stillpass-dev:go$(v go version)-rust$(v rust toolchain)
docker run --rm "$dev_img" cat /usr/bin/bwrap | docker run -i --rm -v "stillpass-work$ns-host2:/work" debian:bookworm-slim \
  sh -c "mkdir -p /work/bwrap-0.8 && cat > /work/bwrap-0.8/bwrap && chmod 755 /work/bwrap-0.8/bwrap"

# Both hosts: describe the host, build stillpass. $1: the shared state dir.
host_script() {
  cat <<EOS
set -eu
export CARGO_BUILD_JOBS=\${CARGO_BUILD_JOBS:-4}
shared=$1
echo "--- host: \$(. /etc/os-release && echo "\$PRETTY_NAME"), \$(ldd --version | head -1), \$(bwrap --version), \$(dpkg-query -W -f '\${Package}\n' | wc -l) packages, go on host: \$(command -v go || echo none), kernel \$(uname -r) \$(uname -m), load \$(cut -d' ' -f1-3 /proc/loadavg)"
cargo build -q --release -p sp-cli
sp() { /src/target/release/stillpass "\$@"; }
EOS
}
# Clone the spike repo at the pinned commit and fill its module cache with
# step A in the rootfs. $1: the state dir to use for step A's store.
step_a_script() {
  cat <<EOS
cd /work
if [ ! -d cli/.git ]; then git clone -q $url cli; fi
git -C cli fetch -q origin $commit 2>/dev/null || true
git -C cli checkout -q --detach $commit
grep -qx '.stillpass/' cli/.git/info/exclude || echo '.stillpass/' >> cli/.git/info/exclude
rm -rf cli/.stillpass
echo "--- step A in the rootfs (network, not memoized)"
sp --state $1 run --repo /work/cli --commit $commit --allow-net --rootfs \$rootfs --step "go mod download" >/dev/null
rm -rf cli/.stillpass/gocache cli/.stillpass/home
cd /src
EOS
}
# Step B with state $1, stdout to $2.out, stderr to $2.err; prints the banner.
step_b_script() {
  cat <<EOS
sp --state $1 run --repo /work/cli --commit $commit --rootfs \$rootfs --step "$step" > $2.out 2> $2.err || true
grep -E '^STILLPASS|^  ' $2.err || true
echo "stdout: \$(wc -c < $2.out) bytes, sha256 \$(sha256sum < $2.out | cut -c1-16), \$(grep -c '^ok' $2.out) packages ok"
EOS
}

echo
echo "== host 1 (scripts/dev.sh), shared state /work/$shared"
"$root/scripts/dev.sh" "$(host_script /work/$shared)
mkdir -p \$shared
rootfs=\$(sp --state \$shared rootfs import /work/rootfs/$name.tar --source stillpass-rootfs:$name)
otherfs=\$(sp --state \$shared rootfs import /work/rootfs/$other.tar --source stillpass-rootfs:$other)
echo \"rootfs \$rootfs, other \$otherfs\"
printf '%s\n%s\n' \$rootfs \$otherfs > \$shared/digests
$(step_a_script /work/$shared)
echo '--- step B with the pinned rootfs (EXECUTED expected)'
$(step_b_script /work/$shared /work/tmp/host1-b)"

echo
echo "== host 2 (scripts/dev-host2.sh), sharing only /work/$shared"
STILLPASS_SHARED=$shared "$root/scripts/dev-host2.sh" "$(host_script /work/state)
rootfs=\$(sed -n 1p \$shared/digests); otherfs=\$(sed -n 2p \$shared/digests)
# A private store on a filesystem of its own: a fresh ext4 in a loop file.
umount /mnt/fs2 2>/dev/null || true
rm -f /work/fs2.img; truncate -s 24G /work/fs2.img; mkfs.ext4 -q -F /work/fs2.img
mkdir -p /mnt/fs2; mount -o loop /work/fs2.img /mnt/fs2
private=/mnt/fs2/state; mkdir -p \$private
echo \"--- private store on \$(stat -f -c %T /mnt/fs2) (loop file, own hash seed), shared on \$(stat -f -c %T \$shared)\"
own=\$(sp --state \$private rootfs import /work/rootfs/$name.tar)
echo \"own import of its own export: \$own (\$( [ \"\$own\" = \"\$rootfs\" ] && echo same digest || echo DIFFERENT))\"
$(step_a_script \$private)
echo \"--- step B fresh on the private store with this host's bubblewrap \$(bwrap --version | cut -d' ' -f2) (EXECUTED expected: another host class)\"
$(step_b_script \$private /work/tmp/host2-fresh)
echo '--- the two executions compared (host 1 recorded, host 2 fresh)'
q() { sqlite3 \"\$1\"/ledger.sqlite \"\$2\"; }
r1=\$(q \$shared 'SELECT json FROM results ORDER BY created_at LIMIT 1')
r2=\$(q \$private 'SELECT json FROM results ORDER BY created_at LIMIT 1')
k1=\$(echo \"\$r1\" | jq -r .step_key); k2=\$(echo \"\$r2\" | jq -r .step_key)
echo \"step key:    \$k1 vs \$k2; key documents differ in:\"
diff <(q \$shared \"SELECT json FROM step_keys WHERE step_key='\$k1'\" | jq -S .) <(q \$private \"SELECT json FROM step_keys WHERE step_key='\$k2'\" | jq -S .) | grep '^[<>]' | sed 's/^/    /'
m1=\$(q \$shared \"SELECT json FROM manifests WHERE id='\$(echo \"\$r1\" | jq -r .manifest_id)'\" | jq -c .entries | sha256sum | cut -c1-16)
m2=\$(q \$private \"SELECT json FROM manifests WHERE id='\$(echo \"\$r2\" | jq -r .manifest_id)'\" | jq -c .entries | sha256sum | cut -c1-16)
echo \"manifest entries: \$(q \$shared \"SELECT json FROM manifests WHERE id='\$(echo \"\$r1\" | jq -r .manifest_id)'\" | jq '.entries | length') vs \$(q \$private \"SELECT json FROM manifests WHERE id='\$(echo \"\$r2\" | jq -r .manifest_id)'\" | jq '.entries | length'), sha256 \$m1 vs \$m2 (\$( [ \"\$m1\" = \"\$m2\" ] && echo identical: both read exactly the same inputs || echo DIFFERENT))\"
echo \"exit code:   \$(echo \"\$r1\" | jq -r .exit_code) vs \$(echo \"\$r2\" | jq -r .exit_code)\"
norm() { sed -E 's/[[:space:]]+[0-9]+\\.[0-9]+s\$//'; }
blob() { h=\$(echo \"\$2\" | jq -r .\$3 | cut -c4-); cat \"\$1\"/cas/\$(echo \$h | cut -c1-2)/\$h; }
if cmp -s <(blob \$shared \"\$r1\" stdout | norm) <(blob \$private \"\$r2\" stdout | norm); then echo 'stdout:      identical with durations normalized'; else echo 'stdout:      DIFFERENT'; diff <(blob \$shared \"\$r1\" stdout | norm) <(blob \$private \"\$r2\" stdout | norm) | head -20; fi
outs() { echo \"\$1\" | jq -r '.outputs[] | [.p, .k, (.m // \"\" | tostring), (.h // \"\")] | @tsv' | sort; }
outs \"\$r1\" > /work/tmp/o1; outs \"\$r2\" > /work/tmp/o2
cut -f1 /work/tmp/o1 | sort > /work/tmp/p1; cut -f1 /work/tmp/o2 | sort > /work/tmp/p2
comm -12 /work/tmp/p1 /work/tmp/p2 > /work/tmp/pb
comm -3 /work/tmp/o1 /work/tmp/o2 | sed 's/^\\t//' | cut -f1 | sort -u | comm -12 - /work/tmp/pb > /work/tmp/pd
comm -3 /work/tmp/p1 /work/tmp/p2 | sed 's/^\\t//' > /work/tmp/po
echo \"outputs:     \$(wc -l < /work/tmp/o1) vs \$(wc -l < /work/tmp/o2); identical path, kind, mode and content: \$(comm -12 /work/tmp/o1 /work/tmp/o2 | wc -l)\"
echo \"  same path, different content: \$(wc -l < /work/tmp/pd), of which Go cache action entries (*-a, they hold a creation time): \$(grep -c '^.stillpass/gocache/.*-a\$' /work/tmp/pd || true)\"
echo \"  paths only one run wrote: \$(wc -l < /work/tmp/po), of which Go cache output entries (*-d, named by content: cached test output holds durations): \$(grep -c '^.stillpass/gocache/.*-d\$' /work/tmp/po || true)\"
echo '  every other difference:'
cat /work/tmp/pd /work/tmp/po | grep -v '^.stillpass/gocache/.*-[ad]\$' | sort -u | sed 's/^/    /'
echo \"root outputs: \$(echo \"\$r1\" | jq '.root_outputs | length') vs \$(echo \"\$r2\" | jq '.root_outputs | length') (cli/cli's tests leave randomly named files in /tmp)\"
echo \"--- step B against the shared ledger with Debian's bubblewrap 0.8.0 first on PATH (host 1's class: REPLAYED expected)\"
PATH=/work/bwrap-0.8:\$PATH
echo \"    bwrap on PATH: \$(bwrap --version)\"
$(step_b_script \$shared /work/tmp/host2-replay)
echo '--- step B with a different rootfs (MISS expected)'
sp --state \$shared run --repo /work/cli --commit $commit --rootfs \$otherfs --step \"$step\" > /dev/null 2> /work/tmp/stepB2.err || true
grep -E '^STILLPASS|^  (image|no manifests|ran)' /work/tmp/stepB2.err || true
echo '--- step B on the host root (the fallback; this host has no Go)'
sp --state \$shared run --repo /work/cli --commit $commit --host-root --step \"$step\" > /dev/null 2> /work/tmp/stepB3.err || true
grep -E '^STILLPASS|^  ' /work/tmp/stepB3.err | head -3 || true
umount /mnt/fs2; rm -f /work/fs2.img
"
