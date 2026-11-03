// quod vitest: function-level recording by instrumentation, in vitest's
// main process (record runs only).
//
// Each workspace source module's transformed code (what vite hands the
// workers) gets, at its start,
//   const __qh=(globalThis.__quod_hits||(n=>new Uint8Array(n)))(N,"<file>");
// and, as the first statement of every function body (after its
// directives), `__qh[i]=1;` (an expression-bodied arrow becomes
// `(__qh[i]=1,<expr>)`). The worker's setup file owns __quod_hits and reads
// the arrays between tests, so each test (and the file as a whole) gets the
// set of functions it executed. The transform's source map is shifted by
// the insertions (insertions never span lines), so stack traces and
// positions are unchanged; each function's own range is mapped back to the
// original source through the transform's map before the edits.
//
// A module whose map is missing or has several sources, which does not
// parse, or whose AST offsets do not land on the expected characters is
// left alone: no function-level data for it (its tests use it whole).

const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
const B64I = new Map([...B64].map((c, i) => [c, i]))

// Absolute segments: [genCol, src, srcLine, srcCol, name?] per line.
export function decode(mappings) {
  const lines = []
  let s = 0
  let sl = 0
  let sc = 0
  let nm = 0
  for (const line of mappings.split(';')) {
    const segs = []
    let gc = 0
    for (const seg of line.split(',')) {
      if (!seg) continue
      const v = []
      let value = 0
      let shift = 0
      for (const ch of seg) {
        const d = B64I.get(ch)
        if (d === undefined) return null
        value += (d & 31) << shift
        if (d & 32) shift += 5
        else {
          v.push(value & 1 ? -(value >>> 1) : value >>> 1)
          value = 0
          shift = 0
        }
      }
      gc += v[0]
      const out = [gc]
      if (v.length >= 4) {
        s += v[1]
        sl += v[2]
        sc += v[3]
        out.push(s, sl, sc)
        if (v.length >= 5) {
          nm += v[4]
          out.push(nm)
        }
      }
      segs.push(out)
    }
    lines.push(segs)
  }
  return lines
}

function vlq(n) {
  let v = n < 0 ? (-n << 1) | 1 : n << 1
  let out = ''
  do {
    let d = v & 31
    v >>>= 5
    if (v) d |= 32
    out += B64[d]
  } while (v)
  return out
}

export function encode(lines) {
  let s = 0
  let sl = 0
  let sc = 0
  let nm = 0
  const out = []
  for (const segs of lines) {
    let gc = 0
    const parts = []
    for (const seg of segs) {
      let p = vlq(seg[0] - gc)
      gc = seg[0]
      if (seg.length >= 4) {
        p += vlq(seg[1] - s) + vlq(seg[2] - sl) + vlq(seg[3] - sc)
        s = seg[1]
        sl = seg[2]
        sc = seg[3]
        if (seg.length >= 5) {
          p += vlq(seg[4] - nm)
          nm = seg[4]
        }
      }
      parts.push(p)
    }
    out.push(parts.join(','))
  }
  return out.join(';')
}

function lineStarts(code) {
  const s = [0]
  for (let i = 0; i < code.length; i++) if (code.charCodeAt(i) === 10) s.push(i + 1)
  return s
}

function lineCol(starts, offset) {
  let lo = 0
  let hi = starts.length - 1
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1
    if (starts[mid] <= offset) lo = mid
    else hi = mid - 1
  }
  return [lo, offset - starts[lo]]
}

function original(lines, starts, offset) {
  const [l, c] = lineCol(starts, offset)
  let best = null
  for (const s of lines[l] ?? []) {
    if (s[0] <= c) best = s
    else break
  }
  if (!best || best.length < 4 || best[1] !== 0) return null
  return [best[2], best[3]]
}

const FN = new Set(['FunctionDeclaration', 'FunctionExpression', 'ArrowFunctionExpression'])

function functions(ast) {
  const out = []
  const stack = [ast]
  while (stack.length) {
    const n = stack.pop()
    if (!n || typeof n !== 'object') continue
    if (Array.isArray(n)) {
      for (const c of n) stack.push(c)
      continue
    }
    if (typeof n.type === 'string' && FN.has(n.type) && n.body) out.push(n)
    for (const k in n) {
      if (k === 'type' || k === 'start' || k === 'end' || k === 'range' || k === 'loc') continue
      const v = n[k]
      if (v && typeof v === 'object') stack.push(v)
    }
  }
  return out.sort((a, b) => a.start - b.start)
}

// Instrument one transform result in place. Returns the function table
// ([[startLine, startCol, endLine, endCol] | null] by index, 0-based, in the
// original source) or null when the module was left alone.
export function instrument(result, file, parseAst) {
  if (!result || result.__quod || typeof result.code !== 'string') return null
  result.__quod = true
  const map = result.map
  if (!map || typeof map.mappings !== 'string' || !Array.isArray(map.sources) || map.sources.length !== 1) return null
  const code = result.code
  let ast
  try {
    ast = parseAst(code)
  } catch {
    return null
  }
  const lines = decode(map.mappings)
  if (!lines) return null
  const starts = lineStarts(code)
  const fns = functions(ast)
  const ins = []
  const table = []
  for (let i = 0; i < fns.length; i++) {
    const f = fns[i]
    const body = f.body
    if (f.type === 'ArrowFunctionExpression' && body.type !== 'BlockStatement') {
      ins.push([body.start, `(__qh[${i}]=1,`, 0], [body.end, ')', 1])
    } else {
      if (code[body.start] !== '{' || code[body.end - 1] !== '}') return null
      let at = body.start + 1
      for (const st of body.body ?? []) {
        if (st.type === 'ExpressionStatement' && typeof st.directive === 'string') at = st.end
        else break
      }
      // after a directive with no semicolon, a statement needs one first
      ins.push([at, `${at === body.start + 1 ? '' : ';'}__qh[${i}]=1;`, 0])
    }
    if (code[f.end - 1] !== '}' && f.type !== 'ArrowFunctionExpression') return null
    const a = original(lines, starts, f.start)
    const b = original(lines, starts, f.end - 1)
    table.push(a && b && b[0] >= a[0] ? [a[0], a[1], b[0], b[1]] : null)
  }
  if (!fns.length) return null
  ins.push([0, `const __qh=(globalThis.__quod_hits||(n=>new Uint8Array(n)))(${fns.length},${JSON.stringify(file)});`, -1])
  // Apply from the end; a closing paren sorts before an opening one at the
  // same offset (`a)(b`: an expression ending where another begins).
  ins.sort((x, y) => x[0] - y[0] || y[2] - x[2])
  let outCode = ''
  let last = 0
  const shifts = new Map() // generated line -> [[col, len], ...] in order
  for (const [off, text] of ins) {
    outCode += code.slice(last, off) + text
    last = off
    const [l, c] = lineCol(starts, off)
    if (!shifts.has(l)) shifts.set(l, [])
    shifts.get(l).push([c, text.length])
  }
  outCode += code.slice(last)
  for (const [l, sh] of shifts) {
    const segs = lines[l]
    if (!segs) continue
    for (const seg of segs) {
      let add = 0
      for (const [c, n] of sh) if (c <= seg[0]) add += n
      seg[0] += add
    }
  }
  result.code = outCode
  result.map = { ...map, mappings: encode(lines) }
  return table
}
