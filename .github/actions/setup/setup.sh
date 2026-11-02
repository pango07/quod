#!/usr/bin/env bash
# The Quod setup action's three steps (action.yml): `host` prepares the
# runner as .github/actions/quod-host does for the gates (scripts/ci-gate.sh
# setup), `install` puts the client at /usr/local/lib/quod with a `quod`
# wrapper on PATH, `env` points later steps at the server. Self-contained:
# a customer's repository has none of this repository's scripts.
set -euo pipefail
ring_param=/sys/module/fuse/parameters/enable_uring
lib=/usr/local/lib/quod
state=/var/lib/quod
die() { echo "::error title=quod setup::$*" >&2; exit 1; }
s=; [ "$(id -u)" = 0 ] || s=sudo

host() {
  [ "$(uname -s)" = Linux ] || die "Quod runs on Linux runners (this is $(uname -s))"
  case "$(uname -m)" in
    x86_64) arch=amd64 narch=x64 triple=x86_64-unknown-linux-gnu;;
    *) die "the quod client is released for x86_64 only so far (this is $(uname -m))";;
  esac
  $s apt-get update -qq
  $s apt-get install -y -qq --no-install-recommends bubblewrap fuse3 >/dev/null
  grep -qs '^user_allow_other' /etc/fuse.conf || echo user_allow_other | $s tee -a /etc/fuse.conf >/dev/null
  # Ubuntu restricts unprivileged user namespaces through AppArmor.
  $s sysctl -q -w kernel.apparmor_restrict_unprivileged_userns=0 2>/dev/null || true
  # quod runs as root (the sandbox and FUSE need it) and reads git in the
  # runner user's checkout: no "dubious ownership" refusals.
  $s git config --system --add safe.directory '*'
  # FUSE over io_uring when the kernel has it; else the classic transport.
  if [ -e "$ring_param" ]; then
    echo Y | $s tee "$ring_param" >/dev/null 2>&1 || echo "quod setup: could not enable FUSE io_uring"
  fi
  $s mkdir -p "$state"
  $s chmod 0700 "$state"
  # Every step key pins the host's /etc (the sandbox binds it read-only),
  # and a GitHub-hosted VM has files there no other VM shares: its SSH host
  # keys and cloud-init's netplan file (the NIC's MAC address). Neither is
  # used after boot on an ephemeral runner (no sshd session, the network is
  # up), so they are moved out of /etc: every VM of an image then shows its
  # steps the same /etc, and a result proved on one replays on the next.
  if [ "${QUOD_SETUP_NORMALIZE_ETC:-true}" = true ]; then
    $s mkdir -p "$state/etc-moved"
    local f
    for f in /etc/ssh/ssh_host_*_key /etc/ssh/ssh_host_*_key.pub /etc/netplan/50-cloud-init.yaml; do
      [ -e "$f" ] && $s mv "$f" "$state/etc-moved/" || true
    done
  fi
  local tc
  for tc in $QUOD_SETUP_TOOLCHAINS; do
    case "$tc" in
      go)
        $s rm -rf /usr/local/go
        curl -fsSL "https://go.dev/dl/go$QUOD_SETUP_GO.linux-$arch.tar.gz" | $s tar -C /usr/local -xz
        echo /usr/local/go/bin >> "$GITHUB_PATH"
        /usr/local/go/bin/go version;;
      node)
        $s rm -rf /usr/local/node
        curl -fsSL "https://nodejs.org/dist/v$QUOD_SETUP_NODE/node-v$QUOD_SETUP_NODE-linux-$narch.tar.gz" | $s tar -C /usr/local -xz
        $s mv "/usr/local/node-v$QUOD_SETUP_NODE-linux-$narch" /usr/local/node
        echo /usr/local/node/bin >> "$GITHUB_PATH"
        echo "node $(/usr/local/node/bin/node --version)";;
      python)
        $s rm -rf /usr/local/python /usr/local/uv
        curl -fsSL "https://github.com/astral-sh/python-build-standalone/releases/download/$QUOD_SETUP_PBS/cpython-$QUOD_SETUP_PYTHON+$QUOD_SETUP_PBS-$triple-install_only_stripped.tar.gz" | $s tar -C /usr/local -xz
        $s mkdir -p /usr/local/uv/bin
        curl -fsSL "https://github.com/astral-sh/uv/releases/download/$QUOD_SETUP_UV/uv-$triple.tar.gz" | $s tar -C /usr/local/uv/bin -xz --strip-components=1
        printf '%s\n' /usr/local/python/bin /usr/local/uv/bin >> "$GITHUB_PATH"
        /usr/local/python/bin/python3 --version;;
      *) die "unknown toolchain '$tc' (go, node or python)";;
    esac
  done
  local ring io
  ring=$(cat "$ring_param" 2>/dev/null || echo absent)
  io=$(cat /proc/sys/kernel/io_uring_disabled 2>/dev/null || echo n/a)
  echo "quod host: kernel $(uname -r), $(nproc) CPUs, fuse.enable_uring=$ring, kernel.io_uring_disabled=$io"
}

# The pinned SHA-256 of each released client tarball (the release workflow,
# .github/workflows/quod-release.yml, prints it).
pinned_sha256() {
  case "$1" in
    # cc34bd9 (exp/cloud-ledger-v2), GitHub run 41511010172
    quod-client-v0.1.0) echo 0e0f8a26fe82cfa720a15742302c66c6867c470658d15db950729f9025af9b00;;
    # b452ae4 (exp/cloud-ledger-v3); see the release notes for the run
    *) echo '';;
  esac
}

