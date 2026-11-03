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

export function createRecorder({ stateDir, codeDir = stateDir, mode, wsRoot = null }) {
  const keys = new Set()
  // A made directory's other spelling (its real path, and back), when they differ.
  const alias = new Map()
  // A path under a made directory is this process's own output, not an input.
  const under = (p) => r.made.some((m) => p === m || p.startsWith(`${m}/`))
  // vitest's own per-run directories (the driver's QUOD_VITEST_SKIP, one per
  // line): vitest 3 hands a worker each transformed module as a file in its
  // project's temp directory, a new one every run. What is in it is a
  // module's transform, keyed where it comes from (the module graph, the
  // config); the path itself is not an input.
  const skip = (process.env.QUOD_VITEST_SKIP ?? '').split('\n').filter(Boolean).map((d) => (d.endsWith('/') ? d : `${d}/`))
  const r = {
    mode, // 'main' | 'worker'
    keys,
    writes: new Set(),
    watches: new Set(),
    // package.json files kept whole in the main process, and who read them
    pjWhole: new Map(),
    // CI-provider variables (and `*`: all of them) keyed by a read outside the tools, and who read them
    envKept: new Map(),
    unobservable: new Set(),
    // Directories this process created (mkdir, recursive mkdir, mkdtemp)
    // outside the workspace, while recording: what is under one is the
    // file's own output, not an input. Nothing under it existed before it
    // was made, so a read there cannot depend on anything but this run.
    // Without a workspace root (wsRoot, with a trailing slash) none count.
    made: [],
    on: true,
    created(p) {
      if (!r.on || !wsRoot || p == null) return
      p = norm(p)
      if (p == null || p === '/' || `${p}/`.startsWith(wsRoot) || under(p)) return
      r.made.push(p)
      // and the same directory by its real path (macOS: /var is /private/var)
      try {
        const rp = real.realpathSync(p)
        if (rp !== p && !`${rp}/`.startsWith(wsRoot)) {
          r.made.push(rp)
          alias.set(p, rp)
          alias.set(rp, p)
        }
      } catch {}
    },
    // Removed or moved away: what appears there later is not its own.
    gone(p) {
      p = norm(p)
      if (p == null) return
      const out = new Set(r.made.filter((m) => m === p || m.startsWith(`${p}/`)))
      for (const m of [...out]) if (alias.has(m)) out.add(alias.get(m))
      r.made = r.made.filter((m) => !out.has(m))
    },
    add(kind, p) {
      if (!r.on || p == null) return
      p = norm(p)
      if (p == null || p.startsWith(stateDir) || p.startsWith(codeDir)) return
      if (r.made.length && under(p)) return
      if (skip.length && skip.some((d) => p.startsWith(d))) return
      if (p.startsWith('/proc/') || p.startsWith('/dev/') || p.startsWith('/sys/')) return
      const nm = p.lastIndexOf('/node_modules/')
      if (nm >= 0) {
        if (SKIP_NM.some((s) => p.includes(s))) return
        const rest = p.slice(nm + 14)
        const seg = rest.split('/')
        const name = seg[0].startsWith('@') ? `${seg[0]}/${seg[1] ?? ''}` : seg[0]
        // A dot entry of node_modules is the package manager's or a tool's
        // (.bin shims, .modules.yaml, .pnpm/lock.yaml, workspace state,
        // caches), not a package: what is installed counts through the
        // packages a run loads (p:), so a reinstall alone changes nothing.
        if (!name || name.startsWith('.')) return
        const pkg = `${p.slice(0, nm + 14)}${name}`
        // A workspace package linked into node_modules (npm, pnpm and yarn
        // workspaces) is source, not an installed package: its package.json
        // says nothing about its other files. A read through the link is
        // keyed at the real path, as a read there would be, plus the link.
        const ws = workspaceLink(pkg)
        if (!ws) {
          keys.add(`p:${pkg}`)
          return
        }
        keys.add(`e:${pkg}`)
        p = ws + p.slice(pkg.length)
        if (p.includes('/node_modules/')) return r.add(kind, p)
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

// The real path of a package root under node_modules that is a link to a
// directory outside every node_modules (a workspace package), else null.
// An installed package (a plain directory, or pnpm's link into its store,
// which is under node_modules/.pnpm) is null: its package.json names it.
const links = new Map()
export function workspaceLink(pkg) {
  let ws = links.get(pkg)
  if (ws !== undefined) return ws
  ws = null
  try {
    const target = real.realpathSync(pkg)
    if (target !== pkg && !`${target}/`.includes('/node_modules/')) ws = target
  } catch {}
  links.set(pkg, ws)
  return ws
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

// Is an environment read the tools' own? Every frame outside Quod and node
// must be in vite, vitest, rolldown or the small helpers they read the
// environment through (std-env, tinyexec, tinyrainbow, obug and debug
// listing DEBUG_* options, the colour helpers), and there must be one. Any
// other frame (the config, a plugin, a test, a library) makes it theirs.
// The whole stack, however deep. Returns the deciding frame, for the log.
const HELPERS = ['/node_modules/std-env/', '/node_modules/tinyexec/', '/node_modules/tinyrainbow/', '/node_modules/obug/', '/node_modules/debug/', '/node_modules/picocolors/', '/node_modules/supports-color/']
function toolCaller() {
  const limit = Error.stackTraceLimit
  Error.stackTraceLimit = Infinity
  let stack
  try {
    stack = new Error().stack ?? ''
  } finally {
    Error.stackTraceLimit = limit
  }
  // Every frame counts, not only the first: a test reading
  // `import.meta.env.X` goes through vitest's own getter, with the test's
  // frame below it.
  let tool = null
  for (const l of stack.split('\n').slice(1)) {
    if (l.includes(SELF) || l.includes('/.quod-vitest/') || l.includes('(node:') || l.includes(' node:') || l.includes('<anonymous>')) continue
    if (HELPERS.some((h) => l.includes(h)) || RESOLVERS.some((x) => l.includes(x))) {
      tool ??= l.trim()
      continue
    }
    return { tool: false, frame: l.trim() }
  }
  return tool ? { tool: true, frame: tool } : { tool: false, frame: '?' }
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

// `after(args, value)` runs once the call succeeded, before the caller sees
// its value: on the sync return, the promise's value, or the callback's.
function wrap(obj, name, before, after) {
  const f = obj?.[name]
  if (typeof f !== 'function' || f.__quod) return
  const w = function (...args) {
    const last = args.length - 1
    const cb = after && typeof args[last] === 'function' ? args[last] : null
    if (cb) {
      args[last] = function (err, v) {
        if (!err) {
          try {
            after(args, v)
          } catch {}
        }
        return cb.apply(this, arguments)
      }
    }
    try {
      before(args)
    } catch {}
    const res = new.target ? Reflect.construct(f, args, new.target) : f.apply(this, args)
    if (!after || cb) return res
    if (res && typeof res.then === 'function') {
      return res.then((v) => {
        try {
          after(args, v)
        } catch {}
        return v
      })
    }
    try {
      after(args, res)
    } catch {}
    return res
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
  const both = (names, fn, after) => {
    for (const n of names) {
      wrap(fs, n, fn, after)
      wrap(fs, `${n}Sync`, fn, after)
      wrap(P, n, fn, after)
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
      'writeFile', 'appendFile', 'unlink', 'truncate',
      'utimes', 'lutimes', 'chmod', 'lchmod', 'chown', 'lchown',
    ],
    w0,
  )
  wrap(fs, 'createWriteStream', w0)
  // A directory the file creates is its own (see `made`): a plain mkdir
  // made its path, a recursive one returns the first it made (none if all
  // existed), mkdtemp returns the one it made.
  both(['mkdir'], w0, (a, v) => {
    const o = a[1]
    r.created(o && typeof o === 'object' && o.recursive ? v : a[0])
  })
  const temp = (a, v) => r.created(typeof v === 'string' ? v : v?.path)
  both(['mkdtemp'], w0, temp)
  wrap(fs, 'mkdtempDisposableSync', w0, temp)
  wrap(P, 'mkdtempDisposable', w0, temp)
  // Once removed or moved away (after the call: its own lstat of the
  // directory is still the file's), the directory is no longer its own.
  const gone = (a) => r.gone(a[0])
  both(['rm', 'rmdir'], w0, gone)
  const move = (a) => {
    r.add('f', a[0])
    r.write(a[0])
    r.write(a[1])
  }
  both(['rename'], move, gone)
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
    if (p && r.on && !(r.made.length && under(p))) r.watches.add(p)
  }
  wrap(fs, 'watch', watch)
  wrap(fs, 'watchFile', watch)
  wrap(P, 'watch', watch)
  try {
    nodeModule.syncBuiltinESMExports()
  } catch {}
}

// What a CI provider sets per branch, event, runner or job, and turbo per
// task run (TURBO_HASH). vitest and vite read the environment for
// themselves (they copy all of it to the workers, pick reporters, find the
// CI): those reads of these names are not inputs of the tests, so a
// recording made on a push replays on a pull request. Their copy of the
// whole environment is `envtool:`, keyed on what node and the tools behave
// on (vitest.rs `tool_env`). Any other code that reads a variable (the
// config, a plugin, a test, a library it loads) keys it. `CI` is not one of
// these: vitest's behaviour depends on it (allowOnly, snapshot updates).
export const ciVarying = (k) => typeof k === 'string' && (k === 'TURBO_HASH' || /^(GITHUB|RUNNER|ACTIONS)_/.test(k))
// What node and the tools behave on (vitest.rs `tool_env`): their reads of
// these are inputs; their reads of anything else (a copy of the whole
// environment, one name at a time) are not.
const TOOL_ENV = new Set(['CI', 'TEST', 'TZ', 'LANG', 'LANGUAGE', 'FORCE_COLOR', 'NO_COLOR', 'TERM', 'TMPDIR', 'HOME', 'PATH'])
export const toolEnv = (k) => TOOL_ENV.has(k) || /^(NODE_|VITE|LC_|DEBUG|UV_)/.test(k)

export function patchEnv(r) {
  const target = process.env
  const hidden = new Set((target.QUOD_VITEST_HIDE ?? '').split(',').filter(Boolean))
  const rec = (k) => {
    if (typeof k !== 'string') return
    if (ciVarying(k) || !toolEnv(k)) {
      const c = toolCaller()
      if (c.tool) return
      if (ciVarying(k) && !r.envKept.has(k)) r.envKept.set(k, c.frame)
    }
    r.keys.add(`env:${k}`)
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
      if (r.on) {
        const c = toolCaller()
        r.keys.add(c.tool ? 'envtool:' : 'envall:')
        if (!c.tool && !r.envKept.has('*')) r.envKept.set('*', c.frame)
      }
      // In a test file's worker, a listing of the environment leaves out
      // the per-run CI variables (QUOD_VITEST_HIDE: the commit, the run id,
      // the event file, ...): code that copies the whole environment (an
      // SDK's config loader) then never holds one, and the file can record.
      // A read by name still sees it and makes the file unrecordable. If a
      // listing without them changed a result, the shadow's comparison with
      // the critical path (which runs plain) quarantines the file.
      const all = Reflect.ownKeys(t)
      return r.mode === 'worker' && hidden.size ? all.filter((k) => !hidden.has(k)) : all
    },
    // Writes go to process.env itself. Without this trap an assignment
    // through the proxy becomes a defineProperty with a partial descriptor,
    // which process.env refuses ("only accepts a configurable, writable, and
    // enumerable data descriptor"): vi.stubEnv threw in every recorded file
    // (class-management's backend; the shadow quarantined them).
    set(t, k, v) {
      return Reflect.set(t, k, v)
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
