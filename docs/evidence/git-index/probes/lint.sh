#!/usr/bin/env bash
# scripts/lint.sh under the shared heavy lock, logged to $1.
cd /src
log=${1:-/work/tmp/gidx-lint.log}
flock /work/tmp/heavy.lock scripts/lint.sh > "$log" 2>&1
echo "exit $?" >> "$log"
tail -5 "$log"
