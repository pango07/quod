// report.md from bench.sh's results.jsonl.
//   node report.mjs results.jsonl "<vitest args>" <project url>
import fs from 'node:fs'

const [path, args, url] = process.argv.slice(2)
const all = fs.readFileSync(path, 'utf8').trim().split('\n').map((l) => JSON.parse(l))
const warm = all.find((r) => r.warmup)
const rows = all.filter((r) => !r.warmup && !r.mutation)
const muts = all.filter((r) => r.mutation)
const s = (ms) => (ms / 1000).toFixed(1)
const x = (a, b) => (b > 0 ? a / b : 0)
const median = (v) => {
  const a = [...v].sort((p, q) => p - q)
  const m = a.length >> 1
  return a.length % 2 ? a[m] : (a[m - 1] + a[m]) / 2
}
const repo = (url || '').replace(/\.git$/, '')
const tp = rows.reduce((a, r) => a + r.plain_ms, 0)
const tq = rows.reduce((a, r) => a + r.quod_ms, 0)
const speed = rows.map((r) => x(r.plain_ms, r.quod_ms))
const wrong = rows.reduce((a, r) => a + (r.wrong ? r.wrong.length : 1), 0)
const flaky = rows.reduce((a, r) => a + (r.flaky ? r.flaky.length : 0), 0)
const total = (r) => (r.ran ?? 0) + (r.replayed ?? 0)
const fullRuns = rows.filter((r) => r.replayed === 0)
const overhead = fullRuns.map((r) => x(r.quod_ms - r.plain_ms, r.plain_ms) * 100)

const out = []
out.push(`## vitest test-file replay: ${repo.split('/').slice(-2).join('/')}, ${rows.length} commits`)
out.push('')
out.push(`\`vitest ${args}\` on a GitHub-hosted runner, plain vs \`quod vitest\`, interleaved per commit, same checkout and dependencies. Every commit's two JSON reports compared file by file and test by test.`)
out.push('')
out.push('| | plain | Quod | |')
out.push('|---|---:|---:|---:|')
out.push(`| **median per commit** | ${s(median(rows.map((r) => r.plain_ms)))} s | ${s(median(rows.map((r) => r.quod_ms)))} s | **${median(speed).toFixed(1)}x** (median of per-commit speedups) |`)
out.push(`| **total, ${rows.length} commits** | ${s(tp)} s | ${s(tq)} s | **${x(tp, tq).toFixed(1)}x** |`)
out.push(`| **wrong replays** | | **${wrong}** | ${flaky} flaky (a test file both ran and disagreed) |`)
out.push('')
if (fullRuns.length) {
  out.push(`Commits where every file had to run (shared inputs changed): ${fullRuns.length}; Quod's cost over plain there: median ${median(overhead).toFixed(1)}%, worst ${Math.max(...overhead).toFixed(1)}%.`)
  out.push('')
}
const best = [...rows].filter((r) => r.ran > 0 && r.touched > 0).sort((a, b) => x(b.plain_ms, b.quod_ms) - x(a.plain_ms, a.quod_ms))[0]
if (best) {
  out.push(`Best case: \`${best.commit.slice(0, 9)}\` ${best.subject} (${best.touched} file${best.touched === 1 ? '' : 's'} changed: ${best.areas}): plain ${s(best.plain_ms)} s, Quod ${s(best.quod_ms)} s (${best.ran} of ${total(best)} test files ran).`)
  out.push('')
}
out.push('| # | commit | what it touched | plain | Quod | speedup | files run / replayed (function-level) | audits | wrong |')
out.push('|---:|---|---|---:|---:|---:|---|---:|---:|')
for (const r of rows) {
  const subj = r.subject.length > 60 ? `${r.subject.slice(0, 57)}...` : r.subject
  const link = repo ? `[${r.commit.slice(0, 9)}](${repo}/commit/${r.commit})` : r.commit.slice(0, 9)
  const why = r.shared_changed && r.replayed === 0 ? ' (shared inputs)' : ''
  out.push(
    `| ${r.i} | ${link} ${subj.replace(/\|/g, '\\|')} | ${r.touched} files: ${r.areas || '-'} | ${s(r.plain_ms)} s | ${s(r.quod_ms)} s | ${x(r.plain_ms, r.quod_ms).toFixed(1)}x | ${r.ran ?? '?'} / ${r.replayed ?? '?'}${r.narrowed ? ` (${r.narrowed})` : ''}${why} | ${r.audited ?? 0}${r.audit_mismatches?.length ? ` (${r.audit_mismatches.length} mismatch)` : ''} | ${r.wrong ? r.wrong.length : 'not compared'} |`,
  )
}
out.push('')
const q = rows.filter((r) => r.select_ms != null)
if (q.length) {
  out.push(`Quod's own time per commit (outside vitest): select median ${median(q.map((r) => r.select_ms)).toFixed(0)} ms, record median ${median(q.map((r) => r.record_ms)).toFixed(0)} ms. vitest start-up in the driver: median ${median(q.map((r) => r.driver?.init ?? 0)).toFixed(0)} ms.`)
}
if (warm) out.push(`Warm-up (the window's parent, not counted; Quod records its ledger there as the previous CI run would have): plain ${s(warm.plain_ms)} s, Quod ${s(warm.quod_ms)} s.`)
const unrec = {}
for (const r of rows) for (const [k, v] of Object.entries(r.unrecordable ?? {})) unrec[k] = Math.max(unrec[k] ?? 0, v)
if (Object.keys(unrec).length) out.push(`Files that always run (the recorder cannot follow them): ${Object.entries(unrec).map(([k, v]) => `${v} (${k})`).join(', ')}.`)
const installs = rows.filter((r) => r.install_ms > 0)
if (installs.length) out.push(`Dependencies reinstalled (lockfile changed; not timed, same for both): commits ${installs.map((r) => r.i).join(', ')}.`)
if (muts.length) {
  out.push('')
  out.push('Mutation check (not timed): a `throw` put into a function a recorded test executed; Quod must run the tests it breaks and report what plain vitest reports.')
  out.push('')
  out.push('| after # | mutated function | plain: failing files | Quod: files run / replayed | Quod exit | wrong |')
  out.push('|---:|---|---:|---|---:|---:|')
  for (const m of muts) out.push(`| ${m.i} | \`${m.target}\` | ${m.plain_failed_files} | ${m.ran} / ${m.replayed} | ${m.quod_exit} | ${m.wrong ? m.wrong.length : 'not compared'} |`)
}
const keys = {}
for (const r of rows) for (const k of r.shared_changed_keys ?? []) keys[k] = (keys[k] ?? 0) + 1
if (Object.keys(keys).length) out.push(`\nShared inputs that changed (commits): ${Object.entries(keys).map(([k, n]) => `\`${k}\` (${n})`).join(', ')}.`)
for (const r of rows) {
  if (r.wrong?.length) out.push(`\n**WRONG REPLAY at ${r.commit.slice(0, 9)}**: ${r.wrong.join(', ')}`)
}
console.log(out.join('\n'))
