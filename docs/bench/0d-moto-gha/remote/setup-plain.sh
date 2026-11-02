#!/usr/bin/env bash
# Provisions server 4 as the plain runner host: docker.io (as on server 1),
# the job image pre-pulled, actions/runner 2.338.2 (checksum from Quod's
# versions.toml), registered with label `plain` (token on stdin), as a service.
set -euo pipefail
read -r token
repo=pango07/quod-demo-moto
export DEBIAN_FRONTEND=noninteractive
apt-get update -q >/dev/null
apt-get install -y -q docker.io curl >/dev/null
systemctl enable --now docker >/dev/null 2>&1
docker version --format 'docker {{.Server.Version}}'
docker pull -q ghcr.io/astral-sh/uv:0.13.6-python3.12-trixie@sha256:101820b96a31d3438aab226230bc5516d62c6b5b506f0e2d72cb4b6975e27dea
ver=2.338.2
sha=baebf204307bac6193480852732f0aba2ae97b6c5597c244f4d5558390147c40
dir=/opt/gha-plain
mkdir -p $dir
curl -fsSL -o /tmp/runner.tgz https://github.com/actions/runner/releases/download/v$ver/actions-runner-linux-x64-$ver.tar.gz
echo "$sha  /tmp/runner.tgz" | sha256sum -c --quiet -
tar -xzf /tmp/runner.tgz -C $dir && rm /tmp/runner.tgz
$dir/bin/installdependencies.sh >/dev/null 2>&1 || true
echo RUNNER_ALLOW_RUNASROOT=1 > $dir/.env
cd $dir
RUNNER_ALLOW_RUNASROOT=1 ./config.sh --unattended --replace --disableupdate \
  --url "https://github.com/$repo" --token "$token" --name hetzner4-plain --labels plain --work _work >/dev/null
cat > /etc/systemd/system/gha-plain.service <<EOF
[Unit]
Description=GitHub Actions runner (plain) for $repo
After=network-online.target docker.service

[Service]
WorkingDirectory=$dir
ExecStart=$dir/run.sh
Environment=RUNNER_ALLOW_RUNASROOT=1
KillMode=mixed
TimeoutStopSec=5min
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now gha-plain >/dev/null 2>&1
sleep 5; systemctl is-active gha-plain
nproc; free -g | head -2; uname -r
