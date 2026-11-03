// quod vitest: what a test consumed of an import.meta.glob map.
//
// A lazy glob compiles to a map of path -> importer, written into the
// globbing module's code from a directory listing. The modules a test loads
// through it are module inputs already (its graph); the listing itself only
// matters through what the test observed of the map:
//
//   - an enumeration (keys, entries, iteration, spread, inspect): the
//     whole listing, as before;
//   - a lookup of a path key, found or not: whether that path is in the
//     listing (`e:` of the file: the glob's patterns are fixed per path);
//   - convex-test's moduleCache (verified against its source), which
//     enumerates the map on every convexTest() call, but only to find the
//     first key containing "_generated" (the modules root) and to index
//     the map by path without extension, which it then reads one function
//     path at a time: the names under the globbed tree that contain
//     "_generated" (`rn.`), and for each path it looked up, the names in
//     its directory that start with "<name>." (`dp.`).
//
// Main process: instrumentGlobs() rewrites vite's output for each lazy
// glob, `/* #__PURE__ */ Object.assign({` -> `(global.$q=NNN,Object).assign({`,
// the same length, so positions, the source map and function-level coverage
// are untouched. Workers: observeGlobs() catches the assignment (a setter:
// the map was created) and the assign right after it (wraps the map in a
// Proxy that records what is read). Without the worker side (the critical
// path, a project Quod does not record) it is plain Object.assign.
// Anything not understood keeps today's keying: the module's whole listing.
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { real } from './record.mjs'

const HEAD = '/* #__PURE__ */ Object.assign('
const NEEDLE = '_generated'
const hex = (s) => Buffer.from(s, 'utf8').toString('hex')

// One JSON string literal at code[i] ('"'): its end, or -1.
function strEnd(code, i) {
  for (let j = i + 1; j < code.length; j++) {
    if (code[j] === '\\') j++
    else if (code[j] === '"') return j + 1
    else if (code[j] === '\n') return -1
  }
  return -1
}

// vite's object for one glob, at code[i] ('{'): its props, its end, and
// whether every value is a lazy importer of the key's own path.
function parseObject(code, i) {
  const props = []
  let j = i + 1
  let lazy = true
  for (;;) {
    while (code[j] === '\n') j++
    if (code[j] === '}') return { props, end: j + 1, lazy }
    if (code[j] !== '"') return null
    const ke = strEnd(code, j)
    if (ke < 0) return null
    let key
    try {
      key = JSON.parse(code.slice(j, ke))
    } catch {
      return null
    }
    if (code.slice(ke, ke + 2) !== ': ') return null
    j = ke + 2
    const imp = '() => import('
    if (code.startsWith(imp, j)) {
      const s0 = j + imp.length
      const se = code[s0] === '"' ? strEnd(code, s0) : -1
      if (se < 0 || code[se] !== ')') return null
      let spec
      try {
        spec = JSON.parse(code.slice(s0, se))
      } catch {
        return null
      }
      j = se + 1
      if (code.startsWith('.then(m => m[', j)) {
        const t0 = j + 13
        const te = code[t0] === '"' ? strEnd(code, t0) : -1
        if (te < 0 || code.slice(te, te + 2) !== '])') return null
        j = te + 2
      }
      // the key is the importer's own path (no `base`, not root-relative)
      if (!(key.startsWith('./') || key.startsWith('../')) || !(spec === key || spec.startsWith(`${key}?`))) lazy = false
    } else {
      // eager (an imported binding) or keys only: not narrowed
      const m = /^[\w$]+/.exec(code.slice(j, j + 200))
      if (!m) return null
      j += m[0].length
      lazy = false
    }
    props.push(key)
    if (code[j] === ',') j++
  }
}

// Main process. Returns the same-length code and the ids it gave the
// module's globs; `narrowable` when every glob the module has is a lazy map
// keyed by path (else the module keeps its whole listing).
export function instrumentGlobs(code, srcCount, nextId) {
  const ids = []
  let out = code
  let found = 0
  let ok = true
  for (let at = code.indexOf(HEAD); at >= 0; at = code.indexOf(HEAD, at + HEAD.length)) {
    found++
    const o = code[at + HEAD.length] === '{' ? parseObject(code, at + HEAD.length) : null
    if (!o || code[o.end] !== ')' || !o.lazy) {
      ok = false
      continue
    }
    const id = nextId()
    if (id == null) {
      ok = false
      continue
    }
    // ids are 100..999: three digits, no leading zero (an octal literal)
    const rep = `(global.$q=${id},Object).assign(`
    if (rep.length !== HEAD.length) {
      ok = false
      continue
    }
    out = out.slice(0, at) + rep + out.slice(at + HEAD.length)
    ids.push(id)
  }
  // every import.meta.glob in the source became one of these
  if (found !== srcCount) ok = false
  return { code: out, ids, narrowable: ok && ids.length > 0 }
}

