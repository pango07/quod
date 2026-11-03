// quod vitest: which functions of the workspace's source a test file
// executed, in the source's own coordinates.
//
// V8's precise coverage (function counts) reports, per script, every
// function with its range in the script. vitest's module runner evaluates a
// module as `<prefix>${code}\n}}` (an async arrow wrapping the transformed
// code), so a range minus the prefix is an offset in the transformed code,
// and the code's inline source map takes it back to the original file. A
// file whose script cannot be matched to its transformed code exactly, or
// whose map has more than one source or leaves an executed function's
// start or end unmapped, gets no entry: no function-level narrowing for it.
//
// Result: { "<abs file>": [[startLine, startCol, endLine, endCol], ...] },
// 0-based, the end being the function's last character.

const B64 = new Map([...'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'].map((c, i) => [c, i]))

function decodeMappings(mappings) {
  const lines = []
  let srcIdx = 0
  let srcLine = 0
  let srcCol = 0
  for (const line of mappings.split(';')) {
    const segs = []
    let genCol = 0
    for (const seg of line.split(',')) {
      if (!seg) continue
      const v = []
      let value = 0
      let shift = 0
      for (const ch of seg) {
        const d = B64.get(ch)
        if (d === undefined) return null
        value += (d & 31) << shift
        if (d & 32) shift += 5
        else {
          v.push(value & 1 ? -(value >>> 1) : value >>> 1)
          value = 0
          shift = 0
        }
      }
      genCol += v[0]
      if (v.length >= 4) {
        srcIdx += v[1]
        srcLine += v[2]
        srcCol += v[3]
        segs.push([genCol, srcIdx, srcLine, srcCol])
      } else segs.push([genCol])
    }
    lines.push(segs)
  }
  return lines
}

const MAP_RE = /\/\/# sourceMappingURL=data:application\/json;(?:charset=utf-8;)?base64,([A-Za-z0-9+/=]+)\s*$/

function inlineMap(code) {
  const m = MAP_RE.exec(code)
  if (!m) return null
  try {
    const map = JSON.parse(Buffer.from(m[1], 'base64').toString('utf8'))
    if (!Array.isArray(map.sources) || map.sources.length !== 1) return null
    const decoded = decodeMappings(map.mappings ?? '')
    return decoded ? { lines: decoded } : null
  } catch {
    return null
  }
}

function lineStarts(code) {
  const s = [0]
  for (let i = 0; i < code.length; i++) if (code.charCodeAt(i) === 10) s.push(i + 1)
  return s
}

function original(map, starts, offset) {
  let lo = 0
  let hi = starts.length - 1
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1
    if (starts[mid] <= offset) lo = mid
    else hi = mid - 1
  }
  const col = offset - starts[lo]
  const segs = map.lines[lo]
  if (!segs) return null
  let best = null
  for (const s of segs) {
    if (s[0] <= col) best = s
    else break
  }
  if (!best || best.length < 4 || best[1] !== 0) return null
  return [best[2], best[3]]
}

export function executedFunctions(scripts, em, root) {
  const byId = new Map()
  for (const node of em?.idToModuleMap?.values() ?? []) {
    if (typeof node.meta?.code === 'string') byId.set(node.id, node)
  }
  const prefixRoot = root.endsWith('/') ? root : `${root}/`
  const out = {}
  for (const s of scripts) {
    const url = s.url.startsWith('file://') ? s.url.slice(7) : s.url
    if (!url.startsWith(prefixRoot) || url.includes('/node_modules/')) continue
    const node = byId.get(s.url) ?? byId.get(url)
    if (!node) continue
    const file = (node.file || '').split('?')[0]
    if (!file.startsWith(prefixRoot) || file.includes('/node_modules/')) continue
    const code = node.meta.code
    const top = s.functions.find((f) => f.ranges[0]?.startOffset === 0)
    if (!top) continue
    const prefix = top.ranges[0].endOffset - code.length - 3
    // the wrapper: `'use strict';async (...)=>{{`
    const wrapper = s.functions.find((f) => f.ranges[0]?.startOffset === 13)
    if (prefix <= 13 || !wrapper || wrapper.ranges[0].endOffset !== top.ranges[0].endOffset) continue
    const map = inlineMap(code)
    if (!map) continue
    const starts = lineStarts(code)
    const fns = []
    let ok = true
    for (const f of s.functions) {
      const r = f.ranges[0]
      if (!r || r.count === 0 || r.startOffset < prefix) continue
      const a = original(map, starts, r.startOffset - prefix)
      const b = original(map, starts, r.endOffset - 1 - prefix)
      if (!a || !b || b[0] < a[0]) {
        ok = false
        break
      }
      fns.push([a[0], a[1], b[0], b[1]])
    }
    if (ok) out[file] = fns
  }
  return out
}
