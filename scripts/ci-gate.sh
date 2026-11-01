#!/usr/bin/env bash
# The heavy gates, one command each, run the same way on the dev box and in
# GitHub Actions (.github/workflows/gate.yml; docs/ci.md). Each gate wraps the
# script that already owns it (lint.sh, merge-gate.sh, acceptance.sh) or, for
# the hooks suite, the one cargo command; this file adds only corpus
# preparation, logs and the summary.
#
#   scripts/ci-gate.sh host                         kernel, CPUs, FUSE ring, check-host
#   scripts/ci-gate.sh setup [go] [node] [python]   CI only: host packages, FUSE, ring, toolchains
#   scripts/ci-gate.sh corpus go|node|python|hooks [dir]   clone at the pinned commit
#   scripts/ci-gate.sh lint
#   scripts/ci-gate.sh merge-gate
#   scripts/ci-gate.sh hooks [dir]                  hooks_acceptance on cli/cli
#   scripts/ci-gate.sh acceptance go|node|python [dir]
#   scripts/ci-gate.sh summary NAME                 markdown for $GITHUB_STEP_SUMMARY
#
# Locally, inside the dev container and under the shared lock:
#   scripts/dev.sh 'flock /work/tmp/heavy.lock scripts/ci-gate.sh hooks /work/cli-quod'
#
# Logs go to $QUOD_GATE_LOGS (default target/gate-logs), one NAME.log per gate.
# In CI (CI=true, not root) the gates re-run themselves under sudo, as the dev
# container runs everything as root, and hand target/ and ~/.cargo back to
# the runner user afterwards so the caches can be saved.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$root/versions.toml"; }
logs=${QUOD_GATE_LOGS:-$root/target/gate-logs}
ring_param=/sys/module/fuse/parameters/enable_uring

die() { echo "ci-gate: $*" >&2; exit 2; }

# Re-run as root in CI; a no-op on the dev box, which is root already.
as_root() {
  if [ "$(id -u)" = 0 ] || [ "${CI:-}" != true ]; then return 0; fi
  local rc=0
  sudo -E env "PATH=$PATH" "HOME=$HOME" "$0" "$@" || rc=$?
  sudo chown -R "$(id -u):$(id -g)" "$root/target" "${CARGO_HOME:-$HOME/.cargo}" 2>/dev/null || true
  exit "$rc"
}

host_line() {
  local ring io
  ring=$(cat "$ring_param" 2>/dev/null || echo absent)
  io=$(cat /proc/sys/kernel/io_uring_disabled 2>/dev/null || echo n/a)
  echo "host: kernel $(uname -r) $(uname -m), $(nproc) CPUs, $(awk '/MemTotal/{printf "%.1f GiB", $2/1048576}' /proc/meminfo), fuse.enable_uring=$ring, kernel.io_uring_disabled=$io, uid $(id -u)"
}

# Runs a gate with its output in $logs/NAME.log as well as on the terminal.
logged() {
  local name=$1; shift
  mkdir -p "$logs"
  local log=$logs/$name.log start rc
  start=$(date +%s)
  { echo "# ci-gate $name ($(date -u +%FT%TZ), $(git rev-parse --short HEAD 2>/dev/null || echo '?'))"; host_line; } | tee "$log"
  # errexit holds inside the gate (it would not on the left of an ||).
  set +e
  ( set -e; "$@" ) 2>&1 | tee -a "$log"
  rc=${PIPESTATUS[0]}
  set -e
  echo "ci-gate $name: exit $rc after $(( $(date +%s) - start ))s" | tee -a "$log"
  return "$rc"
}

corpus_section() {
  case "$1" in
    go|hooks) echo spike_repo;;
    node|python) echo "corpus.$1";;
    *) die "unknown corpus '$1' (go, node, python or hooks)";;
  esac
}
corpus_dir() { echo "${2:-${QUOD_SPIKE_DIR:-$root/target/corpus-$1}}"; }

