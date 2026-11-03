#!/usr/bin/env bash
# The go test benchmark, shadow-recording mode (the vitest shadow bench's
# method, scripts/bench-vitest/shadow.sh, for `quod gotest`).
#
#   shadow.sh <project checkout> <sha> <n> <go test args...>   (QUOD=path/to/quod)
#
# Per commit, in sequence (as CI pushes minutes apart would see it):
#   1. plain     `go test` (the baseline), with its own GOCACHE kept warm
#                across commits (a perfect actions/cache) and the shared
#                module cache: go's own test cache is as warm as CI can make it.
#   2. critical  `quod gotest --no-record`: select against the most recent
#                COMPLETED ledger, run the affected packages with plain go
#                test, replay the rest. The developer's job: the HEADLINE.
#   3. shadow    `quod gotest --audits $SHADOW_AUDITS --expect-report`: the
#                separate, non-blocking job of the same commit; records the
#                affected packages under strace and writes the next ledger.
# plain and critical alternate their order per commit. Commit 0 (the window's
# parent) is the project's first run ever (no ledger) and is not in the
# headline. Every checkout gets fresh mtimes (`touch` of every tracked file),
# as a fresh CI checkout has: go's test cache keys the files a test opened
# by mtime and size, so this is what plain gets in CI.
#
# Every critical run's -json stream is compared with plain's per package and
# per test (a difference in a replayed package is a wrong replay), and its
# exit code with plain's. Mutation check on commits $MUTATE: a panic at the
# top of a function some test covers; the critical path must run that
# package and fail as plain does.
# Writes $RUNNER_TEMP/bench-gotest/{results.jsonl,report.md,logs/}.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
proj=$(cd "$1" && pwd); sha=$2; n=$3; shift 3
args=("$@")
out=${RUNNER_TEMP:-/tmp}/bench-gotest
work=${BENCH_WORK:-/work/state}
state=$work/quod-gotest       # the shadow's working ledger
snaps=$work/completed         # completed recordings, one per shadow (the cache entries)
lag=${SHADOW_LAG:-0}
audits=${SHADOW_AUDITS:-4}
rm -rf "$out" "$state" "$snaps"; mkdir -p "$out/logs" "$snaps" "$state"
: "${QUOD:?QUOD must point at the quod binary}"
# Every flag this script passes must exist in the binary under test: an
# older quod would take an unknown flag as a go test argument. Fail loudly.
need_flags() { # binary, flags...
  local bin=$1 help; shift
  help=$("$bin" gotest --help 2>&1) || { echo "::error::$bin gotest --help failed"; exit 2; }
  for f in "$@"; do
    grep -q -- "$f" <<<"$help" || { echo "::error::$bin does not support quod gotest $f (this bench passes it); use a newer binary"; exit 2; }
  done
}
need_flags "$QUOD" --dir --audits --summary --no-record --expect-report --report
export GOMODCACHE=$work/gomodcache GOTOOLCHAIN=${GOTOOLCHAIN:-local}
cd "$proj"
mapfile -t commits < <(git rev-list --first-parent -n $((n + 1)) "$sha" | tac)
echo "window: ${#commits[@]} commits (first is the first run ever), ${commits[1]:0:9}..${commits[-1]:0:9}; lag $lag, shadow audits $audits; go $(go env GOVERSION); go test ${args[*]}"
# A monotonic clock (CLOCK_BOOTTIME via /proc/uptime, 10 ms resolution): the
# hosted runner's wall clock is stepped by time sync during the job.
ms() { local up _; read -r up _ </proc/uptime; echo $(( 10#${up/./} * 10 )); }
fresh() { # the tree as a fresh checkout of $1 has it
  git checkout -q --force "$1"; git clean -qffdx
  git ls-files -z | xargs -0 touch -c
}

plain() { # $1 = tag
  local s; s=$(ms)
  GOCACHE=$work/gocache-plain go test "${args[@]}" >"$out/plain.out" 2>"$out/logs/plain-$1.err"
  local rc=$?
  echo "$(( $(ms) - s )) $rc"
}
critical() { # $1 = tag, $2 = ledger dir (a fresh copy of a completed recording)
  rm -f "$out/crit-summary.json" "$out/crit-report.json"; local s; s=$(ms)
  GOCACHE=$work/gocache-crit "$QUOD" gotest --no-record --dir "$2" --summary "$out/crit-summary.json" --report "$out/crit-report.json" -- "${args[@]}" >"$out/crit.out" 2>"$out/logs/crit-$1.err"
  local rc=$?
  echo "$(( $(ms) - s )) $rc"
}
shadow() { # $1 = tag
  rm -f "$out/shadow-summary.json"; local s; s=$(ms)
  GOCACHE=$work/gocache-shadow env ${SHADOW_ENV:-} "$QUOD" gotest --dir "$state" --audits "$audits" --expect-report "$out/crit-report.json" --summary "$out/shadow-summary.json" -- "${args[@]}" >"$out/shadow.out" 2>"$out/logs/shadow-$1.err"
  local rc=$?
  echo "$(( $(ms) - s )) $rc"
}
# The ledger the critical path of commit $1 may read: the most recent shadow
# that completed before it (lag: the last $lag shadows are still running).
ledger_for() {
  local k=$(( $1 - 1 - lag ))
  local d=$work/crit-ledger
  rm -rf "$d"; mkdir -p "$d"
  while (( k >= 0 )); do
    if [ -d "$snaps/$k" ]; then cp -a "$snaps/$k/." "$d/"; echo "$d $k"; return; fi
    k=$(( k - 1 ))
  done
  echo "$d -1" # none yet: the first run ever
}

# Warm every series' caches at the window's parent (not timed): the module
# cache, and each GOCACHE with the compiled packages (a CI cache restored).
fresh "${commits[0]}"
go mod download >/dev/null 2>&1 || true
for c in plain crit shadow; do GOCACHE=$work/gocache-$c go test -count=1 -run '^$' ./... >/dev/null 2>&1 || true; done

for i in "${!commits[@]}"; do
  c=${commits[$i]}
  fresh "$c"
  read -r cdir from < <(ledger_for "$i")
  if (( i % 2 == 0 )); then
    read -r p_ms p_rc < <(plain "$i"); read -r q_ms q_rc < <(critical "$i" "$cdir")
  else
    read -r q_ms q_rc < <(critical "$i" "$cdir"); read -r p_ms p_rc < <(plain "$i")
  fi
  cmp=$(node "$here/compare.mjs" "$out/plain.out" "$out/crit.out" "$out/crit-summary.json" "$p_rc" "$q_rc" 2>&1 | tee "$out/logs/compare-$i.log" | tail -1)
  # The shadow job of this commit (after the critical path: it never delays it).
  fresh "$c"
  read -r sh_ms sh_rc < <(shadow "$i")
  scmp=$(node "$here/compare.mjs" "$out/plain.out" "$out/shadow.out" "$out/shadow-summary.json" "$p_rc" "$sh_rc" 2>&1 | tee "$out/logs/compare-shadow-$i.log" | tail -1)
  tail -3 "$out/logs/shadow-$i.err"
  # The shadow completed: its ledger is now a completed recording.
  mkdir -p "$snaps/$i"; cp -a "$state/." "$snaps/$i/"
  ledger_bytes=$(du -sb "$snaps/$i" | cut -f1)

  files=$(git diff --name-only "$c^" "$c" 2>/dev/null)
  touched=$(echo "$files" | grep -c . || true)
  areas=$(echo "$files" | awk -F/ 'NF{ if (NF>1) print $1"/"; else print $1 }' | sort | uniq -c | sort -rn | awk '{printf "%s%s(%s)", sep, $2, $1; sep=", "}')
  subject=$(git log -1 --format=%s "$c")
  node -e '
    const [i, c, subject, touched, areas, p_ms, p_rc, q_ms, q_rc, sh_ms, sh_rc, from, bytes, cmp, scmp, cs, ss] = process.argv.slice(1)
    const fs = require("fs")
    const j = (p) => { try { return JSON.parse(fs.readFileSync(p, "utf8")) } catch { return {} } }
    const k = (t) => { try { return JSON.parse(t) } catch { return { error: t } } }
    const s = j(cs), h = j(ss), a = k(cmp), b = k(scmp)
    console.log(JSON.stringify({ i: +i, first: i === "0", commit: c, subject, touched: +touched, areas,
      plain_ms: +p_ms, plain_exit: +p_rc, crit_ms: +q_ms, crit_exit: +q_rc, ledger_from: +from,
      shadow_ms: +sh_ms, shadow_exit: +sh_rc, ledger_bytes: +bytes,
      packages: s.packages, ran: s.ran, replayed: s.replayed, narrowed: s.narrowed, fast_world: s.fast_world, reasons: s.reasons, select_ms: s.select_ms, go_ms: s.go_ms,
      crit_go_cached: s.go_cached, plain_cached: a.plain_cached, plain_tested: a.plain_tested,
      shadow_ran: h.ran, audited: h.audited, recorded: h.recorded, unrecordable: h.unrecordable,
      behaviour_changed: h.behaviour_changed, audit_mismatches: h.audit_mismatches, record_ms: h.record_ms,
      compared: a.packages, wrong: a.wrong ?? null, flaky: a.flaky ?? null, out_diff: a.out_diff ?? null, compare_error: a.error,
      shadow_flaky: b.flaky ?? null, shadow_broken: h.recorded === undefined }))
  ' "$i" "$c" "$subject" "$touched" "$areas" "$p_ms" "$p_rc" "$q_ms" "$q_rc" "$sh_ms" "$sh_rc" "$from" "$ledger_bytes" "$cmp" "$scmp" "$out/crit-summary.json" "$out/shadow-summary.json" | tee -a "$out/results.jsonl"

  # Mutation check (not timed): a panic at the top of a function some test
  # covers must make the critical path run its package and report what plain
  # reports. The critical path never records: restoring the tree is all.
  if [[ " ${MUTATE:-4 9 14 19} " == *" $i "* ]]; then
    GOCACHE=$work/gocache-mut go test -count=1 -coverprofile="$out/cover.out" ./... >"$out/logs/cover-$i.log" 2>&1
    GOCACHE=$work/gocache-mut go tool cover -func="$out/cover.out" >"$out/cover-func.txt" 2>>"$out/logs/cover-$i.log"
    target=$(node "$here/mutate.mjs" "$out/cover-func.txt" "$(go list -m -f '{{.Path}}')" "$i" 2>"$out/logs/mutate-$i.log")
    if [ -z "$target" ]; then
      echo "mutation check at $i: no target ($(tail -1 "$out/logs/mutate-$i.log"))"
    else
      echo "mutation check at $i: $target"
      read -r _ mp_rc < <(plain "m$i")
      # The newest completed recording (this commit's shadow).
      read -r mdir _ < <(ledger_for $(( i + 1 + lag )))
      read -r mq_ms mq_rc < <(critical "m$i" "$mdir")
      mcmp=$(node "$here/compare.mjs" "$out/plain.out" "$out/crit.out" "$out/crit-summary.json" "$mp_rc" "$mq_rc" 2>&1 | tee "$out/logs/compare-m$i.log" | tail -1)
      node -e '
        const [i, target, prc, qrc, cmp, sumPath] = process.argv.slice(1)
        const fs = require("fs")
        let s = {}; try { s = JSON.parse(fs.readFileSync(sumPath, "utf8")) } catch {}
        let k = {}; try { k = JSON.parse(cmp) } catch { k = { error: cmp } }
        console.log(JSON.stringify({ mutation: true, i: +i, target, plain_exit: +prc, quod_exit: +qrc, ran: s.ran, replayed: s.replayed,
          plain_failed: k.plain_failed, wrong: k.wrong ?? null, flaky: k.flaky ?? null }))
      ' "$i" "$target" "$mp_rc" "$mq_rc" "$mcmp" "$out/crit-summary.json" | tee -a "$out/results.jsonl"
    fi
    fresh "$c"
  fi
done
node "$here/report.mjs" "$out/results.jsonl" "${args[*]}" "$(git remote get-url origin)" "$lag" "$audits" > "$out/report.md"
cat "$out/report.md"
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$out/report.md" >> "$GITHUB_STEP_SUMMARY"
# Loud: a wrong replay on the critical path (or a mutation it missed) fails the job.
if node -e 'const l=require("fs").readFileSync(process.argv[1],"utf8").trim().split("\n").map(JSON.parse);process.exit(l.some(r=>r.wrong===null||r.wrong.length||(r.mutation&&!(r.plain_failed>0)))?1:0)' "$out/results.jsonl"; then
  echo "0 wrong replays"
  node -e 'const l=require("fs").readFileSync(process.argv[1],"utf8").trim().split("\n").map(JSON.parse).filter(r=>r.shadow_broken&&!r.mutation);if(l.length)console.log("::warning::the shadow recorded nothing on commits "+l.map(r=>r.i).join(", "))' "$out/results.jsonl"
else
  echo "::error::WRONG REPLAYS (or an uncompared commit, or a mutation plain did not catch): see report.md"; exit 1
fi
