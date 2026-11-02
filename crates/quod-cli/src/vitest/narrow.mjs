// quod vitest: function-level narrowing, and the bench's mutations.
//
//   node narrow.mjs <request.json> <answer.json>
//   node narrow.mjs mutate <file> <line>       (bench only)
//
// A test file whose recorded inputs changed only in workspace source files
// it reached as modules may still replay, if for every such file every
// changed region lies strictly inside the body of a function the test
// file never executed (when it was recorded). Everything else runs:
//   - a change outside every function body (top-level statements, imports,
//     exports, class fields and static blocks, decorators, module-scope
//     declarations, types), or touching a body's braces, a parameter list
//     or a signature
//   - a change inside a function the test executed, or inside one nested in
//     a function it executed whose line range is ambiguous
//   - a function added or removed outside a body (outside every body)
//   - a new version that does not parse, or that V8 rejects (an early
//     error anywhere fails the whole module)
//   - no typescript to parse with, a parse error in the old version
// The request carries, per test file and changed source file, the old
// version (the one the test file was recorded on, kept by Quod) and the
// executed functions' line ranges (0-based, from V8 coverage through the
// transformed code's source map).
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'

const t0 = performance.now()

function loadTs(root) {
  try {
    return createRequire(path.join(root, 'package.json'))('typescript')
  } catch {
    return null
  }
}

function kindOf(ts, file) {
  const ext = path.extname(file)
  return (
    { '.ts': ts.ScriptKind.TS, '.mts': ts.ScriptKind.TS, '.cts': ts.ScriptKind.TS, '.tsx': ts.ScriptKind.TSX, '.jsx': ts.ScriptKind.JSX }[ext] ??
    ts.ScriptKind.JS
  )
}

// Hunks of a line diff, in old-file line indexes: [a0, a1) replaced by new
// lines; a pure insertion has a0 === a1 (before old line a0).
export function hunks(a, b) {
  let pre = 0
  while (pre < a.length && pre < b.length && a[pre] === b[pre]) pre++
  let suf = 0
  while (suf < a.length - pre && suf < b.length - pre && a[a.length - 1 - suf] === b[b.length - 1 - suf]) suf++
  const A = a.slice(pre, a.length - suf)
  const B = b.slice(pre, b.length - suf)
  if (!A.length && !B.length) return []
  const n = A.length
  const m = B.length
  const max = n + m
  if (max > 20000) return [[pre, pre + n]]
  // Myers, keeping each round's frontier for the path back.
  const off = max + 1
  let v = new Int32Array(2 * max + 3)
  const trace = []
  let found = -1
  for (let d = 0; d <= max; d++) {
    trace.push(v.slice())
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1
      let y = x - k
      while (x < n && y < m && A[x] === B[y]) {
        x++
        y++
      }
      v[off + k] = x
      if (x >= n && y >= m) {
        found = d
        break
      }
    }
    if (found >= 0) break
    if (d > 400) return [[pre, pre + n]]
  }
  // Walk back: the matched (old, new) line pairs.
  const pairs = []
  let x = n
  let y = m
  for (let d = found; d > 0; d--) {
    const vd = trace[d]
    const k = x - y
    const prevK = k === -d || (k !== d && vd[off + k - 1] < vd[off + k + 1]) ? k + 1 : k - 1
    const px = vd[off + prevK]
    const py = px - prevK
    while (x > px && y > py) {
      pairs.push([x - 1, y - 1])
      x--
      y--
    }
    x = px
    y = py
  }
  while (x > 0 && y > 0) {
    pairs.push([x - 1, y - 1])
    x--
    y--
  }
  pairs.reverse()
  pairs.push([n, m])
  // Between consecutive matches: removed old lines [pa+1, a), or an
  // insertion before old line a.
  const out = []
  let pa = -1
  let pb = -1
  for (const [ai, bj] of pairs) {
    if (ai > pa + 1 || bj > pb + 1) out.push([pre + pa + 1, pre + ai])
    pa = ai
    pb = bj
  }
  return out
}

// Every function with a block body: its line range and its body's.
function bodies(ts, sf) {
  const out = []
  const line = (pos) => sf.getLineAndCharacterOfPosition(pos).line
  const visit = (node) => {
    if (ts.isFunctionLike(node) && node.body && ts.isBlock(node.body)) {
      out.push({
        fn: [line(node.getStart(sf)), line(node.end - 1)],
        open: line(node.body.getStart(sf)),
        close: line(node.body.end - 1),
      })
    }
    ts.forEachChild(node, visit)
  }
  visit(sf)
  return out
}

