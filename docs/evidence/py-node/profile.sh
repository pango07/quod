#!/usr/bin/env bash
# The per-job profile behind DECISIONS (finding 8), in the dev container:
#   scripts/dev.sh 'cargo build --release -p quod-cli && docs/evidence/py-node/profile.sh /src/target/release/quod'
set -eu
quod=$1
here=$(cd "$(dirname "$0")" && pwd)
py=/usr/local/python/bin/python3
w=${TMPDIR:-/tmp}/quod-profile
rm -rf "$w" && mkdir -p "$w/a" "$w/b"
echo "== python:3.12-slim, a stdlib import (finding 4) and the per-job cost"
"$py" "$here/drive.py" "$quod" "$w/a" python:3.12-slim 3 'python -c "import json, email, http.client; print(1)"' 2>&1 | cut -c1-300
echo
echo "== 5,000 files, ten steps, a new commit per job"
steps=()
for k in 0 1 2 3 4 5 6 7 8 9; do
  steps+=("python -c \"import json, email\" && head -c 12 pkg/m$k/f$k.py")
done
NFILES=5000 NEWCOMMIT=1 "$py" "$here/drive.py" "$quod" "$w/b" python:3.12-slim 3 "${steps[@]}" 2>&1 | cut -c1-120
echo
echo "== phases of the last job (events.jsonl)"
grep -h '"message":"\(rootfs\|checkout phases\|step phases\|observer\|hook\)"' "$w/b/state/events.jsonl" \
  | sed -e 's/"timestamp":"[^"]*",//' | cut -c1-240 | tail -25
