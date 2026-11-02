#!/usr/bin/env bash
# Installs the freshly built quod and the shim from /opt/quod-gha/src-main, with
# the default policy in replay mode, then wipes all benchmark state (cold start).
set -euo pipefail
systemctl stop gha-plain gha-quod
install -m 0644 /opt/quod-gha/src-main/scripts/runner/hooks/index.js /opt/quod-gha/hooks/index.js
install -m 0755 /opt/quod-gha/target/release/quod /opt/quod-gha/bin/quod
cat > /opt/quod-gha/hooks/quod.json <<'EOF'
{"bin": "/opt/quod-gha/bin/quod", "state": "/var/lib/quod-gha", "policy": {"mode": "replay"}}
EOF
cat /opt/quod-gha/COMMIT
bash /opt/quod-gha/bin/reset-state.sh
