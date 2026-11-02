#!/bin/sh
# quod-server's container entrypoint (Dockerfile.server). A Railway volume
# is mounted root-owned over QUOD_DATA_DIR, so the image starts as root only
# to hand that directory to the unprivileged `quod` user (uid 10001), then
# runs the server as that user with no way back (no-new-privs). Started as
# any other user, it runs the server directly.
set -eu
data="${QUOD_DATA_DIR:-/data}"
if [ "$(id -u)" = 0 ]; then
  mkdir -p "$data"
  if [ "$(stat -c %u "$data")" != 10001 ]; then
    chown -R 10001:10001 "$data"
  fi
  chmod 0700 "$data"
  exec setpriv --reuid=10001 --regid=10001 --init-groups --no-new-privs -- quod-server "$@"
fi
exec quod-server "$@"
