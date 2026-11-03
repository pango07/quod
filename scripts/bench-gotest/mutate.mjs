// Pick a function some test covers (from `go tool cover -func`) and put a
// panic at the top of its body. Prints "<file> <line>".
//   node mutate.mjs cover-func.txt <module path> <seed>
import fs from 'node:fs'
const [funcPath, mod, seed] = process.argv.slice(2)
const rows = fs.readFileSync(funcPath, 'utf8').split('\n')
  .map((l) => l.match(/^(\S+\.go):(\d+):\s+(\S+)\s+([\d.]+)%$/))
  .filter((m) => m && +m[4] > 0 && !m[1].endsWith('_test.go') && m[1].startsWith(`${mod}/`))
  .map((m) => ({ file: m[1].slice(mod.length + 1), line: +m[2], fn: m[3] }))
  .filter((r) => !/(^|\/)(testdata|internal\/testutil|examples?)\//.test(r.file) && r.fn !== 'init')
if (!rows.length) { console.error('no covered function'); process.exit(1) }
const start = (+seed * 7919) % rows.length
for (let k = 0; k < rows.length; k++) {
  const r = rows[(start + k) % rows.length]
  let src
  try { src = fs.readFileSync(r.file, 'utf8') } catch { continue }
  if (/^\/\/ Code generated .* DO NOT EDIT\.$/m.test(src)) continue
  const lines = src.split('\n')
  const decl = lines[r.line - 1]
  if (!decl || !/^func\b/.test(decl) || !decl.trimEnd().endsWith('{')) continue
  lines[r.line - 1] = `${decl.trimEnd()} panic("quod mutation check: ${r.fn}");`
  fs.writeFileSync(r.file, lines.join('\n'))
  console.error(`mutated ${r.fn} at ${r.file}:${r.line}`)
  console.log(`${r.file} ${r.line}`)
  process.exit(0)
}
console.error('no mutable function')
process.exit(1)
