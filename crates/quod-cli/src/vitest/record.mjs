// quod vitest: the recorder. Shared by the driver (vitest's main process)
// and the setup file (each test file's worker). It records, by wrapping
// node's fs, env, module loader and process/network entry points, every
// input a process reads, as dependency keys the Rust side hashes:
//
//   f:<path>   a file's content (or that it is absent or a directory)
//   pj:<path>  a workspace package.json, with dependency version values
//              blanked (the installed versions are p: keys)
//   d:<path>   a directory's listing        r:<path>  a recursive listing
//   e:<path>   that a path exists, its type, and a symlink's target
//   p:<path>   an installed package (its root and package.json)
//   env:<KEY>  one environment variable     envall:   all of them (but the
//                                                     per-run CI ones)
//
// Over-record, never under-record: anything this cannot see (a child
// process, the network, a worker thread) makes the test file unobservable,
// and an unobservable file always runs.
import fs from 'node:fs'
import path from 'node:path'
import nodeModule from 'node:module'
import { fileURLToPath } from 'node:url'

// The originals, for Quod's own I/O (never recorded).
export const real = {
  readFileSync: fs.readFileSync,
  writeFileSync: fs.writeFileSync,
  renameSync: fs.renameSync,
  mkdirSync: fs.mkdirSync,
  existsSync: fs.existsSync,
  realpathSync: fs.realpathSync,
  readdirSync: fs.readdirSync,
}

export const VERSION = 'quod-vitest-1'

const SKIP_NM = ['/node_modules/.vite/', '/node_modules/.vitest/', '/node_modules/.cache/']

export function createRecorder({ stateDir, codeDir = stateDir, mode }) {
  const keys = new Set()
  const r = {
    mode, // 'main' | 'worker'
    keys,
    writes: new Set(),
    watches: new Set(),
    // package.json files kept whole in the main process, and who read them
    pjWhole: new Map(),
    unobservable: new Set(),
    on: true,
    add(kind, p) {
      if (!r.on || p == null) return
      p = norm(p)
      if (p == null || p.startsWith(stateDir) || p.startsWith(codeDir)) return
      if (p.startsWith('/proc/') || p.startsWith('/dev/') || p.startsWith('/sys/')) return
      const nm = p.lastIndexOf('/node_modules/')
      if (nm >= 0) {
        if (SKIP_NM.some((s) => p.includes(s))) return
        const rest = p.slice(nm + 14)
        const seg = rest.split('/')
        const name = seg[0].startsWith('@') ? `${seg[0]}/${seg[1] ?? ''}` : seg[0]
        if (name && !name.startsWith('.')) {
          keys.add(`p:${p.slice(0, nm + 14)}${name}`)
          return
        }
        // A dot entry of node_modules is the package manager's or a tool's
        // (.bin shims, .modules.yaml, .pnpm/lock.yaml, workspace state,
        // caches), not a package: what is installed counts through the
        // packages a run loads (p:), so a reinstall alone changes nothing.
        return
      }
      if (kind === 'f' && r.mode === 'main' && p.endsWith('/package.json')) {
        if (resolverCaller()) kind = 'pj'
        else if (!r.pjWhole.has(p)) r.pjWhole.set(p, callerFrame())
      }
      keys.add(`${kind}:${p}`)
    },
    write(p) {
      if (!r.on || p == null) return
      p = norm(p)
      if (p != null && !p.startsWith(stateDir) && !p.startsWith(codeDir)) r.writes.add(p)
    },
    unobs(reason) {
      if (r.on) r.unobservable.add(reason)
    },
  }
  return r
}

// Is a package.json read vite's or vitest's own (module resolution, which
// uses the resolution fields only)? The first frame outside Quod and node
// must be in one of their packages. Any other reader (the config, a plugin
// that injects `version`, a test) keeps the whole file.
const SELF = fileURLToPath(import.meta.url)
const RESOLVERS = ['/node_modules/vite/', '/node_modules/vitest/', '/node_modules/@vitest/', '/node_modules/rolldown/', '/node_modules/@rolldown/']
export function resolverCaller(stack = new Error().stack) {
  const lines = (stack ?? '').split('\n').slice(1)
  for (const l of lines) {
    if (l.includes(SELF) || l.includes('/.quod-vitest/') || l.includes('(node:') || l.includes(' node:') || l.includes('<anonymous>')) continue
    if (l.includes('/node_modules/.vite-temp/')) return false
    return RESOLVERS.some((x) => l.includes(x))
  }
  return false
}

