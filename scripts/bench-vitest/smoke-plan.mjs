// Smoke only: a plan that replays every recorded passing file of out.json
// (no selection), auditing the first two. Exercises the driver alone, and
// prints what the record looks like.
import fs from 'node:fs'

const [out, state, plan] = process.argv.slice(2)
const o = JSON.parse(fs.readFileSync(out, 'utf8'))
const replay = {}
for (const f of o.ran) {
  if ((f.state === 'pass' || f.state === 'skip') && f.keys && !f.unobservable.length) replay[f.id] = f.result
}
fs.writeFileSync(plan, JSON.stringify({ stateDir: state, replay, audit: Object.keys(replay).slice(0, 2), expect: o.facts }))
const count = (xs) => xs.reduce((a, x) => ((a[x] = (a[x] || 0) + 1), a), {})
const sizes = o.ran.map((f) => f.keys?.length ?? -1).sort((a, b) => a - b)
console.log(
  JSON.stringify(
    {
      ran: o.ran.length,
      replayable: Object.keys(replay).length,
      states: count(o.ran.map((f) => f.state)),
      unobservable: count(o.ran.flatMap((f) => f.unobservable)),
      norecord: o.ran.filter((f) => !f.keys).map((f) => f.file).slice(0, 5),
      keysMedian: sizes[sizes.length >> 1],
      keysMax: sizes.at(-1),
      global: o.global.length,
      globalKinds: count(o.global.map((k) => k.split(':')[0])),
      timings: o.timings,
    },
    null,
    1,
  ),
)
console.log('global (not packages):\n' + o.global.filter((k) => !k.startsWith('p:')).slice(0, 80).join('\n'))
const one = o.ran.find((f) => f.keys)
if (one) console.log(`one file (${one.file}):\n` + one.keys.filter((k) => !k.startsWith('p:')).slice(0, 40).join('\n'))