function compiles(ts, file, text) {
  try {
    const t = ts.transpileModule(text, {
      fileName: file,
      reportDiagnostics: true,
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ESNext, jsx: ts.JsxEmit.Preserve },
    })
    if (t.diagnostics?.length) return false
    new vm.Script(`(function (exports, require, module, __filename, __dirname) {${t.outputText}\n})`, { filename: file })
    return true
  } catch {
    return false
  }
}

// For one changed file (old version -> current): the line ranges of the
// functions whose bodies hold every change, or null (a change outside).
function regions(ts, file, oldText, newText) {
  const sfOld = ts.createSourceFile(file, oldText, ts.ScriptTarget.Latest, true, kindOf(ts, file))
  const sfNew = ts.createSourceFile(file, newText, ts.ScriptTarget.Latest, true, kindOf(ts, file))
  if (sfOld.parseDiagnostics?.length || sfNew.parseDiagnostics?.length) return { why: 'does not parse' }
  if (!compiles(ts, file, newText)) return { why: 'new version does not compile' }
  const bs = bodies(ts, sfOld)
  const out = []
  for (const [a0, a1] of hunks(oldText.split('\n'), newText.split('\n'))) {
    let best = null
    for (const b of bs) {
      const inside = a0 === a1 ? b.open < a0 && a0 <= b.close : b.open < a0 && a1 - 1 < b.close
      if (inside && (!best || b.close - b.open < best.close - best.open)) best = b
    }
    if (!best) return { why: 'a change outside every function body' }
    out.push(best.fn)
  }
  return { fns: out }
}

// May a test file that executed `ranges` skip changes confined to `fns`?
function untouched(fns, ranges) {
  for (const [ns, ne] of fns) {
    for (const [sl, , el] of ranges) {
      const disjoint = el < ns || sl > ne
      const strictlyContains = sl <= ns && el >= ne && !(sl === ns && el === ne)
      if (!disjoint && !strictlyContains) return false
    }
  }
  return true
}

function narrow(req) {
  const ts = loadTs(req.root)
  const why = {}
  const note = (w) => (why[w] = (why[w] ?? 0) + 1)
  const skip = []
  if (!ts) {
    note('no typescript to parse with')
    return { skip, why }
  }
  const cache = new Map()
  const current = new Map()
  for (const t of req.tests) {
    let ok = true
    for (const c of t.checks) {
      const k = `${c.file}\0${c.old}`
      let r = cache.get(k)
      if (!r) {
        try {
          if (!current.has(c.file)) current.set(c.file, fs.readFileSync(c.file, 'utf8'))
          r = regions(ts, c.file, fs.readFileSync(c.old, 'utf8'), current.get(c.file))
        } catch (e) {
          r = { why: `unreadable (${e.code ?? e.message})` }
        }
        cache.set(k, r)
      }
      if (!r.fns) {
        note(r.why)
        ok = false
        break
      }
      if (!untouched(r.fns, c.ranges)) {
        note('changes a function it executed')
        ok = false
        break
      }
    }
    if (ok) {
      skip.push(t.id)
      note('changes only functions it never ran')
    }
  }
  return { skip, why }
}

if (process.argv[2] === 'mutate') {
  // Insert a throw as the first statement of the innermost function with a
  // block body that starts on <line> (0-based), for the bench's mutation
  // check. Prints the function's line range.
  const [file, lineArg] = process.argv.slice(3)
  const ts = loadTs(process.cwd())
  const text = fs.readFileSync(file, 'utf8')
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, kindOf(ts, file))
  const want = Number(lineArg)
  let best = null
  const visit = (node) => {
    if (ts.isFunctionLike(node) && node.body && ts.isBlock(node.body)) {
      const s = sf.getLineAndCharacterOfPosition(node.getStart(sf)).line
      if (s === want && (!best || node.end - node.pos < best.end - best.pos)) best = node
    }
    ts.forEachChild(node, visit)
  }
  visit(sf)
  if (!best) {
    console.log('no function starts on that line')
    process.exit(1)
  }
  const at = best.body.getStart(sf) + 1
  fs.writeFileSync(file, `${text.slice(0, at)} throw new Error('quod-mutation'); ${text.slice(at)}`)
  console.log(`mutated ${file}:${want + 1} (${sf.getLineAndCharacterOfPosition(best.end).line - want + 1} lines)`)
} else if (process.argv[1]?.endsWith('narrow.mjs')) {
  const [reqPath, outPath] = process.argv.slice(2)
  const ans = narrow(JSON.parse(fs.readFileSync(reqPath, 'utf8')))
  ans.ms = performance.now() - t0
  fs.writeFileSync(outPath, JSON.stringify(ans))
}