function callerFrame(stack = new Error().stack) {
  for (const l of (stack ?? '').split('\n').slice(1)) {
    if (l.includes(SELF) || l.includes('/.quod-vitest/') || l.includes('(node:') || l.includes(' node:') || l.includes('<anonymous>')) continue
    return l.trim()
  }
  return '?'
}

function norm(p) {
  try {
    if (typeof p === 'number') return null
    if (p instanceof URL) p = fileURLToPath(p)
    else if (typeof p !== 'string') p = String(p)
    if (p.startsWith('file:')) p = fileURLToPath(p)
    return path.resolve(p)
  } catch {
    return null
  }
}

function wrap(obj, name, before) {
  const f = obj?.[name]
  if (typeof f !== 'function' || f.__quod) return
  const w = function (...args) {
    try {
      before(args)
    } catch {}
    return new.target ? Reflect.construct(f, args, new.target) : f.apply(this, args)
  }
  try {
    Object.defineProperties(w, Object.getOwnPropertyDescriptors(f))
  } catch {}
  Object.defineProperty(w, '__quod', { value: true })
  obj[name] = w
}

function isWrite(flags) {
  if (flags == null) return false
  if (typeof flags === 'number') {
    const c = fs.constants
    return (flags & (c.O_WRONLY | c.O_RDWR | c.O_CREAT | c.O_TRUNC | c.O_APPEND)) !== 0
  }
  return /[wa+]/.test(String(flags))
}
function isRead(flags) {
  if (flags == null) return true
  if (typeof flags === 'number') return (flags & fs.constants.O_WRONLY) === 0
  return /[r+]/.test(String(flags))
}

export function patchFs(r) {
  const P = fs.promises
  const both = (names, fn) => {
    for (const n of names) {
      wrap(fs, n, fn)
      wrap(fs, `${n}Sync`, fn)
      wrap(P, n, fn)
    }
  }
  const file = (a) => r.add('f', a[0])
  both(['readFile'], file)
  wrap(fs, 'createReadStream', file)
  // stat-like calls see size: a content dependency; lstat also sees a link
  both(['stat'], file)
  both(['lstat'], (a) => {
    r.add('f', a[0])
    r.add('e', a[0])
  })
  both(['access'], (a) => r.add('e', a[0]))
  wrap(fs, 'existsSync', (a) => r.add('e', a[0]))
  wrap(fs, 'exists', (a) => r.add('e', a[0]))
  both(['realpath', 'readlink'], (a) => r.add('e', a[0]))
  wrap(fs.realpathSync, 'native', (a) => r.add('e', a[0]))
  wrap(fs.realpath, 'native', (a) => r.add('e', a[0]))
  const dir = (a) => {
    const o = a[1]
    r.add(o && typeof o === 'object' && o.recursive ? 'r' : 'd', a[0])
  }
  both(['readdir', 'opendir'], dir)
  both(['open'], (a) => {
    if (isRead(a[1])) r.add('f', a[0])
    if (isWrite(a[1])) r.write(a[0])
  })
  const w0 = (a) => r.write(a[0])
  both(
    [
      'writeFile', 'appendFile', 'mkdir', 'rm', 'rmdir', 'unlink', 'truncate',
      'utimes', 'lutimes', 'chmod', 'lchmod', 'chown', 'lchown', 'mkdtemp',
    ],
    w0,
  )
  wrap(fs, 'createWriteStream', w0)
  const move = (a) => {
    r.add('f', a[0])
    r.write(a[0])
    r.write(a[1])
  }
  both(['rename'], move)
  const copy = (a) => {
    r.add('f', a[0])
    r.add('r', a[0])
    r.write(a[1])
  }
  both(['copyFile', 'cp', 'link', 'symlink'], copy)
  // A watch sees whatever lands under its path, for as long as it lives:
  // the driver keys it as the path's whole recursive content at the end
  // of the file (or not at all, under a directory the file created).
  const watch = (a) => {
    const p = norm(a[0])
    if (p && r.on) r.watches.add(p)
  }
  wrap(fs, 'watch', watch)
  wrap(fs, 'watchFile', watch)
  wrap(P, 'watch', watch)
  try {
    nodeModule.syncBuiltinESMExports()
  } catch {}
}

