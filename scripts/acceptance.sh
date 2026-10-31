#!/usr/bin/env bash
# The spike is done when this passes from a clean checkout.
#   scripts/acceptance.sh [spike-checkout-dir]
# Clones the spike repo at the pinned commit (versions.toml) if needed, fills
# its module cache with step A, then runs A1-A7 (+A7b) against a copy of it.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$root/versions.toml"; }
url=$(v spike_repo url); commit=$(v spike_repo commit)
repo=${1:-${STILLPASS_SPIKE_DIR:-$root/target/spike-repo}}

"$root/scripts/check-host.sh" >/dev/null || { "$root/scripts/check-host.sh"; exit 1; }
if [ ! -d "$repo/.git" ]; then git clone -q "$url" "$repo"; fi
git -C "$repo" fetch -q origin "$commit" 2>/dev/null || true
git -C "$repo" checkout -q --detach "$commit"
grep -qx '.stillpass/' "$repo/.git/info/exclude" || echo '.stillpass/' >> "$repo/.git/info/exclude"
cd "$root"
cargo build -q --release -p sp-cli
state=$(mktemp -d)
trap 'rm -rf "$state"' EXIT
# Rootfs mode: STILLPASS_ROOTFS names the pinned rootfs and
# STILLPASS_ROOTFS_SRC its export (scripts/build-rootfs.sh); every state dir
# the suite makes imports it first. Without them the host is the pin.
if [ -n "${STILLPASS_ROOTFS:-}" ]; then
  : "${STILLPASS_ROOTFS_SRC:?STILLPASS_ROOTFS is set: STILLPASS_ROOTFS_SRC must name its export}"
  echo "== rootfs mode: importing $STILLPASS_ROOTFS_SRC"
  got=$(target/release/stillpass --state "$state" rootfs import "$STILLPASS_ROOTFS_SRC")
  [ "$got" = "$STILLPASS_ROOTFS" ] || { echo "export imports to $got, not $STILLPASS_ROOTFS" >&2; exit 1; }
else
  echo "== host mode (no STILLPASS_ROOTFS): the key pins this host's fingerprint"
fi
echo "== step A: go mod download (network, not memoized)"
target/release/stillpass --state "$state" run --repo "$repo" --commit "$commit" --allow-net --step "go mod download"
# Step B starts from a tree whose only earlier output is the module cache.
rm -rf "$repo/.stillpass/gocache" "$repo/.stillpass/home"
echo "== A1-A7 on $url@${commit:0:12}"
STILLPASS_INTEGRATION=1 STILLPASS_SPIKE_REPO="$repo" STILLPASS_A7B_SAMPLES="${STILLPASS_A7B_SAMPLES:-10}" \
  cargo test -q --release -p sp-cli --test acceptance -- --nocapture 2>&1 \
  | grep -E '^(A[0-9]|W[0-9]|acceptance|\$ stillpass|STILLPASS|  )|panicked|assertion|FAILED|test result'
