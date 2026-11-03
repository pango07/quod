// report.md from shadow.sh's results.jsonl: the critical path against plain
// (the headline), shadow compute as its own column.
//   node report-shadow.mjs results.jsonl "<vitest args>" <project url> <lag> <audits>
import fs from 'node:fs'

const [path, args, url, lag, audits] = process.argv.slice(2)
const all = fs.readFileSync(path, 'utf8').trim().split('\n').map((l) => JSON.parse(l))
const first = all.find((r) => r.first && !r.mutation)
const rows = all.filter((r) => !r.first && !r.mutation)
const muts = all.filter((r) => r.mutation)
const s = (ms) => (ms / 1000).toFixed(1)
const x = (a, b) => (b > 0 ? a / b : 0)
const median = (v) => {
  const a = [...v].sort((p, q) => p - q)
  const m = a.length >> 1
  return a.length ? (a.length % 2 ? a[m] : (a[m - 1] + a[m]) / 2) : 0
}
const sum = (v) => v.reduce((a, b) => a + b, 0)
const repo = (url || '').replace(/\.git$/, '')
const tp = sum(rows.map((r) => r.plain_ms))
const tc = sum(rows.map((r) => r.crit_ms))
const ts = sum(rows.map((r) => r.shadow_ms))
const wrong = rows.reduce((a, r) => a + (r.wrong ? r.wrong.length : 1), 0)
const flaky = rows.reduce((a, r) => a + (r.flaky ? r.flaky.length : 0), 0)
const swrong = rows.reduce((a, r) => a + (r.shadow_wrong ? r.shadow_wrong.length : 0), 0)
const mism = rows.reduce((a, r) => a + (r.audit_mismatches?.length ?? 0), 0)
const aud = rows.reduce((a, r) => a + (r.audited ?? 0), 0)
const full = [first, ...rows.filter((r) => r.replayed === 0)].filter(Boolean)
const over = full.map((r) => x(r.crit_ms - r.plain_ms, r.plain_ms) * 100)

