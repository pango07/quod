#!/usr/bin/env bash
# The spike is done when this passes from a clean checkout, for every corpus
# repo.
#   scripts/acceptance.sh [go|node|python] [checkout-dir]
# Clones the repo at its pinned commit (versions.toml: [spike_repo] for go,
# [corpus.<toolchain>] otherwise) if needed, fills its dependency tree with
# step A (in the state dir's deps, not the checkout: crates/sp-cli/src/deps.rs),
# then runs A1-A7 (+A7b, and W1-W4 where the profile has warm state)
# against a copy of it, followed by the trust suite (T1-T11) and the
# observer failure paths (T7) on the tiny Go module.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$root/versions.toml"; }
tc=${1:-go}
case "$tc" in
  go) section=spike_repo; corpus_setup=; corpus_step=;;
  node|python) section="corpus.$tc"; corpus_setup=$(v "$section" setup); corpus_step=$(v "$section" step);;
  *) echo "usage: scripts/acceptance.sh [go|node|python] [checkout-dir]" >&2; exit 2;;
esac
url=$(v "$section" url); commit=$(v "$section" commit)
[ -n "$url" ] && [ -n "$commit" ] || { echo "versions.toml has no [$section] url/commit" >&2; exit 2; }
repo=${2:-${STILLPASS_SPIKE_DIR:-$root/target/corpus-$tc}}

"$root/scripts/check-host.sh" >/dev/null || { "$root/scripts/check-host.sh"; exit 1; }
if [ ! -d "$repo/.git" ]; then git clone -q "$url" "$repo"; fi
git -C "$repo" fetch -q origin "$commit" 2>/dev/null || true
git -C "$repo" checkout -q --detach "$commit"
cd "$root"
cargo build -q --release -p sp-cli
sp=${CARGO_TARGET_DIR:-$root/target}/release/stillpass
# The profile's defaults for anything versions.toml leaves empty, and the
# directories step A leaves behind that are not step B's inputs.
eval "$("$sp" profile --repo "$repo" --toolchain "$tc")"
setup=${corpus_setup:-$setup}
# STILLPASS_STEP overrides the corpus step (a parallelism cap on a shared box).
step=${STILLPASS_STEP:-${corpus_step:-$test}}
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
# Dependency trees live outside the checkout and outside the suite's temp
# state, so step A fetches them once per box, not per run.
export STILLPASS_DEPS_DIR="${STILLPASS_DEPS_DIR:-${STILLPASS_HOME:-$HOME/.cache/stillpass}/deps}"
echo "== step A ($tc): $setup (network, not memoized; deps in $STILLPASS_DEPS_DIR)"
"$sp" --state "$state" run --repo "$repo" --commit "$commit" --toolchain "$tc" --allow-net --step "$setup"
# Step B starts from a tree whose only earlier output is the dependency tree.
for d in $ephemeral; do rm -rf "$repo/$d"; done
echo "== A1-A7 on $url@${commit:0:12} ($tc profile), step: $step"
STILLPASS_INTEGRATION=1 STILLPASS_SPIKE_REPO="$repo" STILLPASS_TOOLCHAIN="$tc" STILLPASS_STEP="$step" \
  STILLPASS_A7B_SAMPLES="${STILLPASS_A7B_SAMPLES:-10}" \
  cargo test -q --release -p sp-cli --test acceptance -- --exact acceptance --nocapture 2>&1 \
  | grep -E '^(step A|A[0-9]|P[0-9]|W[0-9]|W:|acceptance|manifest:|\$ stillpass|STILLPASS|  )|panicked|assertion|FAILED|test result'
echo "== trust suite (tiny Go module)"
# Its temp repos get their own deps dirs inside their temp states.
env -u STILLPASS_DEPS_DIR STILLPASS_INTEGRATION=1 cargo test -q --release -p sp-cli --test trust -- --nocapture --test-threads=1 2>&1 \
  | grep -E '^(T[0-9]|\$ stillpass|STILLPASS|  )|panicked|assertion|FAILED|test result'
# The ring case runs where the host offers FUSE over io_uring.
ring=; grep -qx Y /sys/module/fuse/parameters/enable_uring 2>/dev/null && ring=--include-ignored
echo "== observer failure paths, T7 (fault-injection build; ring too: ${ring:-no})"
env -u STILLPASS_DEPS_DIR STILLPASS_INTEGRATION=1 cargo test -q -p sp-cli --features fault-injection --test observer_faults -- $ring --nocapture 2>&1 \
  | grep -E '^(T[0-9]|\$ stillpass|STILLPASS|  )|panicked|assertion|FAILED|test result'
