// report.md from shadow.sh's results.jsonl: the critical path against plain
// go test (the headline), shadow compute as its own column.
//   node report.mjs results.jsonl "<go test args>" <project url> <lag> <audits>
import fs from 'node:fs'
const [path, args, url, lag, audits] = process.argv.slice(2)
const all = fs.readFileSync(path, 'utf8').trim().split('\n').map((l) => JSON.parse(l))
const first = all.find((r) => r.first && !r.mutation)
const rows = all.filter((r) => !r.first && !r.mutation)
const muts = all.filter((r) => r.mutation)
const s = (ms) => (ms / 1000).toFixed(1)
const x = (a, b) => a / Math.max(b, 10) // the clock's resolution is 10 ms
const median = (v) => {
  const a = [...v].sort((p, q) => p - q)
  const m = a.length >> 1
  return a.length ? (a.length % 2 ? a[m] : (a[m - 1] + a[m]) / 2) : 0
}
const sum = (v) => v.reduce((a, b) => a + (b || 0), 0)
const repo = (url || '').replace(/\.git$/, '')
const tp = sum(rows.map((r) => r.plain_ms))
const tc = sum(rows.map((r) => r.crit_ms))
const ts = sum(rows.map((r) => r.shadow_ms))
const wrong = rows.reduce((a, r) => a + (r.wrong ? r.wrong.length : 1), 0)
const flaky = rows.reduce((a, r) => a + (r.flaky ? r.flaky.length : 0), 0)
const outDiff = rows.reduce((a, r) => a + (r.out_diff ? r.out_diff.length : 0), 0)
const mism = rows.reduce((a, r) => a + Object.keys(r.audit_mismatches || {}).length, 0)
const changed = rows.reduce((a, r) => a + Object.keys(r.behaviour_changed || {}).length, 0)
const aud = sum(rows.map((r) => r.audited))
const pc = sum(rows.map((r) => r.plain_cached)), pt = sum(rows.map((r) => r.plain_tested))
const ran = sum(rows.map((r) => r.ran)), rep = sum(rows.map((r) => r.replayed))
const out = []
out.push(`## go test replay, shadow recording: ${repo.split('/').slice(-2).join('/')}, ${rows.length} commits`)
out.push('')
out.push(`\`go test ${args}\` on a GitHub-hosted runner. Per commit, in sequence: plain go test (its own GOCACHE warm across commits, fresh-checkout mtimes); the **critical path** (\`quod gotest --no-record\`: replay the packages the last completed recording proves unchanged, run the rest with plain go test); then the **shadow** (\`quod gotest\`, a separate non-blocking job: records the affected packages under strace, audits ${audits} replayed packages, records a package only if it ran as on the critical path). ${+lag ? `Lag ${lag}.` : 'Each critical path reads the previous commit\'s shadow.'} Every critical-path -json stream compared with plain's, package by package and test by test.`)
out.push('')
out.push('| | plain | critical path | |')
out.push('|---|---:|---:|---:|')
out.push(`| **median per commit** | ${s(median(rows.map((r) => r.plain_ms)))} s | ${s(median(rows.map((r) => r.crit_ms)))} s | **${median(rows.map((r) => x(r.plain_ms, r.crit_ms))).toFixed(2)}x** (median of per-commit speedups) |`)
out.push(`| **total, ${rows.length} commits** | ${s(tp)} s | ${s(tc)} s | **${x(tp, tc).toFixed(2)}x** |`)
out.push(`| **wrong replays** | | **${wrong}** | ${flaky} flaky (a package both ran and disagreed); ${outDiff} replayed packages with a normalized-output difference |`)
out.push(`| packages | go's own test cache hit ${pc} of ${pt} passing packages (${pt ? Math.round((100 * pc) / pt) : 0}%) | ${ran} run, ${rep} replayed | |`)
out.push(`| shadow compute (separate job) | | ${s(ts)} s total | median ${s(median(rows.map((r) => r.shadow_ms)))} s; ${aud} audits, ${mism} mismatches, ${changed} recordings that changed behaviour (quarantined) |`)
out.push('')
if (first) {
  out.push(`First run ever (\`${first.commit.slice(0, 9)}\`, no recording): plain ${s(first.plain_ms)} s, critical path ${s(first.crit_ms)} s (${first.ran} packages run plain); the shadow recorded ${first.recorded ?? '?'} of ${first.shadow_ran ?? '?'} in ${s(first.shadow_ms)} s.`)
  out.push('')
}
if (muts.length) {
  out.push(`Mutation checks (a panic at the top of a covered function, against the newest recording): ${muts.map((m) => `#${m.i} ${m.target}: plain failed ${m.plain_failed} packages, quod ran ${m.ran}, replayed ${m.replayed}, wrong ${m.wrong ? m.wrong.length : 'not compared'}`).join('; ')}.`)
  out.push('')
}
const unrec = {}
for (const r of [first, ...rows].filter(Boolean)) for (const [p, w] of Object.entries(r.unrecordable || {})) unrec[p] = w
if (Object.keys(unrec).length) {
  out.push(`Unrecordable (always run): ${Object.entries(unrec).map(([p, w]) => `\`${p}\` (${w.slice(0, 120)})`).join('; ')}.`)
  out.push('')
}
out.push('| # | commit | touched | plain (go cached) | critical path | speedup | run / replayed | why run | shadow | recorded / audited | wrong |')
out.push('|---:|---|---|---:|---:|---:|---|---|---:|---|---:|')
for (const r of [first, ...rows].filter(Boolean)) {
  const subj = r.subject.length > 44 ? `${r.subject.slice(0, 41)}...` : r.subject
  const link = repo ? `[${r.commit.slice(0, 9)}](${repo}/commit/${r.commit})` : r.commit.slice(0, 9)
  const why = {}
  for (const w of Object.values(r.reasons || {})) { const k = w.replace(/^f:.*|^[sdl]:.*/, 'an input').replace(/ changed$/, '').slice(0, 40); why[k] = (why[k] || 0) + 1 }
  const whys = Object.entries(why).map(([k, n]) => `${k} (${n})`).join(', ')
  out.push(
    `| ${r.i}${r.first ? ' (first)' : ''} | ${link} ${subj.replace(/\|/g, '\\|')} | ${r.touched}: ${(r.areas || '-').slice(0, 40)} | ${s(r.plain_ms)} s (${r.plain_cached ?? '?'}/${r.plain_tested ?? '?'}) | **${s(r.crit_ms)} s** | ${x(r.plain_ms, r.crit_ms).toFixed(2)}x | ${r.ran ?? '?'} / ${r.replayed ?? '?'} | ${whys || '-'} | ${s(r.shadow_ms)} s | ${r.recorded ?? '?'} / ${r.audited ?? 0} | ${r.wrong ? r.wrong.length : 'not compared'} |`,
  )
}
console.log(out.join('\n'))
