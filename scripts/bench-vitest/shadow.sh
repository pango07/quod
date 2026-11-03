#!/usr/bin/env bash
# The vitest benchmark, shadow-recording mode: recording off the critical path.
#
#   shadow.sh <project checkout> <sha> <n> <vitest args...>   (QUOD=path/to/quod)
#
# Per commit, in sequence (as CI pushes minutes apart would see it):
#   1. plain     `vitest` (the baseline)
#   2. critical  `quod vitest --no-record`: select against the most recent
#                COMPLETED ledger, replay what is unchanged, run the rest with
#                plain vitest (no hooks, no coverage, no audits, ledger untouched).
#                This is the developer's CI job: the HEADLINE number.
#   3. shadow    `quod vitest --audits $SHADOW_AUDITS`: the separate,
#                non-blocking job of the same commit. Re-selects against the
#                same ledger, runs the affected files with full recording,
#                audits replayed files (a mismatch quarantines), and writes
#                the ledger the next commit's critical path reads.
# plain and critical alternate their order per commit. Commit 0 (the window's
# parent) is the project's first run ever: no ledger, so the critical path is
# plain vitest under the driver, and the shadow records everything (plus the
# observer backstop, once).
#
# SHADOW_LAG=k: the critical path of commit i reads the ledger the shadow of
# commit i-1-k wrote (the k most recent shadows have not finished yet): the
# fallback to the most recent completed recording.
#
# Every critical run's JSON report is compared with plain's per file and per
# test (a difference in a replayed file is a wrong replay); so is every
# shadow's (its files ran; a difference there is flaky, or a recording bug).
# Mutation check on commits $MUTATE: a throw in a recorded function; the
# critical path must run the tests it breaks and report what plain reports.
# Writes $RUNNER_TEMP/bench-vitest/{results.jsonl,report.md,logs/}.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
proj=$(cd "$1" && pwd); sha=$2; n=$3; shift 3
args=("$@")
out=${RUNNER_TEMP:-/tmp}/bench-vitest
state=/work/state/quod-vitest        # the shadow's working ledger
snaps=/work/state/completed          # completed recordings, one per shadow (the cache entries)
lag=${SHADOW_LAG:-0}
audits=${SHADOW_AUDITS:-4}
rm -rf "$out" "$state" "$snaps"; mkdir -p "$out/logs" "$snaps" "$state"
: "${QUOD:?QUOD must point at the quod binary}"
cd "$proj"
mapfile -t commits < <(git rev-list --first-parent -n $((n + 1)) "$sha" | tac)
echo "window: ${#commits[@]} commits (first is the first run ever), ${commits[1]:0:9}..${commits[-1]:0:9}; lag $lag, shadow audits $audits"
# A monotonic clock (CLOCK_BOOTTIME via /proc/uptime, 10 ms resolution): the wall
# clock of a fresh hosted runner is stepped by time sync during the job, which
# made some runs come out 0.5-0.8 s shorter or longer than they were.
ms() { local up _; read -r up _ </proc/uptime; echo $(( 10#${up/./} * 10 )); }
lock=$(sha1sum pnpm-lock.yaml 2>/dev/null | cut -c1-40)

plain() { # $1 = tag
  local s; s=$(ms)
  node_modules/.bin/vitest "${args[@]}" --reporter=default --reporter=json --outputFile.json="$out/plain.json" >"$out/logs/plain-$1.log" 2>&1
  local rc=$?
  echo "$(( $(ms) - s )) $rc"
}
critical() { # $1 = tag, $2 = ledger dir (a completed recording; a fresh copy, so nothing leaks back)
  # The same vitest arguments as the shadow (the ledger keys them): one output path, renamed after.
  local s; s=$(ms)
  "$QUOD" vitest --no-record --dir "$2" --summary "$out/crit-summary.json" -- "${args[@]}" --reporter=default --reporter=json --outputFile.json="$out/quod.json" >"$out/logs/crit-$1.log" 2>&1
  local rc=$?; mv -f "$out/quod.json" "$out/crit.json"
  echo "$(( $(ms) - s )) $rc"
}
# Same-job A/B (QUOD_B=<another quod>, B_ENV="K=V ..." for its runs): B's
# critical path on its own fresh copy of the same recording, in the same
# job, plain/A/B in rotating order per commit; compared with plain as well.
critical_b() { # $1 = tag, $2 = ledger dir
  local s; s=$(ms)
  env ${B_ENV:-} "$QUOD_B" vitest --no-record --dir "$2" --summary "$out/crit-b-summary.json" -- "${args[@]}" --reporter=default --reporter=json --outputFile.json="$out/quod.json" >"$out/logs/crit-b-$1.log" 2>&1
  local rc=$?; mv -f "$out/quod.json" "$out/crit-b.json"
  echo "$(( $(ms) - s )) $rc"
}
shadow() { # $1 = tag
  local s; s=$(ms)
  env ${SHADOW_ENV:-} "$QUOD" vitest --dir "$state" --audits "$audits" --expect-report "$out/crit.json" --summary "$out/shadow-summary.json" -- "${args[@]}" --reporter=default --reporter=json --outputFile.json="$out/quod.json" >"$out/logs/shadow-$1.log" 2>&1
  local rc=$?; mv -f "$out/quod.json" "$out/shadow.json"
  echo "$(( $(ms) - s )) $rc"
}
# The ledger the critical path of commit $1 may read: the most recent shadow
# that completed before it (lag: the last $lag shadows are still running).
ledger_for() {
  local k=$(( $1 - 1 - lag ))
  local d=/work/state/crit-ledger
  rm -rf "$d"; mkdir -p "$d"
  while (( k >= 0 )); do
    if [ -d "$snaps/$k" ]; then cp -a "$snaps/$k/." "$d/"; echo "$d $k"; return; fi
    k=$(( k - 1 ))
  done
  echo "$d -1" # none yet: the first run ever
}

for i in "${!commits[@]}"; do
  c=${commits[$i]}
  git checkout -q --force "$c"
  nl=$(sha1sum pnpm-lock.yaml 2>/dev/null | cut -c1-40)
  install_ms=0
  read -r cdir from < <(ledger_for "$i")
  plan_ms=0; verdict=; pcmp='{}'
  if [ "${WHOLE_JOB:-0}" = 1 ]; then
    # The whole job: a fresh checkout has no node_modules. Quod's plan runs
    # before the install (the pnpm store is warm, as a CI cache restores it).
    find . -name node_modules -type d -prune -exec rm -rf {} +
    s=$(ms)
    "$QUOD" vitest --plan "$out/plan.txt" --dir "$cdir" --summary "$out/plan-summary.json" -- "${args[@]}" --reporter=default --reporter=json --outputFile.json="$out/quod.json" >"$out/logs/plan-$i.log" 2>&1
    plan_ms=$(( $(ms) - s ))
    verdict=$(cat "$out/plan.txt" 2>/dev/null)
    [ "$verdict" = done ] && mv -f "$out/quod.json" "$out/plan.json"
    rm -f "$out/quod.json"
    lock=
  fi
  if [ "$nl" != "$lock" ]; then
    s=$(ms)
    PUPPETEER_SKIP_DOWNLOAD=true pnpm install --frozen-lockfile --prefer-offline >"$out/logs/install-$i.log" 2>&1 || echo "install failed at $c"
    install_ms=$(( $(ms) - s ))
    lock=$nl
  fi
  b_ms=0; b_rc=0; bcmp='{}'
  if [ -n "${QUOD_B:-}" ]; then
    bdir=/work/state/crit-ledger-b; rm -rf "$bdir"; cp -a "$cdir" "$bdir"
    case $(( i % 3 )) in
      0) read -r p_ms p_rc < <(plain "$i"); read -r q_ms q_rc < <(critical "$i" "$cdir"); read -r b_ms b_rc < <(critical_b "$i" "$bdir") ;;
      1) read -r q_ms q_rc < <(critical "$i" "$cdir"); read -r b_ms b_rc < <(critical_b "$i" "$bdir"); read -r p_ms p_rc < <(plain "$i") ;;
      2) read -r b_ms b_rc < <(critical_b "$i" "$bdir"); read -r p_ms p_rc < <(plain "$i"); read -r q_ms q_rc < <(critical "$i" "$cdir") ;;
    esac
    bcmp=$(node "$here/compare.mjs" "$out/plain.json" "$out/crit-b.json" "$bdir/out.json" 2>&1 | tee "$out/logs/compare-b-$i.log" | tail -1)
  elif (( i % 2 == 0 )); then
    read -r p_ms p_rc < <(plain "$i"); read -r q_ms q_rc < <(critical "$i" "$cdir")
  else
    read -r q_ms q_rc < <(critical "$i" "$cdir"); read -r p_ms p_rc < <(plain "$i")
  fi
  cmp=$(node "$here/compare.mjs" "$out/plain.json" "$out/crit.json" "$cdir/out.json" 2>&1 | tee "$out/logs/compare-$i.log" | tail -1)
  if [ "$verdict" = done ]; then
    echo '{"ran":[]}' > "$out/plan-out.json"
    pcmp=$(node "$here/compare.mjs" "$out/plain.json" "$out/plan.json" "$out/plan-out.json" 2>&1 | tee "$out/logs/compare-plan-$i.log" | tail -1)
  fi
  # The shadow job of this commit (after the critical path: it never delays it).
  read -r sh_ms sh_rc < <(shadow "$i")
  obs_ms=0
  if (( i == 0 )) && [ "${OBSERVE:-1}" = 1 ]; then
    echo "::group::observer backstop (the first shadow)"
    s=$(ms)
    bash "$here/observe.sh" "$proj" "$state" "${args[@]}" 2>&1 | tee "$out/logs/observe.log" | tail -40
    obs_ms=$(( $(ms) - s ))
    echo "::endgroup::"
    cd "$proj"
  fi
  scmp=$(node "$here/compare.mjs" "$out/plain.json" "$out/shadow.json" "$state/out.json" 2>&1 | tee "$out/logs/compare-shadow-$i.log" | tail -1)
  # The shadow completed: its ledger is now a completed recording (the cache entry for this commit).
  mkdir -p "$snaps/$i"; cp -a "$state/." "$snaps/$i/"; rm -f "$snaps/$i/out.json" "$snaps/$i/plan.json"
  ledger_bytes=$(du -sb "$snaps/$i" | cut -f1)

  files=$(git diff --name-only "$c^" "$c" 2>/dev/null)
  touched=$(echo "$files" | grep -c . || true)
  areas=$(echo "$files" | awk -F/ 'NF{ if ($1=="packages"||$1=="packages-private") print $1"/"$2; else if (NF>1) print $1"/"; else print $1 }' | sort | uniq -c | sort -rn | awk '{printf "%s%s(%s)", sep, $2, $1; sep=", "}')
  subject=$(git log -1 --format=%s "$c")
  node -e '
    const [i, c, subject, touched, areas, p_ms, p_rc, q_ms, q_rc, sh_ms, sh_rc, obs_ms, from, install_ms, bytes, cmp, scmp, cs, ss, plan_ms, verdict, pcmp, b_ms, bcmp, bs] = process.argv.slice(1)
    const fs = require("fs")
    const j = (p) => { try { return JSON.parse(fs.readFileSync(p, "utf8")) } catch { return {} } }
    const k = (t) => { try { return JSON.parse(t) } catch { return { error: t } } }
    const s = j(cs), h = j(ss), a = k(cmp), b = k(scmp), pc = k(pcmp), bc = k(bcmp), bsum = j(bs)
    const ab = +b_ms ? { crit_b_ms: +b_ms, b_wrong: bc.wrong ?? null, b_ran: bsum.ran, b_fast: !!bsum.fast } : {}
    const whole = verdict ? { plan_ms: +plan_ms, plan: verdict, whole_plain_ms: +install_ms + +p_ms,
      whole_quod_ms: +plan_ms + (verdict === "done" ? 0 : +install_ms + +q_ms),
      plan_wrong: verdict === "done" ? (pc.wrong ?? null) : [] } : {}
    console.log(JSON.stringify({ i: +i, first: i === "0", commit: c, subject, touched: +touched, areas,
      plain_ms: +p_ms, plain_exit: +p_rc, crit_ms: +q_ms, crit_exit: +q_rc, ledger_from: +from,
      shadow_ms: +sh_ms, shadow_exit: +sh_rc, observe_ms: +obs_ms, install_ms: +install_ms, ledger_bytes: +bytes,
      ran: s.ran, replayed: s.replayed, narrowed: s.narrowed, shared_changed: s.shared_inputs_changed,
      shared_changed_keys: s.shared_changed_keys, select_ms: s.select_ms, vitest_ms: s.vitest_ms, driver: s.driver,
      shadow_ran: h.ran, shadow_replayed: h.replayed, audited: h.audited, audit_mismatches: h.audit_mismatches,
      unrecordable: h.unrecordable, record_ms: h.record_ms, shadow_driver: h.driver,
      files: a.files, wrong: a.wrong ?? null, flaky: a.flaky ?? null, compare_error: a.error,
      shadow_wrong: b.wrong ?? null, shadow_flaky: b.flaky ?? null, fast: !!s.fast, render: h.render ?? null, ...whole, ...ab }))
  ' "$i" "$c" "$subject" "$touched" "$areas" "$p_ms" "$p_rc" "$q_ms" "$q_rc" "$sh_ms" "$sh_rc" "$obs_ms" "$from" "$install_ms" "$ledger_bytes" "$cmp" "$scmp" "$out/crit-summary.json" "$out/shadow-summary.json" "$plan_ms" "$verdict" "$pcmp" "$b_ms" "$bcmp" "$out/crit-b-summary.json" | tee -a "$out/results.jsonl"

  # Mutation check (not timed): a throw in a function some recorded test
  # executed must make the critical path run that test and report the same
  # failures as plain vitest. The critical path never records, so restoring
  # the tree is all the clean-up there is.
  if [[ " ${MUTATE:-4 9 14 19} " == *" $i "* ]]; then
    cp "$snaps/$i/ledger.json" "$out/logs/ledger-$i.json"
    target=$(node "$here/mutate-pick.mjs" "$snaps/$i/ledger.json" "$i" 2>"$out/logs/mutate-pick-$i.log")
    [ -n "$target" ] || echo "mutation check at $i: no target ($(tail -1 "$out/logs/mutate-pick-$i.log"))"
    if [ -n "$target" ]; then
      mfile=${target% *}; mline=${target##* }
      what=$(node "$proj/node_modules/.quod-vitest/narrow.mjs" mutate "$mfile" "$mline" 2>&1)
      plain "m$i" >/dev/null
      read -r mdir _ < <(ledger_for $(( i + 1 + lag )))
      read -r mq_ms mq_rc < <(critical "m$i" "$mdir")
      mcmp=$(node "$here/compare.mjs" "$out/plain.json" "$out/crit.json" "$mdir/out.json" 2>&1 | tee "$out/logs/compare-m$i.log" | tail -1)
      node -e '
        const [i, target, what, rc, cmp, sumPath, plainPath] = process.argv.slice(1)
        const fs = require("fs")
        let s = {}; try { s = JSON.parse(fs.readFileSync(sumPath, "utf8")) } catch {}
        let k = {}; try { k = JSON.parse(cmp) } catch { k = { error: cmp } }
        let failedFiles = 0; try { failedFiles = JSON.parse(fs.readFileSync(plainPath, "utf8")).testResults.filter(t => t.status === "failed").length } catch {}
        console.log(JSON.stringify({ mutation: true, i: +i, target, what, quod_exit: +rc, ran: s.ran, replayed: s.replayed, narrowed: s.narrowed,
          plain_failed_files: failedFiles, wrong: k.wrong ?? null, flaky: k.flaky ?? null }))
      ' "$i" "$target" "$what" "$mq_rc" "$mcmp" "$out/crit-summary.json" "$out/plain.json" | tee -a "$out/results.jsonl"
      git checkout -q --force "$c"
    fi
  fi
done
node "$here/report-shadow.mjs" "$out/results.jsonl" "${args[*]}" "$(git remote get-url origin)" "$lag" "$audits" > "$out/report.md"
cat "$out/report.md"
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$out/report.md" >> "$GITHUB_STEP_SUMMARY"
# Loud: a wrong replay on the critical path (or a mutation it missed) fails the job.
if node -e 'const l=require("fs").readFileSync(process.argv[1],"utf8").trim().split("\n").map(JSON.parse);process.exit(l.some(r=>r.wrong===null||r.wrong.length||(r.b_wrong!==undefined&&(r.b_wrong===null||r.b_wrong.length))||(r.plan_wrong!==undefined&&(r.plan_wrong===null||r.plan_wrong.length)))?1:0)' "$out/results.jsonl"; then
  echo "0 wrong replays"
else
  echo "::error::WRONG REPLAYS (or an uncompared commit): see report.md"; exit 1
fi