# A full clone at the pinned commit, on a branch as acceptance.sh leaves it.
# For hooks, also the module cache the hooks test seeds jobs from
# (<checkout>/.quod/gomod: step A's output, made here with the pinned Go on
# the host, as an operator's seed is).
corpus() {
  local tc=$1 section url commit dir
  section=$(corpus_section "$tc")
  url=$(v "$section" url); commit=$(v "$section" commit)
  dir=$(corpus_dir "$tc" "${2:-}")
  if [ ! -d "$dir/.git" ]; then git clone -q "$url" "$dir"; fi
  git -C "$dir" cat-file -e "$commit^{commit}" 2>/dev/null || git -C "$dir" fetch -q origin "$commit"
  git -C "$dir" checkout -q -B quod-acceptance "$commit"
  grep -qsx '.quod/' "$dir/.git/info/exclude" || echo '.quod/' >> "$dir/.git/info/exclude"
  if [ "$tc" = hooks ] && [ ! -d "$dir/.quod/gomod/cache/download" ]; then
    (cd "$dir" && GOMODCACHE="$dir/.quod/gomod" GOFLAGS=-mod=mod GOTOOLCHAIN=local \
      GOPROXY=https://proxy.golang.org,direct /usr/local/go/bin/go mod download)
  fi
  echo "corpus $tc: $url@${commit:0:12} in $dir ($(du -sh "$dir" | cut -f1))"
}

# CI host preparation: what Dockerfile.dev bakes into the dev image, at the
# same paths (the sandbox sees only /usr, and the tests look for the
# toolchains under /usr/local), plus FUSE over io_uring when the kernel has it.
setup() {
  [ "${CI:-}" = true ] || [ "${QUOD_GATE_SETUP:-}" = 1 ] || die "setup changes the host; it runs in CI (or with QUOD_GATE_SETUP=1)"
  local s=; [ "$(id -u)" = 0 ] || s=sudo
  $s apt-get update -qq
  $s apt-get install -y -qq --no-install-recommends bubblewrap fuse3 sqlite3 pkg-config >/dev/null
  grep -qs '^user_allow_other' /etc/fuse.conf || echo user_allow_other | $s tee -a /etc/fuse.conf >/dev/null
  # Ubuntu restricts unprivileged user namespaces through AppArmor.
  $s sysctl -q -w kernel.apparmor_restrict_unprivileged_userns=0 2>/dev/null || true
  # The ring is off by default; on a kernel without CONFIG_FUSE_IO_URING the
  # parameter is absent and the observer uses the classic transport.
  if [ -e "$ring_param" ]; then
    echo Y | $s tee "$ring_param" >/dev/null 2>&1 || echo "ci-gate: could not enable FUSE io_uring"
  fi
  local arch triple narch
  case "$(uname -m)" in
    x86_64) arch=amd64; narch=x64; triple=x86_64-unknown-linux-gnu;;
    aarch64) arch=arm64; narch=arm64; triple=aarch64-unknown-linux-gnu;;
    *) die "unsupported arch $(uname -m)";;
  esac
  local tc
  for tc in "$@"; do
    case "$tc" in
      go)
        $s rm -rf /usr/local/go
        curl -fsSL "https://go.dev/dl/go$(v go version).linux-$arch.tar.gz" | $s tar -C /usr/local -xz
        /usr/local/go/bin/go version;;
      node)
        $s rm -rf /usr/local/node
        curl -fsSL "https://nodejs.org/dist/v$(v node version)/node-v$(v node version)-linux-$narch.tar.gz" | $s tar -C /usr/local -xz
        $s mv "/usr/local/node-v$(v node version)-linux-$narch" /usr/local/node
        echo "node $(/usr/local/node/bin/node --version)";;
      python)
        $s rm -rf /usr/local/python /usr/local/uv
        curl -fsSL "https://github.com/astral-sh/python-build-standalone/releases/download/$(v python pbs_release)/cpython-$(v python version)+$(v python pbs_release)-$triple-install_only_stripped.tar.gz" | $s tar -C /usr/local -xz
        $s mkdir -p /usr/local/uv/bin
        curl -fsSL "https://github.com/astral-sh/uv/releases/download/$(v uv version)/uv-$triple.tar.gz" | $s tar -C /usr/local/uv/bin -xz --strip-components=1
        /usr/local/python/bin/python3 --version; /usr/local/uv/bin/uv --version;;
      *) die "unknown toolchain '$tc'";;
    esac
  done
}

