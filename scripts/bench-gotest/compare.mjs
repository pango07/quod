// Compare a quod gotest run's `go test -json` stream with plain go test's,
// package by package and test by test.
//   node compare.mjs plain.out quod.out quod-summary.json <plain exit> <quod exit>
// Last line: {packages, wrong: [...], flaky: [...], out_diff: [...], plain_cached, plain_tested, plain_failed}.
// A difference in a package quod replayed is wrong; in one it ran, flaky
// (both ran it). out_diff: replayed packages whose output differs from
// plain's once durations, "(cached)" and line order are normalized out.
import fs from 'node:fs'
const [plainPath, quodPath, sumPath, prc, qrc] = process.argv.slice(2)
const parse = (p) => {
  const pkgs = new Map()
  const get = (name) => {
    if (!pkgs.has(name)) pkgs.set(name, { status: '', tests: new Map(), out: [] })
    return pkgs.get(name)
  }
  let text = ''
  try { text = fs.readFileSync(p, 'utf8') } catch {}
  for (const line of text.split('\n')) {
    if (!line.startsWith('{')) continue
    let e
    try { e = JSON.parse(line) } catch { continue }
    const name = e.Package ?? (e.ImportPath ? e.ImportPath.replace(/ \[(.*)\.test\]$/, (_, r) => `\u0000${r}`).split('\u0000').pop() : null)
    if (!name) continue
    const p = get(name)
    if (e.Output != null) p.out.push(e.Output)
    if (!['pass', 'fail', 'skip'].includes(e.Action)) continue
    if (e.Test) p.tests.set(e.Test, e.Action)
    else if (e.Package) p.status = e.Action
  }
  return pkgs
}
const norm = (lines) => lines.join('').split('\n')
  .map((l) => l.replace(/\(\d+(\.\d+)?s\)/g, '(T)').replace(/\t(\d+(\.\d+)?s|\(cached\))/g, '\tT'))
  .filter((l) => l !== '')
  .sort().join('\n')
// Test names with what varies run to run (hex addresses, RFC 3339 times,
// time.Date literals) normalized, as quod gotest matches them: used only when
// it maps both sides one to one.
const normName = (n) => n
  .replace(/0x[0-9a-fA-F]+/g, '0x?')
  .replace(/time\.Date\([^)]*\)/g, 'time.Date(?)')
  .replace(/\d{4}-\d{2}-\d{2}[T _]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?/g, '<time>')
const normTests = (m) => {
  const o = new Map()
  for (const [k, v] of m) o.set(normName(k), v)
  return o.size === m.size ? o : null
}
const a = parse(plainPath), b = parse(quodPath)
for (const [n, p] of a) {
  const q = b.get(n)
  if (!q) continue
  const np = normTests(p.tests), nq = normTests(q.tests)
  if (np && nq) { p.tests = np; q.tests = nq }
}
let sum = {}
try { sum = JSON.parse(fs.readFileSync(sumPath, 'utf8')) } catch {}
const ran = new Set(sum.ran_pkgs || [])
const wrong = [], flaky = [], outDiff = []
const names = new Set([...a.keys(), ...b.keys()])
for (const n of names) {
  const p = a.get(n), q = b.get(n)
  const replayed = !ran.has(n)
  const diff = []
  if (!p || !q) diff.push(p ? 'missing in quod' : 'missing in plain')
  else {
    if (p.status !== q.status) diff.push(`status ${p.status || '-'} vs ${q.status || '-'}`)
    const tn = new Set([...p.tests.keys(), ...q.tests.keys()])
    let td = 0
    for (const t of tn) if (p.tests.get(t) !== q.tests.get(t)) { td++; if (td <= 3) diff.push(`${t}: ${p.tests.get(t) || '-'} vs ${q.tests.get(t) || '-'}`) }
    if (td > 3) diff.push(`... ${td} tests differ`)
    if (!diff.length && replayed && norm(p.out) !== norm(q.out)) outDiff.push(n)
  }
  if (diff.length) {
    console.log(`${replayed ? 'WRONG (replayed)' : 'flaky (ran)'} ${n}: ${diff.join('; ')}`);
    (replayed ? wrong : flaky).push(n)
  }
}
if (+prc !== +qrc) { console.log(`WRONG exit code: plain ${prc}, quod ${qrc}`); wrong.push(`exit ${prc} vs ${qrc}`) }
for (const n of outDiff) console.log(`output differs (replayed, normalized) ${n}`)
const tested = [...a.values()].filter((p) => p.status === 'pass')
const cached = tested.filter((p) => p.out.some((o) => /\t\(cached\)/.test(o))).length
const failed = [...a.values()].filter((p) => p.status === 'fail').length
console.log(JSON.stringify({ packages: names.size, wrong, flaky, out_diff: outDiff, plain_cached: cached, plain_tested: tested.length, plain_failed: failed }))
