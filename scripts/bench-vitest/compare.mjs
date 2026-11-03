// Compare two vitest JSON reports per file: the file's status and every
// test's state. With quod's out.json, a difference in a file Quod replayed
// is a WRONG REPLAY; one in a file Quod ran (and recorded) means the
// recording changed the test's behaviour: also a failure, reported apart.
//
//   node compare.mjs plain.json quod.json [quod-out.json]
//
// Prints the differences, then one JSON line:
//   {files, differences, wrong: [file...], recording: [file...]}
// Exits 1 on any difference.
import fs from 'node:fs'

const [a, b, outPath] = process.argv.slice(2)
const load = (p) => {
  const m = new Map()
  for (const t of JSON.parse(fs.readFileSync(p, 'utf8')).testResults) {
    const tests = t.assertionResults.map((x) => `${x.fullName}=${x.status}`).sort()
    m.set(t.name, { status: t.status, tests })
  }
  return m
}
const A = load(a)
const B = load(b)
const ran = outPath ? new Set(JSON.parse(fs.readFileSync(outPath, 'utf8')).ran.map((r) => r.file)) : null
const wrong = []
const flaky = []
const note = (f, lines) => {
  for (const l of lines.slice(0, 12)) console.log(l)
  if (ran && ran.has(f)) flaky.push(f)
  else wrong.push(f)
}
for (const [f, x] of A) {
  const y = B.get(f)
  if (!y) {
    note(f, [`MISSING in quod: ${f}`])
    continue
  }
  if (x.status !== y.status || x.tests.join('\n') !== y.tests.join('\n')) {
    const lines = [`DIFF ${f}: plain=${x.status} quod=${y.status}`]
    const sa = new Set(x.tests)
    const sb = new Set(y.tests)
    for (const t of x.tests) if (!sb.has(t)) lines.push(`   - ${t}`)
    for (const t of y.tests) if (!sa.has(t)) lines.push(`   + ${t}`)
    note(f, lines)
  }
}
for (const f of B.keys()) if (!A.has(f)) note(f, [`EXTRA in quod: ${f}`])
console.log(JSON.stringify({ files: A.size, differences: wrong.length + flaky.length, wrong, recording: flaky, flaky }))
process.exit(wrong.length || flaky.length ? 1 : 0)