export function patchEnv(r) {
  const target = process.env
  const rec = (k) => {
    if (typeof k === 'string') r.keys.add(`env:${k}`)
  }
  const proxy = new Proxy(target, {
    get(t, k) {
      if (r.on) rec(k)
      return Reflect.get(t, k)
    },
    has(t, k) {
      if (r.on) rec(k)
      return Reflect.has(t, k)
    },
    getOwnPropertyDescriptor(t, k) {
      if (r.on) rec(k)
      return Reflect.getOwnPropertyDescriptor(t, k)
    },
    ownKeys(t) {
      if (r.on) r.keys.add('envall:')
      return Reflect.ownKeys(t)
    },
  })
  try {
    process.env = proxy
  } catch {
    r.unobs('env')
  }
}

// Loads through node's loader (ESM and CJS), from now on.
export function patchLoader(r) {
  if (typeof nodeModule.registerHooks === 'function') {
    nodeModule.registerHooks({
      load(url, context, next) {
        if (url.startsWith('file:')) r.add('f', url)
        return next(url, context)
      },
    })
  } else {
    r.loaderMissing = true
  }
  wrap(process, 'dlopen', (a) => r.add('f', a[1]))
}

// What the recorder cannot follow: another process, the network, another
// thread. Workers only (the main process's own are the tools'). A builtin
// already loaded is patched now; any other when first resolved (loading
// them all up front cost ~30 ms per test file).
const ESCAPES = {
  child_process: (m, r) => {
    for (const n of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) {
      wrap(m, n, () => r.unobs('child_process'))
    }
  },
  net: (m, r) => {
    wrap(m, 'connect', () => r.unobs('network'))
    wrap(m, 'createConnection', () => r.unobs('network'))
    wrap(m.Socket?.prototype, 'connect', () => r.unobs('network'))
  },
  tls: (m, r) => wrap(m, 'connect', () => r.unobs('network')),
  http: (m, r) => ['request', 'get'].forEach((n) => wrap(m, n, () => r.unobs('network'))),
  https: (m, r) => ['request', 'get'].forEach((n) => wrap(m, n, () => r.unobs('network'))),
  http2: (m, r) => wrap(m, 'connect', () => r.unobs('network')),
  dgram: (m, r) => wrap(m, 'createSocket', () => r.unobs('network')),
  dns: (m, r) => ['lookup', 'resolve'].forEach((n) => wrap(m, n, () => r.unobs('network'))),
  worker_threads: (m, r) => wrap(m, 'Worker', () => r.unobs('worker_threads')),
  cluster: (m, r) => wrap(m, 'fork', () => r.unobs('child_process')),
}
export function patchEscapes(r) {
  const done = new Set()
  const patch = (name) => {
    if (done.has(name)) return
    done.add(name)
    try {
      ESCAPES[name](process.getBuiltinModule(name), r)
      nodeModule.syncBuiltinESMExports()
    } catch {
      r.unobs(`could not watch ${name}`)
    }
  }
  const loaded = new Set(process.moduleLoadList?.filter((m) => m.startsWith('NativeModule ')).map((m) => m.slice(13)))
  for (const name of Object.keys(ESCAPES)) if (loaded.has(name) || !process.moduleLoadList) patch(name)
  if (typeof nodeModule.registerHooks === 'function') {
    nodeModule.registerHooks({
      resolve(specifier, context, next) {
        const name = specifier.startsWith('node:') ? specifier.slice(5) : specifier
        if (Object.hasOwn(ESCAPES, name)) patch(name)
        return next(specifier, context)
      },
    })
  } else for (const name of Object.keys(ESCAPES)) patch(name)
  wrap(globalThis, 'fetch', () => r.unobs('network'))
  if (typeof globalThis.WebSocket === 'function') wrap(globalThis, 'WebSocket', () => r.unobs('network'))
}

// Write JSON atomically with the unrecorded fs.
export function writeJson(file, value) {
  const tmp = `${file}.${process.pid}.tmp`
  real.writeFileSync(tmp, JSON.stringify(value))
  real.renameSync(tmp, file)
}
