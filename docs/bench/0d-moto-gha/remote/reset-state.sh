#!/usr/bin/env bash
# Fresh start for the benchmark, keeping both runner registrations: stops the
# runners, drops Quod's hooks state and both runners' work dirs (workspaces,
# tool caches), starts the runners again.
set -u
systemctl stop gha-plain gha-quod
grep ' /var/lib/quod-gha/' /proc/mounts | awk '{print $2}' | sort -r | xargs -r -n1 umount -l 2>/dev/null
rm -rf /var/lib/quod-gha /opt/gha-plain/_work /opt/gha-quod/_work
docker ps -aq | xargs -r docker rm -f >/dev/null
systemctl start gha-plain gha-quod
sleep 3
systemctl is-active gha-plain gha-quod | tr '\n' ' '
df -h / | tail -1
