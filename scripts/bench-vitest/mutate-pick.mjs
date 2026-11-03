// Pick a mutation target for the bench's mutation check: a random function
// that some recorded test file executed, in a workspace source file (not a
// test file), spanning several lines. Prints "<path> <0-based start line>".
//   node mutate-pick.mjs <ledger.json> <seed>
import fs from 'node:fs'

const [ledgerPath, seedArg] = process.argv.slice(2)
const l = JSON.parse(fs.readFileSync(ledgerPath, 'utf8'))
let seed = Number(seedArg) || 1
const rand = (n) => {
  seed = (seed * 1103515245 + 12345) % 2147483648
  return seed % n
}
const options = []
for (const rec of Object.values(l.files)) {
  for (const [k, fns] of Object.entries(rec.cov ?? {})) {
    const key = l.keys[Number(k)]
    if (!key?.startsWith('f:') || /__tests__|\.spec\.|\.test\./.test(key) || key.includes('/node_modules/')) continue
    if (rec.pinned?.[k]) continue
    for (const [sl, , el] of fns) if (el - sl >= 2) options.push(`${key.slice(2)} ${sl}`)
  }
}
const uniq = [...new Set(options)].sort()
// No function-level data in the ledger (coverage did not map for this
// project): fall back to a block-bodied function declaration in a workspace
// source file some recorded test file depends on (run from the project root).
if (!uniq.length) {
  const deps = new Set()
  for (const rec of Object.values(l.files)) for (const k of rec.deps ?? []) deps.add(l.keys[k])
  for (const key of deps) {
    if (!key?.startsWith('f:') || !/\.[cm]?[tj]sx?$/.test(key) || /__tests__|\.spec\.|\.test\.|(^|\/)test\//.test(key) || key.includes('node_modules')) continue
    let lines
    try {
      lines = fs.readFileSync(key.slice(2), 'utf8').split('\n')
    } catch {
      continue
    }
    lines.forEach((t, n) => {
      if (/^\s*(export\s+)?(default\s+)?(async\s+)?function\s*\*?\s*\w*\s*(<[^>]*>)?\(.*\{\s*$/.test(t) && !/\}\s*$/.test(t.replace(/\{\s*$/, ''))) {
        if (lines.slice(n + 1, n + 4).some((x) => x.trim() && x.trim() !== '}')) uniq.push(`${key.slice(2)} ${n}`)
      }
    })
  }
  uniq.sort()
  if (uniq.length) console.error(`no function-level data; ${uniq.length} declared functions in dependencies`)
}
if (!uniq.length) {
  console.error('no mutation target')
  process.exit(1)
}
console.log(uniq[rand(uniq.length)])