install() {
  local v=$QUOD_SETUP_VERSION want=${QUOD_SETUP_SHA256:-} dir asset=quod-x86_64-linux.tar.gz
  [ -n "$want" ] || want=$(pinned_sha256 "$v")
  dir=$(mktemp -d)
  gh release download "$v" -R "$QUOD_SETUP_REPOSITORY" -p "$asset" -D "$dir" \
    || die "could not download $asset from $QUOD_SETUP_REPOSITORY release $v (the token must be able to read that repository)"
  local got
  got=$(sha256sum "$dir/$asset" | cut -c1-64)
  if [ -n "$want" ]; then
    [ "$got" = "$want" ] || die "$asset of $v has SHA-256 $got, not the pinned $want"
  else
    echo "::warning title=quod setup::no SHA-256 pinned for $v; got $got (set the sha256 input to pin it)"
  fi
  tar -C "$dir" -xzf "$dir/$asset"
  # Under /usr: the sandbox binds /usr, and quod runs itself inside it to
  # probe the host class.
  $s install -d -m 0755 "$lib"
  $s install -m 0755 "$dir/quod" "$lib/quod"
  rm -rf "$dir"
  # The `quod` steps call: quod as root, with the job's environment by name.
  # The environment travels in a 0600 file rather than on a command line, so
  # the OIDC request token is never in a process listing.
  $s tee "$lib/quod-root" >/dev/null <<'EOF'
#!/bin/bash
# Run by /usr/local/bin/quod through sudo: load the caller's environment,
# then exec quod in the caller's directory.
set -eu
f=$1 cwd=$2
shift 2
# shellcheck disable=SC1090
. "$f"
rm -f "$f"
export HOME=/root PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export QUOD_HOME=${QUOD_HOME:-/var/lib/quod}
cd "$cwd"
exec /usr/local/lib/quod/quod "$@"
EOF
  $s tee /usr/local/bin/quod >/dev/null <<'EOF'
#!/bin/bash
# quod (Quod setup action): runs /usr/local/lib/quod/quod as root, as its
# sandbox and FUSE observer need, with this step's CI, GitHub, runner, Quod
# and OIDC request variables.
set -eu
if [ "$(id -u)" = 0 ]; then
  export QUOD_HOME=${QUOD_HOME:-/var/lib/quod}
  exec /usr/local/lib/quod/quod "$@"
fi
f=$(umask 077 && mktemp "${RUNNER_TEMP:-/tmp}/quod-env.XXXXXX")
for n in $(compgen -e); do
  case "$n" in
    CI|LANG|TZ|TERM|NO_COLOR|QUOD_*|GITHUB_*|RUNNER_*|ACTIONS_ID_TOKEN_REQUEST_URL|ACTIONS_ID_TOKEN_REQUEST_TOKEN)
      printf 'export %s=%q\n' "$n" "${!n}";;
  esac
done > "$f"
exec sudo -n /usr/local/lib/quod/quod-root "$f" "$PWD" "$@"
EOF
  $s chmod 0755 "$lib/quod-root" /usr/local/bin/quod
  echo "version=$v" >> "$GITHUB_OUTPUT"
  echo "quod client $v ($got) at $lib/quod"
}

env_() {
  {
    echo "QUOD_HOME=$state"
    echo "QUOD_MODE=$QUOD_SETUP_MODE"
    [ -z "$QUOD_SETUP_JOB_ID" ] || echo "QUOD_JOB_ID=$QUOD_SETUP_JOB_ID"
    echo "QUOD_JOB_NAME=${GITHUB_JOB:-}"
  } >> "$GITHUB_ENV"
  if [ -z "$QUOD_SETUP_SERVER" ]; then
    echo "quod: no server; results stay on this runner"
    return
  fi
  [[ $QUOD_SETUP_SERVER_KEY =~ ^[0-9a-f]{64}$ ]] || die "server-key must be the server's public key, 64 hex digits (GET $QUOD_SETUP_SERVER/v1/keys)"
  printf 'QUOD_SERVER=%s\nQUOD_SERVER_KEY=%s\n' "$QUOD_SETUP_SERVER" "$QUOD_SETUP_SERVER_KEY" >> "$GITHUB_ENV"
  if [ -z "${ACTIONS_ID_TOKEN_REQUEST_URL:-}" ]; then
    echo "::warning title=quod setup::this job cannot request an OIDC token (add 'permissions: id-token: write'); quod run will execute every step and record nothing"
  fi
  # The server's key as it serves it now, against the pin: a mismatch is
  # worth knowing before any step runs (every replay would be refused).
  local served
  served=$(curl -fsS --max-time 10 "$QUOD_SETUP_SERVER/v1/keys" 2>/dev/null | grep -o '"public_key":"[0-9a-f]*"' | head -1 | cut -d'"' -f4 || true)
  if [ -z "$served" ]; then
    echo "::warning title=quod setup::$QUOD_SETUP_SERVER/v1/keys did not answer; quod run will execute steps and record nothing if the server stays unreachable"
  elif [ "$served" != "$QUOD_SETUP_SERVER_KEY" ]; then
    echo "::warning title=quod setup::$QUOD_SETUP_SERVER serves key $served, not the pinned $QUOD_SETUP_SERVER_KEY; every replay will be refused until the pin is updated"
  else
    echo "quod: server $QUOD_SETUP_SERVER, key pinned (${served:0:16}...), mode $QUOD_SETUP_MODE"
  fi
}

case "${1:-}" in
  host) host;;
  install) install;;
  env) env_;;
  *) die "usage: setup.sh host|install|env";;
esac
