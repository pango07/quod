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

import path from 'node:path'
import crypto from 'node:crypto'

// Quod's own I/O (never recorded): set by the caller.
export const io = { read: null, write: null, rename: null, mkdir: null }

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

// The last inline map comment: vitest's module cache appends its own
// metadata after it, so it is not always the code's last line.
const MAP_RE = /\/\/# sourceMappingURL=data:application\/json;(?:charset=utf-8;)?base64,([A-Za-z0-9+/=]+)/g
function lastMap(code) {
  let m = null
  MAP_RE.lastIndex = 0
  for (let x; (x = MAP_RE.exec(code)); ) m = x
  return m
}

// The module's inline map, and which of its sources is the file itself (a
// map may list more: a plugin's helper, a virtual module); positions that
// map to another source count as unmapped.
function inlineMap(code, file) {
  const m = lastMap(code)
  if (!m) {
    unmappedSample[file] ??= code.slice(-200)
    return { why: 'no inline map' }
  }
  try {
    const map = JSON.parse(Buffer.from(m[1], 'base64').toString('utf8'))
    if (!Array.isArray(map.sources) || !map.sources.length) return { why: 'a map without sources' }
    let src = 0
    if (map.sources.length > 1) {
      const dir = path.dirname(file)
      const root = map.sourceRoot ?? ''
      const hits = map.sources
        .map((x, i) => [i, x == null ? '' : x.startsWith('file://') ? x.slice(7) : x])
        .filter(([, x]) => path.resolve(dir, root, x) === file || x === file)
      if (hits.length !== 1) return { why: 'a map with several sources, none (or several) the file' }
      src = hits[0][0]
    }
    const decoded = decodeMappings(map.mappings ?? '')
    return decoded ? { lines: decoded, src } : { why: 'unreadable mappings' }
  } catch {
    return { why: 'unreadable map' }
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
  if (!best || best.length < 4 || best[1] !== map.src) return null
  return [best[2], best[3]]
}

// Why files got no entry (for diagnostics): reason -> count.
export const unmapped = {}
// One example of the end of a module whose map was not found (diagnostics).
export const unmappedSample = {}
const miss = (why) => (unmapped[why] = (unmapped[why] ?? 0) + 1)

const EXPORT_GETTER =
  /^(?:get\s*\(\)|\(\)\s*=>)\s*\{\s*(?:try\s*\{\s*)?return\s+[\w$.]+;?\s*(?:\}\s*catch\s*(?:\(\w*\))?\s*\{\s*\})?\s*\}$/

// The mapping of one module's functions, cached by a hash of the file name
// and its transformed code (the inline map included): the same code maps the
// same way, so a hit is the computation's own answer. Per code offset pair
// (`start:end`, relative to the code): [startLine, startCol, endLine,
// endCol], 0 (an export getter: skipped) or -1 (maps nowhere); `map:` the
// reason a module has no usable map. Written whole, atomically; two workers
// racing lose only entries, which are recomputed.
// The key also covers this module's own source (the mapping's rules).
let self = null
const sha1 = (x) => (crypto.hash ? crypto.hash('sha1', x) : crypto.createHash('sha1').update(x).digest('hex'))
function cached(dir, file, code) {
  if (!dir) return { get: () => undefined, set() {}, save() {} }
  let key
  try {
    self ??= sha1(io.read(new URL(import.meta.url), 'utf8'))
    key = sha1(`${self}\0${file}\0${code}`)
  } catch {
    return { get: () => undefined, set() {}, save() {} }
  }
  const p = `${dir}/${key}.json`
  let c = null
  try {
    c = JSON.parse(io.read(p, 'utf8'))
  } catch {}
  let dirty = false
  return {
    get: (k) => c?.[k],
    set(k, v) {
      c ??= {}
      c[k] = v
      dirty = true
    },
    save() {
      if (!dirty) return
      try {
        io.mkdir(dir, { recursive: true })
        const tmp = `${p}.${process.pid}.tmp`
        io.write(tmp, JSON.stringify(c))
        io.rename(tmp, p)
      } catch {}
    },
  }
}

export function executedFunctions(scripts, em, root, cacheDir = null) {
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
    if (!node) {
      miss('script not in the module graph')
      continue
    }
    const file = (node.file || '').split('?')[0]
    if (!file.startsWith(prefixRoot) || file.includes('/node_modules/')) continue
    const code = node.meta.code
    const top = s.functions.find((f) => f.ranges[0]?.startOffset === 0)
    if (!top) {
      miss('no top-level function')
      continue
    }
    const prefix = top.ranges[0].endOffset - code.length - 3
    // the wrapper: `'use strict';async (...)=>{{`
    const wrapper = s.functions.find((f) => f.ranges[0]?.startOffset === 13)
    if (prefix <= 13 || !wrapper || wrapper.ranges[0].endOffset !== top.ranges[0].endOffset) {
      miss('script is not the module runner wrapper around its code')
      continue
    }
    const c = cached(cacheDir, file, code)
    let map = null
    let starts = null
    const why = c.get('map:')
    if (why) {
      miss(why)
      continue
    }
    const fns = []
    let ok = true
    for (const f of s.functions) {
      const r = f.ranges[0]
      if (!r || r.count === 0 || r.startOffset < prefix) continue
      const k = `${r.startOffset - prefix}:${r.endOffset - prefix}`
      let v = c.get(k)
      if (v === undefined) {
        if (!map) {
          map = inlineMap(code, file)
          if (!map.lines) {
            c.set('map:', map.why)
            break
          }
          starts = lineStarts(code)
        }
        const a = original(map, starts, r.startOffset - prefix)
        const b = original(map, starts, r.endOffset - 1 - prefix)
        if (!a || !b || b[0] < a[0]) {
          // vite's SSR transform adds an export getter per export
          // (`get(){ return x }`, `() => { try { return x } catch {} }`):
          // generated code, mapped nowhere, that only reads a binding (whose
          // own code is keyed where it is).
          const text = code.slice(r.startOffset - prefix, r.endOffset - prefix)
          if (EXPORT_GETTER.test(text)) v = 0
          else {
            v = -1
            unmappedSample[`${file} (function)`] ??= text.slice(0, 120)
          }
        } else v = [a[0], a[1], b[0], b[1]]
        c.set(k, v)
      }
      if (v === 0) continue
      if (v === -1) {
        ok = false
        break
      }
      fns.push(v)
    }
    c.save()
    if (map && !map.lines) {
      miss(map.why)
      continue
    }
    if (ok) out[file] = fns
    else miss('an executed function maps nowhere')
  }
  return out
}
