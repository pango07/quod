// Compare two vitest JSON reports per file: the file's status and every
// test's state. Prints each difference; exits 1 when they disagree.
//
//   node compare.mjs plain.json quod.json [labelA labelB]
import fs from 'node:fs'

const [a, b, labelA = 'plain', labelB = 'quod'] = process.argv.slice(2)
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
const diffs = []
for (const [f, x] of A) {
  const y = B.get(f)
  if (!y) {
    diffs.push(`MISSING in ${labelB}: ${f}`)
    continue
  }
  if (x.status !== y.status || x.tests.join('\n') !== y.tests.join('\n')) {
    const lines = [`DIFF ${f}: ${labelA}=${x.status} ${labelB}=${y.status}`]
    const sa = new Set(x.tests)
    const sb = new Set(y.tests)
    for (const t of x.tests) if (!sb.has(t)) lines.push(`   - ${t}`)
    for (const t of y.tests) if (!sa.has(t)) lines.push(`   + ${t}`)
    diffs.push(lines.slice(0, 12).join('\n'))
  }
}
for (const f of B.keys()) if (!A.has(f)) diffs.push(`EXTRA in ${labelB}: ${f}`)
for (const d of diffs) console.log(d)
console.log(JSON.stringify({ files: A.size, differences: diffs.length, differing: diffs.map((d) => d.split('\n')[0]) }))
process.exit(diffs.length ? 1 : 0)
