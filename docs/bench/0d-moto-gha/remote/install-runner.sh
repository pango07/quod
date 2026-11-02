#!/usr/bin/env bash
# Installs one official actions/runner for the demo repo.
#   install-runner.sh plain|quod [owner/repo]   (registration token on stdin, never on disk)
# plain: /opt/gha-plain, label `plain`, jobs in docker (the runner's own container support).
# quod:  /opt/gha-quod, label `quod`, Quod's container hooks as scripts/runner/install.sh
#        installs them, with the shim and its quod.json under /opt/quod-gha/hooks
#        (install.sh hard-codes /opt/quod, which on this box is another bench's checkout).
set -euo pipefail
kind=$1
read -r token
repo=${2:-pango07/quod-demo-moto}
src=/opt/quod-gha/src
v() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$src/versions.toml"; }
ver=$(v actions_runner version)
dir=/opt/gha-$kind
tarball=actions-runner-linux-x64-$ver.tar.gz
if [ ! -x "$dir/run.sh" ]; then
  mkdir -p "$dir"
  [ -f "/opt/quod-gha/$tarball" ] || curl -fsSL -o "/opt/quod-gha/$tarball" "https://github.com/actions/runner/releases/download/v$ver/$tarball"
  echo "$(v actions_runner sha256_x64)  /opt/quod-gha/$tarball" | sha256sum -c --quiet -
  tar -xzf "/opt/quod-gha/$tarball" -C "$dir"
fi
if [ "$kind" = quod ]; then
  mkdir -p /opt/quod-gha/hooks /opt/quod-gha/bin
  install -m 0644 "$src/scripts/runner/hooks/index.js" /opt/quod-gha/hooks/index.js
  install -m 0755 /opt/quod-gha/target/release/quod /opt/quod-gha/bin/quod
  # The policy: replay mode; a result replays after one recorded execution
  # (proofs 1); the default 5% audit; the default network rules plus `uv sync`.
  cat > /opt/quod-gha/hooks/quod.json <<'EOF'
{"bin": "/opt/quod-gha/bin/quod", "state": "/var/lib/quod-gha", "policy": {"mode": "replay", "proofs": 1, "audit": 0.05, "net_steps": ["go mod download", "go get ", "go install ", "npm ci", "npm install", "yarn install", "pnpm install", "pip install", "cargo fetch", "apt-get ", "apk add", "curl ", "wget ", "uv sync"]}}
EOF
  cat > "$dir/.env" <<'EOF'
ACTIONS_RUNNER_CONTAINER_HOOKS=/opt/quod-gha/hooks/index.js
ACTIONS_RUNNER_REQUIRE_JOB_CONTAINER=true
RUNNER_ALLOW_RUNASROOT=1
EOF
else
  cat > "$dir/.env" <<'EOF'
RUNNER_ALLOW_RUNASROOT=1
EOF
fi
cd "$dir"
if [ ! -f .runner ]; then
  RUNNER_ALLOW_RUNASROOT=1 ./config.sh --unattended --replace --disableupdate \
    --url "https://github.com/$repo" --token "$token" \
    --name "hetzner1-$kind" --labels "$kind" --work _work >/dev/null
fi
cat > /etc/systemd/system/gha-$kind.service <<EOF
[Unit]
Description=GitHub Actions runner ($kind) for $repo
After=network-online.target docker.service

[Service]
WorkingDirectory=$dir
ExecStart=$dir/run.sh
Environment=RUNNER_ALLOW_RUNASROOT=1
Environment=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
KillMode=mixed
KillSignal=SIGTERM
TimeoutStopSec=5min
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now gha-$kind >/dev/null 2>&1
sleep 3
systemctl is-active gha-$kind
