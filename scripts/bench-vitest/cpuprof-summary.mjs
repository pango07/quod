// Summarize .cpuprofile files: self time by package (or node internal), top N.
//   node cpuprof-summary.mjs <dir> [top]
import fs from 'node:fs'
import path from 'node:path'
const [dir, topArg] = process.argv.slice(2)
const top = +(topArg ?? 18)
for (const f of fs.readdirSync(dir).filter((x) => x.endsWith('.cpuprofile')).sort()) {
  const p = JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8'))
  const byId = new Map(p.nodes.map((n) => [n.id, n]))
  const self = new Map()
  const dt = p.timeDeltas
  for (let i = 0; i < p.samples.length; i++) {
    const n = byId.get(p.samples[i])
    self.set(n, (self.get(n) ?? 0) + (dt[i] ?? 0))
  }
  const cat = new Map()
  const fn = new Map()
  let total = 0
  for (const [n, us] of self) {
    total += us
    const url = n.callFrame.url || `(${n.callFrame.functionName || 'program'})`
    let c
    const m = url.match(/node_modules\/(?:\.pnpm\/[^/]+\/node_modules\/)?((?:@[^/]+\/)?[^/]+)/)
    if (m) c = m[1]
    else if (url.startsWith('node:')) c = url
    else if (url.startsWith('(')) c = url
    else c = 'project:' + url.split('/').slice(-2).join('/')
    cat.set(c, (cat.get(c) ?? 0) + us)
    const k = `${c} ${n.callFrame.functionName || '(anon)'}:${n.callFrame.lineNumber}`
    fn.set(k, (fn.get(k) ?? 0) + us)
  }
  const ms = (us) => (us / 1000).toFixed(0)
  console.log(`== ${f}: ${ms(p.endTime - p.startTime)} ms wall, ${ms(total)} ms sampled`)
  for (const [c, us] of [...cat].sort((a, b) => b[1] - a[1]).slice(0, top)) console.log(`  ${ms(us).padStart(6)} ms  ${c}`)
  console.log('  top functions:')
  for (const [c, us] of [...fn].sort((a, b) => b[1] - a[1]).slice(0, 10)) console.log(`  ${ms(us).padStart(6)} ms  ${c}`)
}