// convex-test's moduleCache and findModulesRoot, as the model below assumes
// them (identical in 0.0.61 through 0.0.67).
const CT_SOURCE = `function moduleCache(specifiedModules) {
    const modules = specifiedModules ?? import.meta.glob("../../../convex/**/*.*s");
    const prefix = findModulesRoot(Object.keys(modules), specifiedModules !== undefined);
    const modulesWithoutExtension = Object.fromEntries(Object.entries(modules).map(([path, module]) => [
        path.replace(/\\.[^.]+$/, ""),
        module,
    ]));
    return async (path) => {
        const module = modulesWithoutExtension[prefix + path];
        if (module === undefined) {
            throw new Error(\`Could not find module for: "\${path}"\`);
        }
        return await module();
    };
}
function findModulesRoot(modulesPaths, userProvidedModules) {
    const generatedFilePath = modulesPaths.find((path) => path.includes("_generated"));
    if (generatedFilePath !== undefined) {
        return generatedFilePath.split("_generated", 2)[0];
    }
`
const SELF = fileURLToPath(import.meta.url)
const ctVerified = new Map()
// Is the code enumerating the map convex-test's moduleCache (the verified
// source)? The first frame that is not this file or a builtin.
function convexTestCaller() {
  const lines = (new Error().stack ?? '').split('\n').slice(1)
  for (const l of lines) {
    if (l.includes(SELF) || l.includes('(<anonymous>)') || l.includes('(native)') || /^\s*at (Function|Object|Reflect)\.\w+ \(/.test(l) || /^\s*at \w+ \(<anonymous>\)/.test(l)) continue
    const m = /^\s*at moduleCache \((?:file:\/\/)?(\/[^()]*\/node_modules\/convex-test\/dist\/index\.js)(?:\?[^:()]*)?:\d+:\d+\)$/.exec(l)
    if (!m) return false
    const f = m[1]
    if (!ctVerified.has(f)) {
      let ok = false
      try {
        ok = real.readFileSync(f, 'utf8').includes(CT_SOURCE)
      } catch {}
      ctVerified.set(f, ok)
    }
    return ctVerified.get(f)
  }
  return false
}

const INSPECT = Symbol.for('nodejs.util.inspect.custom')
const isPathKey = (k) => typeof k === 'string' && (k.startsWith('./') || k.startsWith('../') || k.startsWith('/'))

// Worker (record runs). Returns the observations, per glob id.
export function observeGlobs() {
  const obs = new Map() // id -> { enumerated, ct, keys, stripped }
  const realAssign = Object.assign
  const realFromEntries = Object.fromEntries
  let pending
  let inCT = null // the map convex-test's moduleCache is reading
  const g = globalThis
  // a map made before this file could watch: none of them is observed
  if (Object.prototype.hasOwnProperty.call(g, '$q')) return null
  try {
    Object.defineProperty(g, '$q', {
      configurable: true,
      enumerable: false,
      get: () => undefined,
      set(v) {
        // a map assigned but never wrapped (someone replaced Object.assign):
        // it was consumed unobserved
        if (pending !== undefined) obs.get(pending).enumerated = true
        pending = v
        if (!obs.has(v)) obs.set(v, { enumerated: false, ct: false, keys: new Set(), stripped: new Set(), created: 0 })
        obs.get(v).created++
      },
    })
  } catch {
    return null
  }
  const wrap = (target, o) => {
    const look = (k) => {
      if (inCT !== o && isPathKey(k)) o.keys.add(k)
    }
    const enumerate = () => {
      if (inCT === o) return
      if (inCT === null && convexTestCaller()) {
        // moduleCache: Object.keys, then Object.entries, then
        // Object.fromEntries, synchronously; its lookups go to what
        // fromEntries returns
        o.ct = true
        inCT = o
        Object.fromEntries = function fromEntries(it) {
          Object.fromEntries = realFromEntries
          inCT = null
          const byPath = realFromEntries(it)
          const rec = (k) => {
            if (typeof k === 'string') o.stripped.add(k)
          }
          return new Proxy(byPath, {
            get: (t, k, r) => (rec(k), Reflect.get(t, k, r)),
            has: (t, k) => (rec(k), Reflect.has(t, k)),
            getOwnPropertyDescriptor: (t, k) => (rec(k), Reflect.getOwnPropertyDescriptor(t, k)),
            ownKeys: (t) => ((o.enumerated = true), Reflect.ownKeys(t)),
          })
        }
        queueMicrotask(() => {
          if (inCT === o) {
            // not the sequence the model expects: the whole listing
            inCT = null
            Object.fromEntries = realFromEntries
            o.enumerated = true
          }
        })
        return
      }
      o.enumerated = true
    }
    // util.inspect (console.log) reads the target, not through the traps
    try {
      Object.defineProperty(target, INSPECT, {
        configurable: true,
        enumerable: false,
        writable: true,
        value() {
          o.enumerated = true
          return realAssign({}, target)
        },
      })
    } catch {
      o.enumerated = true
    }
    return new Proxy(target, {
      get: (t, k, r) => (k === INSPECT ? undefined : (look(k), Reflect.get(t, k, r))),
      has: (t, k) => (k === INSPECT ? false : (look(k), Reflect.has(t, k))),
      getOwnPropertyDescriptor: (t, k) => (k === INSPECT ? undefined : (look(k), Reflect.getOwnPropertyDescriptor(t, k))),
      ownKeys: (t) => (enumerate(), Reflect.ownKeys(t).filter((k) => k !== INSPECT)),
      preventExtensions: (t) => {
        delete t[INSPECT]
        o.enumerated = true
        return Reflect.preventExtensions(t)
      },
    })
  }
  const ours = function assign(...a) {
    if (pending === undefined) return realAssign(...a)
    const o = obs.get(pending)
    pending = undefined
    const target = realAssign(...a)
    return a.length === 1 ? wrap(target, o) : ((o.enumerated = true), target)
  }
  Object.assign = ours
  return {
    // at the end of the file: what each map it created was asked
    dump() {
      if (pending !== undefined) obs.get(pending).enumerated = true
      if (Object.assign !== ours) for (const o of obs.values()) o.enumerated = true
      return [...obs].map(([id, o]) => ({ id, created: o.created, enumerated: o.enumerated, ct: o.ct, keys: [...o.keys], stripped: [...o.stripped] }))
    },
  }
}

// Driver: the keys a test's observations of module `mod`'s globs come to,
// or null for the module's whole listing. `listing` is what the module's
// globs listed (d:/r: keys); `globs` the test's observations of the ids
// the module's transform gave out.
export function globKeys(mod, listing, globs, lstat) {
  const dir = path.dirname(mod)
  const canon = (abs) => {
    const r = path.relative(dir, abs)
    return r.startsWith('../') || r.startsWith('./') ? r : `./${r}`
  }
  const out = new Set()
  for (const o of globs) {
    if (o.enumerated) return null
    for (const k of o.keys) {
      if (k.startsWith('/')) return null
      const abs = path.resolve(dir, k)
      if (canon(abs) !== k) continue // vite's keys are canonical: never in the map
      const st = lstat(abs)
      if (st?.isSymbolicLink()) return null
      out.add(`e:${abs}`)
    }
    if (o.ct) {
      const dirs = [...listing].map((k) => k.slice(k.indexOf(':') + 1))
      if (!dirs.length) return null
      let root = dirs[0]
      for (const d of dirs) while (root !== '/' && d !== root && !d.startsWith(`${root}/`)) root = path.dirname(root)
      if (path.relative(dir, root).includes(NEEDLE)) return null
      out.add(`rn.${hex(NEEDLE)}:${root}`)
    }
    for (const l of o.stripped) {
      if (!(l.startsWith('./') || l.startsWith('../')) || l.endsWith('/') || /(^|\/)\.\.?$/.test(l)) return null
      const abs = path.resolve(dir, l)
      if (canon(abs) !== l) return null
      const base = `${path.basename(abs)}.`
      const pdir = path.dirname(abs)
      // a directory "<name>.x" would put candidates below it
      let names = []
      try {
        names = real.readdirSync(pdir, { withFileTypes: true }).filter((e) => e.name.startsWith(base))
      } catch {}
      if (names.some((e) => !e.isFile())) return null
      out.add(`dp.${hex(base)}:${pdir}`)
    }
  }
  return [...out]
}