host() {
  host_line
  if [ "$(cat "$ring_param" 2>/dev/null)" = Y ]; then
    echo "transport: FUSE over io_uring available (the observer uses it as root, else classic /dev/fuse)"
  else
    echo "transport: classic /dev/fuse only (no ring: $(cat "$ring_param" 2>/dev/null || echo 'no enable_uring parameter'))"
  fi
  "$root/scripts/check-host.sh" || true
}

hooks() {
  local dir; dir=$(corpus_dir hooks "${1:-}")
  [ -d "$dir/.quod/gomod" ] || corpus hooks "$dir"
  QUOD_INTEGRATION=1 QUOD_SPIKE_REPO="$dir" QUOD_TOOLCHAIN=go \
    cargo test --release -p quod-cli --test hooks_acceptance -- --nocapture --test-threads=1
}

acceptance() {
  local tc=${1:?acceptance needs go, node or python} dir
  dir=$(corpus_dir "$tc" "${2:-}")
  [ -d "$dir/.git" ] || corpus "$tc" "$dir"
  # Jest's suite order and coverage timestamps, pytest's duration line: P1
  # cannot compare these corpora's step output byte for byte (acceptance.rs).
  case "$tc" in node|python) export QUOD_SKIP_P1=${QUOD_SKIP_P1:-1};; esac
  "$root/scripts/acceptance.sh" "$tc" "$dir"
}

# Markdown for one gate's log: the host, the transport the observer used,
# the criteria lines and the test totals.
summary() {
  local name=$1 log=$logs/$1.log
  echo "### $name"
  if [ ! -f "$log" ]; then echo; echo "no log ($log): the gate did not start"; return 0; fi
  echo
  echo '```text'
  grep -m1 '^host: ' "$log" || true
  local t
  t=$(grep -o '"transport":"[a-z-]*"' "$log" | sort | uniq -c | awk '{gsub(/"transport":|"/,"",$2); printf "%s%s x%s", sep, $2, $1; sep=", "}')
  local fb; fb=$(grep -c '"uring_fallback":true' "$log" || true)
  [ -z "$t" ] || echo "observer transport: $t (uring fallbacks: $fb)"
  grep -E '^(merge gate|ci-gate [a-z-]+: exit|lint (ok|FAILED)|== )|SKIPPED|^(step A|A[0-9]+b?|P[0-9]|W[0-9]|T[0-9]+|G[0-9]|S[0-9]|manifest:)[: ].*(ok|false hits|skipped|entries)' "$log" \
    | grep -v -e '^== ' -e ' event: ' | cut -c1-240 | head -80 || true
  grep -E '^test result:' "$log" | awk '
    { for (i = 1; i <= NF; i++) { if ($i ~ /^passed/) p += $(i-1); if ($i ~ /^failed/) f += $(i-1); if ($i ~ /^ignored/) g += $(i-1) } }
    END { if (NR) printf "tests: %d passed, %d failed, %d ignored (%d test binaries)\n", p, f, g, NR }'
  echo '```'
  if grep -qE '^test .* FAILED|panicked at|^---- ' "$log"; then
    echo
    echo '<details><summary>failures</summary>'
    echo
    echo '```text'
    grep -E '^test .* FAILED|panicked at|^---- ' "$log" | sort -u | cut -c1-240 | head -30
    echo '```'
    echo '</details>'
  fi
}

cmd=${1:-}; [ $# -gt 0 ] && shift
case "$cmd" in
  host) host;;
  setup) setup "$@";;
  corpus) corpus "${1:?corpus needs go, node, python or hooks}" "${2:-}";;
  lint) as_root lint "$@"; logged lint "$root/scripts/lint.sh";;
  merge-gate) as_root merge-gate "$@"; logged merge-gate "$root/scripts/merge-gate.sh";;
  hooks) as_root hooks "$@"; logged hooks hooks "$@";;
  acceptance) as_root acceptance "$@"; logged "acceptance-${1:-go}" acceptance "$@";;
  summary) summary "${1:?summary needs a gate name}";;
  *) sed -n '2,24p' "$0" >&2; exit 2;;
esac
