#!/usr/bin/env bash
# Host changes for 0e, after install.sh (0b's, unchanged) and before any
# measured run. The host fingerprint hashes /etc, so nothing here may run
# once measurement starts. /etc/resolv.conf is left as Ubuntu's symlink
# (main binds what it points to; 0b's F1 workaround is not applied).
set -euxo pipefail
# SSH: keys only.
sed -i -E 's/^#?\s*PasswordAuthentication .*/PasswordAuthentication no/; s/^#?\s*KbdInteractiveAuthentication .*/KbdInteractiveAuthentication no/' /etc/ssh/sshd_config
grep -q '^PasswordAuthentication no' /etc/ssh/sshd_config || echo 'PasswordAuthentication no' >> /etc/ssh/sshd_config
grep -q '^KbdInteractiveAuthentication no' /etc/ssh/sshd_config || echo 'KbdInteractiveAuthentication no' >> /etc/ssh/sshd_config
for f in /etc/ssh/sshd_config.d/*.conf; do [ -e "$f" ] && sed -i -E 's/^\s*PasswordAuthentication .*/PasswordAuthentication no/' "$f"; done
systemctl reload ssh || systemctl reload sshd
# Firewall: SSH only.
command -v ufw >/dev/null || apt-get install -y -q ufw
ufw default deny incoming
ufw default allow outgoing
ufw allow OpenSSH
ufw --force enable
# The plain baseline's user (Quod runs steps as uid 1001; cli/cli's
# permission tests fail as root) and git's ownership check for the shared
# source clone.
id runner >/dev/null 2>&1 || useradd -m -u 1001 -U runner
git config --system safe.directory '*'
# FUSE over io_uring.
echo Y > /sys/module/fuse/parameters/enable_uring
# Nothing scheduled may rewrite /etc (trust-v2 saw apt-daily-upgrade do it
# mid-run) or compete for the 4 vCPUs while measuring.
systemctl disable --now unattended-upgrades.service 2>/dev/null || true
for t in apt-daily apt-daily-upgrade update-notifier-download update-notifier-motd motd-news \
         man-db dpkg-db-backup logrotate sysstat-collect sysstat-summary sysstat-rotate \
         e2scrub_all xfs_scrub_all fstrim systemd-tmpfiles-clean; do
  systemctl disable --now "$t.timer" 2>/dev/null || true
  systemctl mask "$t.timer" 2>/dev/null || true
done
systemctl list-timers --all --no-pager
cat /sys/module/fuse/parameters/enable_uring
ls -la /etc/resolv.conf
echo SETUP_DONE
