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

export function createRecorder({ stateDir, mode }) {
  const keys = new Set()
  const r = {
    mode, // 'main' | 'worker'
    keys,
    writes: new Set(),
    unobservable: new Set(),
    on: true,
    add(kind, p) {
      if (!r.on || p == null) return
      p = norm(p)
      if (p == null || p.startsWith(stateDir)) return
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
        if (!name) return
        // node_modules/.pnpm/lock.yaml and the like: a plain file
      }
      if (kind === 'f' && r.mode === 'main' && p.endsWith('/package.json')) kind = 'pj'
      keys.add(`${kind}:${p}`)
    },
    write(p) {
      if (!r.on || p == null) return
      p = norm(p)
      if (p != null && !p.startsWith(stateDir)) r.writes.add(p)
    },
    unobs(reason) {
      if (r.on) r.unobservable.add(reason)
    },
  }
  return r
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
  wrap(fs, 'watch', () => r.unobs('fs.watch'))
  wrap(fs, 'watchFile', () => r.unobs('fs.watchFile'))
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
// thread. Workers only (the main process's own are the tools').
export async function patchEscapes(r) {
  const cp = await import('node:child_process')
  for (const n of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) {
    wrap(cp.default, n, () => r.unobs('child_process'))
  }
  const net = (await import('node:net')).default
  wrap(net, 'connect', () => r.unobs('network'))
  wrap(net, 'createConnection', () => r.unobs('network'))
  wrap(net.Socket.prototype, 'connect', () => r.unobs('network'))
  wrap((await import('node:tls')).default, 'connect', () => r.unobs('network'))
  for (const m of ['node:http', 'node:https']) {
    const h = (await import(m)).default
    wrap(h, 'request', () => r.unobs('network'))
    wrap(h, 'get', () => r.unobs('network'))
  }
  wrap((await import('node:dgram')).default, 'createSocket', () => r.unobs('network'))
  wrap((await import('node:dns')).default, 'lookup', () => r.unobs('network'))
  wrap(globalThis, 'fetch', () => r.unobs('network'))
  wrap((await import('node:worker_threads')).default, 'Worker', () => r.unobs('worker_threads'))
  try {
    nodeModule.syncBuiltinESMExports()
  } catch {}
}

// Write JSON atomically with the unrecorded fs.
export function writeJson(file, value) {
  const tmp = `${file}.${process.pid}.tmp`
  real.writeFileSync(tmp, JSON.stringify(value))
  real.renameSync(tmp, file)
}
