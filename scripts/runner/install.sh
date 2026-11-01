#!/usr/bin/env bash
# Installs one official actions/runner wired to Quod's container hooks.
#   scripts/runner/install.sh <owner/repo> <registration-token> [dir] [name]
# Downloads the runner pinned in versions.toml (linux, x64 or arm64), installs
# the hook shim at /opt/quod/hooks/index.js with its quod.json
# (binary, state dir QUOD_HOME, policy mode QUOD_MODE: read here,
# at install time only) and the quod binary at /opt/quod/bin
# (QUOD_BIN overrides the source), writes the runner's .env, and
# registers it with the label `quod`.
set -euo pipefail
repo=${1:?owner/repo}; token=${2:?registration token}
dir=${3:-/opt/actions-runner}; name=${4:-quod-$(hostname)}
root=$(cd "$(dirname "$0")/../.." && pwd)
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$root/versions.toml"; }
ver=$(v actions_runner version)
case "$(uname -m)" in
  x86_64) arch=x64 ;;
  aarch64|arm64) arch=arm64 ;;
  *) echo "unsupported arch $(uname -m)" >&2; exit 1 ;;
esac

if [ ! -x "$dir/run.sh" ]; then
  mkdir -p "$dir"
  tarball="actions-runner-linux-$arch-$ver.tar.gz"
  echo "downloading actions/runner $ver ($arch)"
  curl -fsSL -o "/tmp/$tarball" "https://github.com/actions/runner/releases/download/v$ver/$tarball"
  # The release notes' SHA-256, pinned in versions.toml: refuse anything else.
  want=$(v actions_runner "sha256_$arch")
  if [ -z "$want" ] || ! echo "$want  /tmp/$tarball" | sha256sum -c --quiet -; then
    echo "actions/runner $ver ($arch): checksum does not match versions.toml" >&2
    rm -f "/tmp/$tarball"
    exit 1
  fi
  tar -xzf "/tmp/$tarball" -C "$dir"
  rm -f "/tmp/$tarball"
fi

mkdir -p /opt/quod/hooks /opt/quod/bin
install -m 0644 "$root/scripts/runner/hooks/index.js" /opt/quod/hooks/index.js
src=${QUOD_BIN:-$root/target/release/quod}
if [ -x "$src" ]; then
  install -m 0755 "$src" /opt/quod/bin/quod
else
  echo "note: no quod binary at $src yet (cargo build --release -p quod-cli); rerun to install it" >&2
fi

# The shim's configuration: the binary, the state and the policy. Never the
# runner's environment: the runner hands the hook the step's environment.
state=${QUOD_HOME:-/var/lib/quod}
mode=${QUOD_MODE:-observe}
cat > /opt/quod/hooks/quod.json <<EOF
{"bin": "/opt/quod/bin/quod", "state": "$state", "policy": {"mode": "$mode"}}
EOF
cat > "$dir/.env" <<EOF
ACTIONS_RUNNER_CONTAINER_HOOKS=/opt/quod/hooks/index.js
ACTIONS_RUNNER_REQUIRE_JOB_CONTAINER=true
EOF

if [ ! -f "$dir/.runner" ]; then
  (cd "$dir" && ./config.sh --unattended --replace \
      --url "https://github.com/$repo" --token "$token" \
      --name "$name" --labels quod --work _work)
fi
echo "runner $name installed in $dir; start it with: cd $dir && ./run.sh"
