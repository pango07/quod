#!/usr/bin/env bash
# The observer backstop for `quod vitest`'s recorder: run the suite once
# under Quod's FUSE observer (`quod run`, as root), take the manifest of
# everything any process read, and fold what the recorder's hooks did not
# account for into the ledger's shared inputs (`quod vitest --absorb`).
#
#   observe.sh <project checkout> <ledger dir> <vitest args...>   (QUOD=...)
set -uo pipefail
proj=$(cd "$1" && pwd); dir=$2; shift 2
qstate=/work/state/quod-observer
mkdir -p "$qstate"
vars=()
for n in $(compgen -e); do
  case "$n" in PATH|HOME|CI|LANG|TMPDIR|QUOD_*|GITHUB_*|RUNNER_*|PNPM_*|NODE_*) vars+=("$n=${!n}");; esac
done
script="node_modules/.bin/vitest $(printf "%q " "$@")"
s=$(date +%s%3N)
sudo env "${vars[@]}" "$QUOD" --state "$qstate" run --repo "$proj" --toolchain node --timeout 20m --step "$script" >"$qstate/run.log" 2>&1
rc=$?
echo "observed run: exit $rc in $(( $(date +%s%3N) - s )) ms"
grep -E 'QUOD|Test Files|Tests |inputs|Error' "$qstate/run.log" | head -20
sudo chown -R "$(id -u):$(id -g)" "$qstate" "$proj" 2>/dev/null || true
mid=$(grep -h '"manifest_id"' "$qstate"/events*.jsonl 2>/dev/null | tail -1 | sed -E 's/.*"manifest_id":"([^"]+)".*/\1/')
if [ -z "$mid" ]; then echo "no manifest recorded"; tail -30 "$qstate/run.log"; exit 1; fi
"$QUOD" --state "$qstate" inspect "$mid" | sed 1d > "$qstate/manifest.json"
node -e 'const m=require(process.argv[1]);const c={};for(const e of m.entries)c[e.k]=(c[e.k]||0)+1;console.log("manifest", m.entries.length, JSON.stringify(c), "roots", JSON.stringify(m.roots));console.log(m.entries.filter(e=>e.k==="f").slice(0,8).map(e=>e.p).join("\n"))' "$qstate/manifest.json"
(cd "$proj" && "$QUOD" vitest --dir "$dir" --absorb "$qstate/manifest.json")