const out = []
out.push(`## vitest replay, shadow recording: ${repo.split('/').slice(-2).join('/')}, ${rows.length} commits`)
out.push('')
out.push(`\`vitest ${args}\` on a GitHub-hosted runner. Per commit, in sequence: plain vitest; the **critical path** (\`quod vitest --no-record\`: replay what the last completed recording proves unchanged, run the rest with plain vitest, no recording); then the **shadow** (\`quod vitest\`, a separate non-blocking job: records the affected files, audits ${audits} replayed files, writes the ledger the next commit reads). ${+lag ? `Lag ${lag}: each critical path reads the recording of ${+lag + 1} commits back (the newer shadows are still running).` : 'Each critical path reads the previous commit\'s shadow.'} Every critical-path report compared with plain's, file by file and test by test.`)
out.push('')
out.push('| | plain | critical path | |')
out.push('|---|---:|---:|---:|')
out.push(`| **median per commit** | ${s(median(rows.map((r) => r.plain_ms)))} s | ${s(median(rows.map((r) => r.crit_ms)))} s | **${median(rows.map((r) => x(r.plain_ms, r.crit_ms))).toFixed(1)}x** (median of per-commit speedups) |`)
out.push(`| **total, ${rows.length} commits** | ${s(tp)} s | ${s(tc)} s | **${x(tp, tc).toFixed(1)}x** |`)
out.push(`| **wrong replays** | | **${wrong}** | ${flaky} flaky (a file both ran and disagreed) |`)
out.push(`| shadow compute (separate job, not on the critical path) | | ${s(ts)} s total | median ${s(median(rows.map((r) => r.shadow_ms)))} s per commit; ${aud} audits, ${mism} mismatches (quarantined) |`)
out.push('')
if (first) {
  out.push(`First run ever (\`${first.commit.slice(0, 9)}\`, no recording yet): plain ${s(first.plain_ms)} s, critical path ${s(first.crit_ms)} s (${first.ran} files run plain); the shadow recorded all ${first.shadow_ran} files in ${s(first.shadow_ms)} s${first.observe_ms ? `, plus the observer backstop ${s(first.observe_ms)} s` : ''}.`)
  out.push('')
}
if (full.length) out.push(`The critical path's own cost where every file ran (${full.length} runs: the first run, shared inputs changed): median ${median(over).toFixed(1)}% over plain, worst ${Math.max(...over).toFixed(1)}%.`)
out.push('')
out.push('| # | commit | what it touched | plain | critical path | speedup | files run / replayed (function-level) | ledger from | shadow compute | shadow ran / audited | wrong |')
out.push('|---:|---|---|---:|---:|---:|---|---:|---:|---|---:|')
for (const r of [first, ...rows].filter(Boolean)) {
  const subj = r.subject.length > 50 ? `${r.subject.slice(0, 47)}...` : r.subject
  const link = repo ? `[${r.commit.slice(0, 9)}](${repo}/commit/${r.commit})` : r.commit.slice(0, 9)
  const why = r.shared_changed && r.replayed === 0 && r.ledger_from >= 0 ? ' (shared inputs)' : ''
  const from = r.ledger_from < 0 ? 'none' : `#${r.ledger_from}`
  out.push(
    `| ${r.i}${r.first ? ' (first run)' : ''} | ${link} ${subj.replace(/\|/g, '\\|')} | ${r.touched} files: ${r.areas || '-'} | ${s(r.plain_ms)} s | **${s(r.crit_ms)} s** | ${x(r.plain_ms, r.crit_ms).toFixed(1)}x | ${r.ran ?? '?'} / ${r.replayed ?? '?'}${r.narrowed ? ` (${r.narrowed})` : ''}${why} | ${from} | ${s(r.shadow_ms)} s | ${r.shadow_ran ?? '?'} / ${r.audited ?? 0}${r.audit_mismatches?.length ? ` (${r.audit_mismatches.length} mismatch)` : ''} | ${r.wrong ? r.wrong.length : 'not compared'} |`,
  )
}
out.push('')
const q = rows.filter((r) => r.select_ms != null)
if (q.length) out.push(`Critical path, Quod's own time: select median ${median(q.map((r) => r.select_ms)).toFixed(0)} ms; vitest start-up in the driver median ${median(q.map((r) => r.driver?.init ?? 0)).toFixed(0)} ms. Ledger (the hand-off) median ${(median(rows.map((r) => r.ledger_bytes ?? 0)) / 1e6).toFixed(1)} MB.`)
if (swrong) out.push(`Shadow runs that disagreed with plain in a file they replayed: ${swrong} (they report nothing to the developer; listed for the recorder).`)
const installs = rows.filter((r) => r.install_ms > 0)
if (installs.length) out.push(`Dependencies reinstalled (lockfile changed; not timed, same for all): commits ${installs.map((r) => r.i).join(', ')}.`)
if (muts.length) {
  out.push('')
  out.push('Mutation check (not timed): a `throw` put into a function a recorded test executed; the critical path must run the tests it breaks and report what plain vitest reports.')
  out.push('')
  out.push('| after # | mutated function | plain: failing files | critical: files run / replayed | exit | wrong |')
  out.push('|---:|---|---:|---|---:|---:|')
  for (const m of muts) out.push(`| ${m.i} | \`${m.target}\` | ${m.plain_failed_files} | ${m.ran} / ${m.replayed} | ${m.quod_exit} | ${m.wrong ? m.wrong.length : 'not compared'} |`)
}
const keys = {}
for (const r of rows) for (const k of r.shared_changed_keys ?? []) keys[k] = (keys[k] ?? 0) + 1
if (Object.keys(keys).length) out.push(`\nShared inputs that changed (commits): ${Object.entries(keys).map(([k, n]) => `\`${k}\` (${n})`).join(', ')}.`)
for (const r of all) if (r.wrong?.length) out.push(`\n**WRONG REPLAY at ${r.commit?.slice(0, 9) ?? `mutation #${r.i}`}**: ${r.wrong.join(', ')}`)
console.log(out.join('\n'))
