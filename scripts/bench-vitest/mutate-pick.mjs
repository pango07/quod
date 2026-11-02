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
if (!uniq.length) process.exit(1)
console.log(uniq[rand(uniq.length)])
