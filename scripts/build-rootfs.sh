#!/usr/bin/env bash
# Builds the step rootfs (Dockerfile.rootfs) for the Docker host's
# architecture, exports its filesystem into the dev namespace's work volume
# and imports it into the dev container's store with `quod rootfs
# import`, which prints the digest the step key pins. Record that digest in
# versions.toml [rootfs].
#
#   QUOD_DEV_NS=rootfs scripts/build-rootfs.sh            # the pinned rootfs
#   QUOD_DEV_NS=rootfs scripts/build-rootfs.sh --extra jq # a different one (miss test)
#   scripts/build-rootfs.sh --export-only                      # skip the import
#
# Runs on the Docker host (macOS). Everything that pins the content lives in
# versions.toml: the base image's OCI digest per arch, the Debian snapshot
# time for the packages, the Go tarball's sha256 per arch.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$root/versions.toml"; }
extra=""; import=1
while [ $# -gt 0 ]; do
  case $1 in
    --extra) extra=$2; shift 2 ;;
    --export-only) import=0; shift ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
arch=$(docker version --format '{{.Server.Arch}}')   # arm64 or amd64
go=$(v go version)
base_digest=$(v rootfs "base_digest_$arch")
go_sha=$(v rootfs "go_sha256_$arch")
snapshot=$(v rootfs apt_snapshot)
[ -n "$base_digest" ] && [ -n "$go_sha" ] || { echo "versions.toml [rootfs] has no pins for $arch" >&2; exit 1; }

ctx=$root/target/rootfs-ctx-$arch
mkdir -p "$ctx"
if ! echo "$go_sha  $ctx/go.tgz" | shasum -a 256 -c --status 2>/dev/null; then
  curl -fsSL -o "$ctx/go.tgz" "https://go.dev/dl/go${go}.linux-${arch}.tar.gz"
  echo "$go_sha  $ctx/go.tgz" | shasum -a 256 -c --status || { echo "go tarball sha256 mismatch" >&2; exit 1; }
fi
name="go${go}-bookworm-${arch}${extra:+-$(echo "$extra" | tr ' ,' '--')}"
tag="quod-rootfs:$name"
echo "== building $tag (base $base_digest, snapshot $snapshot${extra:+, extra: $extra})"
docker build -q -f "$root/Dockerfile.rootfs" -t "$tag" \
  --build-arg BASE_DIGEST="$base_digest" --build-arg APT_SNAPSHOT="$snapshot" \
  --build-arg EXTRA_PKGS="$extra" "$ctx" >/dev/null
ns=${QUOD_DEV_NS:+-$QUOD_DEV_NS}
cid=$(docker create "$tag" true)
echo "== exporting to volume stillpass-work$ns:/work/rootfs/$name.tar"
docker export "$cid" | docker run -i --rm -v "stillpass-work$ns:/work" "$tag" \
  sh -c "mkdir -p /work/rootfs && cat > /work/rootfs/$name.tar"
docker rm "$cid" >/dev/null
docker image inspect "$tag" --format "image id {{.Id}}, {{.Size}} bytes"
if [ "$import" = 1 ]; then
  echo "== importing"
  "$root/scripts/dev.sh" "export CARGO_BUILD_JOBS=\${CARGO_BUILD_JOBS:-4}; cargo build -q --release -p quod-cli && target/release/quod rootfs import /work/rootfs/$name.tar --source $tag --oci $(docker image inspect "$tag" --format '{{.Id}}')"
fi
