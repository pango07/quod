// quod vitest: the recorder. Shared by the driver (vitest's main process)
// and the setup file (each test file's worker). It records, by wrapping
// node's fs, env, module loader and process/network entry points, every
// input a process reads, as dependency keys the Rust side hashes:
//
//   f:<path>   a file's content (or that it is absent or a directory)
//   pj:<path>  a workspace package.json, with dependency version values
//              blanked (the installed versions are p: keys)
//   pjf:<path>\0<fields>  a package.json the config parsed: its top-level
//              names, and the values of the fields it read (JSON array)
//   d:<path>   a directory's listing        r:<path>  a recursive listing
//   dj:<path>  a listing's subdirectories and .json files (tsconfck's walk)
//   e:<path>   that a path exists, its type, and a symlink's target
//   p:<path>   an installed package (its root and package.json)
//   env:<KEY>  one environment variable     envall:   all of them (but the
//                                                     per-run CI ones)
//   envtool:   the variables node and the tools behave on (vitest.rs tool_env)
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
  // (kind, absolute path) pairs whose key is in: a repeated read adds
  // nothing. Marked only once added (a path under a made directory, a
  // relative path, a main-process package.json are worked out every time).
  const seen = new Set()
  const r = {
    mode, // 'main' | 'worker'
    keys,
    writes: new Set(),
    watches: new Set(),
    // package.json files kept whole in the main process, and who read them
    pjWhole: new Map(),
    // CI-provider variables (and `*`: all of them) keyed by a read outside the tools, and who read them
    envKept: new Map(),
    // The environment copied for a known child process (esbuild's service), by who spawned it
    envChild: new Map(),
    // main process, package.json files read whole: path -> reads, and the
    // reads parsed straight into an object whose fields are watched (pjf:)
    pjReads: new Map(),
    pjParsed: new Map(),
    pjUse: new Map(),
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
      let memo = null
      if (typeof p === 'string' && p.charCodeAt(0) === 47 && !(r.mode === 'main' && p.endsWith('package.json'))) {
        memo = `${kind}\0${p}`
        if (seen.has(memo)) return
      }
      p = norm(p)
      if (p == null || p.startsWith(stateDir) || p.startsWith(codeDir)) return
      if (r.made.length && under(p)) return
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
          if (memo) seen.add(memo)
          keys.add(`p:${pkg}`)
          return
        }
        keys.add(`e:${pkg}`)
        p = ws + p.slice(pkg.length)
        if (p.includes('/node_modules/')) return r.add(kind, p)
      }
      if (kind === 'f' && r.mode === 'main' && p.endsWith('/package.json')) {
        if (resolverCaller()) kind = 'pj'
        else {
          if (!r.pjWhole.has(p)) r.pjWhole.set(p, callerFrame())
          r.pjReads.set(p, (r.pjReads.get(p) ?? 0) + 1)
        }
      }
      // tsconfck (vite-tsconfig-paths) walks the workspace for tsconfig
      // files: it descends into subdirectories and keeps files by name, so
      // what it sees of a listing is those (dj:), not every file (a new
      // changeset, a build's output chunks).
      if (kind === 'd' && r.mode === 'main' && callerFrame().includes('/node_modules/tsconfck/')) kind = 'dj'
      // a dj: read is worked out every time (the same path's d: may follow)
      if (memo && kind !== 'dj') seen.add(memo)
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
// Frames that say nothing of who reads: Quod's own, node's, and those with
// no file (a builtin: `Array.map (<anonymous>)`, `async Promise.allSettled
// (index 0)`). Not a CommonJS module's top level, which V8 names
// `Object.<anonymous> (/path/file.js:1:2)`: skipping that hid the module
// reading (a library copying the environment showed as
// `async Promise.allSettled`) and could take its read for the tools'.
function skipFrame(l) {
  return l.includes(SELF) || l.includes('/.quod-vitest/') || l.includes('(node:') || l.includes(' node:') || !l.includes('/')
}
const RESOLVERS = ['/node_modules/vite/', '/node_modules/vitest/', '/node_modules/@vitest/', '/node_modules/rolldown/', '/node_modules/@rolldown/']
export function resolverCaller(stack = new Error().stack) {
  const lines = (stack ?? '').split('\n').slice(1)
  for (const l of lines) {
    if (skipFrame(l)) continue
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
  let spawn = false
  for (const l of stack.split('\n').slice(1)) {
    if (l.includes('(node:child_process:') || l.includes('(node:internal/child_process:')) spawn = true
    if (skipFrame(l)) continue
    // node copying the environment for a child a known program spawned
    // (CHILDREN): the child's input, not this process's.
    if (spawn && !tool && CHILDREN.some((c) => l.includes(c))) return { tool: true, child: true, frame: l.trim() }
    if (HELPERS.some((h) => l.includes(h)) || RESOLVERS.some((x) => l.includes(x))) {
      tool ??= l.trim()
      continue
    }
    return { tool: false, frame: l.trim() }
  }
  return tool ? { tool: true, frame: tool } : { tool: false, frame: '?' }
}

// Programs whose child process's reads of the environment are known.
// esbuild's JS API spawns its Go service (and its sync API runs the same
// binary) with node's copy of the whole environment, which lists and reads
// every variable: in vitest's main process that made every variable, the
// CI's per-branch and per-event ones and the package manager's
// (npm_package_version) among them, an input of every file, so a pull
// request's check replayed nothing a push recorded. The service reads none
// of them: what node and the tools behave on (envtool:), Go's runtime
// settings and esbuild's own (CHILD_ENV).
const CHILDREN = ['/node_modules/esbuild/', '/node_modules/@esbuild/']
const CHILD_ENV = /^(GOGC|GOMAXPROCS|GODEBUG|GOMEMLIMIT|GOTRACEBACK|GORACE|ESBUILD_.*)$/

function callerFrame(stack = new Error().stack) {
  for (const l of (stack ?? '').split('\n').slice(1)) {
    if (skipFrame(l)) continue
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

// `sync: false` leaves node:fs's ESM exports to the caller's one
// syncBuiltinESMExports (each call walks every loaded builtin's exports).
export function patchFs(r, { sync = true } = {}) {
  const P = fs.promises
  const both = (names, fn, after) => {
    for (const n of names) {
      wrap(fs, n, fn, after)
      wrap(fs, `${n}Sync`, fn, after)
      wrap(P, n, fn, after)
    }
  }
  const file = (a) => r.add('f', a[0])
  both(['readFile'], file, r.mode === 'main' ? (a, v) => pjText(r, a[0], v) : undefined)
  if (r.mode === 'main') patchJsonParse(r)
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
  if (sync) syncESM()
}

export function syncESM() {
  try {
    nodeModule.syncBuiltinESMExports()
  } catch {}
}

// A package.json the main process reads whole (the config, a plugin) is
// keyed whole (f:), unless the read goes straight into JSON.parse, as in
// `JSON.parse(readFileSync(p, 'utf8'))`: then what the code can see of the
// file is the object, and the object's fields are watched (pjf:). A config
// that takes its dependency names from package.json (convex-helpers: one
// project per `zod-4.x` alias) then re-runs nothing for a version bump or
// a script change. Straight: a synchronous read whose text the very next
// JSON.parse call takes, from the same place in the same file, that call
// written before the read (so the read is its argument, not a variable
// that could be used again), without a reviver, giving an object. The
// object stays a plain object (a proxy cannot be structured-cloned, and
// vitest clones `provide`); its fields become accessors that note a read.
// Its names are always keyed (Object.keys, `in` and for...in see no
// accessor). Anything else keeps the file whole.
const pending = { text: null, buf: null, p: null, at: null }
function callerAt() {
  const f = callerFrame()
  const m = /^(?:at .*?\()?(?:at )?(.*):(\d+):(\d+)\)?$/.exec(f)
  return m ? { file: m[1], line: +m[2], col: +m[3] } : null
}
function pjText(r, p0, v) {
  pending.text = pending.buf = null
  if (!r.on || p0 == null) return
  const p = norm(p0)
  if (p == null || !p.endsWith('/package.json') || resolverCaller()) return
  const at = callerAt()
  if (!at) return
  if (typeof v === 'string') pending.text = v
  else if (v instanceof Uint8Array) pending.buf = v
  else return
  pending.p = p
  pending.at = at
  // The very next parse, synchronously: nothing async comes in between.
  queueMicrotask(() => {
    pending.text = pending.buf = null
  })
}
function patchJsonParse(r) {
  const parse = JSON.parse
  if (parse.__quod) return
  const w = {
    parse(text, reviver) {
      const v = Reflect.apply(parse, this, arguments)
      if (pending.text === null && pending.buf === null) return v
      const hit = typeof text === 'string' ? text === pending.text : text === pending.buf
      const p = pending.p
      const was = pending.at
      pending.text = pending.buf = null
      if (!hit || reviver !== undefined || !r.on || v === null || typeof v !== 'object' || Array.isArray(v)) return v
      const at = callerAt()
      if (!at || at.file !== was.file || was.line - at.line > 2 || !(at.line < was.line || (at.line === was.line && at.col < was.col))) return v
      return watchFields(r, p, v)
    },
  }.parse
  Object.defineProperty(w, '__quod', { value: true })
  JSON.parse = w
}
function watchFields(r, p, obj) {
  let use = r.pjUse.get(p)
  if (!use) r.pjUse.set(p, (use = new Set()))
  r.pjParsed.set(p, (r.pjParsed.get(p) ?? 0) + 1)
  for (const k of Object.keys(obj)) {
    const val = obj[k]
    Object.defineProperty(obj, k, {
      enumerable: true,
      configurable: true,
      get() {
        use.add(k)
        return val
      },
      set(x) {
        Object.defineProperty(obj, k, { value: x, writable: true, enumerable: true, configurable: true })
      },
    })
  }
  return obj
}
// The main process's keys, with each package.json every whole read of
// which was parsed and watched keyed by what was read of it (pjf:).
export function settledKeys(r) {
  const keys = new Set(r.keys)
  for (const [p, n] of r.pjReads) {
    if (r.pjParsed.get(p) !== n || !keys.has(`f:${p}`)) continue
    keys.delete(`f:${p}`)
    keys.add(`pjf:${p}\0${JSON.stringify([...r.pjUse.get(p)].sort())}`)
    r.pjWhole.delete(p)
  }
  return keys
}

// What a CI provider sets per branch, event, runner or job, and turbo and
// Nx per task run (TURBO_HASH, NX_TASK_HASH, ...). vitest and vite read the environment for
// themselves (they copy all of it to the workers, pick reporters, find the
// CI): those reads of these names are not inputs of the tests, so a
// recording made on a push replays on a pull request. Their copy of the
// whole environment is `envtool:`, keyed on what node and the tools behave
// on (vitest.rs `tool_env`). Any other code that reads a variable (the
// config, a plugin, a test, a library it loads) keys it. `CI` is not one of
// these: vitest's behaviour depends on it (allowOnly, snapshot updates).
export const ciVarying = (k) => typeof k === 'string' && (k === 'TURBO_HASH' || /^(GITHUB|RUNNER|ACTIONS)_/.test(k) || /^NX_(TASK_HASH|ANALYTICS_SESSION_ID|TERMINAL_OUTPUT_PATH)$/.test(k))
// What node and the tools behave on (vitest.rs `tool_env`): their reads of
// these are inputs; their reads of anything else (a copy of the whole
// environment, one name at a time) are not.
const TOOL_ENV = new Set(['CI', 'TEST', 'TZ', 'LANG', 'LANGUAGE', 'FORCE_COLOR', 'NO_COLOR', 'TERM', 'TMPDIR', 'HOME', 'PATH'])
export const toolEnv = (k) => TOOL_ENV.has(k) || /^(NODE_|VITE|LC_|DEBUG|UV_)/.test(k)

export function patchEnv(r) {
  const target = process.env
  const hidden = new Set((target.QUOD_VITEST_HIDE ?? '').split(',').filter(Boolean))
  // names whose env: key is in (a repeated read adds nothing, and walks no stack)
  const seen = new Set()
  const rec = (k) => {
    if (typeof k !== 'string' || seen.has(k)) return
    if (ciVarying(k) || !toolEnv(k)) {
      const c = toolCaller()
      if (c.child && CHILD_ENV.test(k)) r.keys.add(`env:${k}`)
      if (c.tool) return
      if (ciVarying(k) && !r.envKept.has(k)) r.envKept.set(k, c.frame)
    }
    seen.add(k)
    r.keys.add(`env:${k}`)
  }
  // A descriptor asked for by name is the variable's presence (envp:), not
  // its value: Object.keys and for...in on a proxy ask for every name's
  // descriptor only to see it is enumerable (the `debug` package lists the
  // environment that way in vite's plugins; by value that keyed the commit
  // and the run id for every file, so nothing replayed from one CI job to
  // the next). A copy ({ ...env }, JSON.stringify) reads each value through
  // `get`, keyed by value as before; so does Object.entries/values.
  const seenP = new Set()
  const recPresence = (k) => {
    if (typeof k !== 'string' || seen.has(k) || seenP.has(k)) return
    if (ciVarying(k) || !toolEnv(k)) {
      if (toolCaller().tool) return
    }
    seenP.add(k)
    r.keys.add(`envp:${k}`)
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
      if (r.on) recPresence(k)
      return Reflect.getOwnPropertyDescriptor(t, k)
    },
    ownKeys(t) {
      if (r.on) {
        const c = toolCaller()
        r.keys.add(c.tool ? 'envtool:' : 'envall:')
        if (!c.tool && !r.envKept.has('*')) r.envKept.set('*', c.frame)
        if (c.child) {
          r.envChild.set(c.frame, (r.envChild.get(c.frame) ?? 0) + 1)
          // Go's and esbuild's settings, set or not
          for (const k of ['GOGC', 'GOMAXPROCS', 'GODEBUG', 'GOMEMLIMIT', 'GOTRACEBACK', 'GORACE']) r.keys.add(`env:${k}`)
          for (const k of Reflect.ownKeys(t)) if (typeof k === 'string' && CHILD_ENV.test(k)) r.keys.add(`env:${k}`)
        }
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
export function patchEscapes(r, { sync = true } = {}) {
  const done = new Set()
  const patch = (name, now = true) => {
    if (done.has(name)) return
    done.add(name)
    try {
      ESCAPES[name](process.getBuiltinModule(name), r)
      if (now) nodeModule.syncBuiltinESMExports()
    } catch {
      r.unobs(`could not watch ${name}`)
    }
  }
  // net always (loading it if need be): every TCP connection made in JS
  // goes through net.Socket.prototype.connect, node's own too (undici's
  // fetch and WebSocket, http, tls), even from a module node loads
  // internally, past the resolve hook. So the WebSocket global is left
  // alone: reading it loads undici (~8 ms per worker).
  const loaded = new Set(process.moduleLoadList?.filter((m) => m.startsWith('NativeModule ')).map((m) => m.slice(13)))
  loaded.add('net')
  for (const name of Object.keys(ESCAPES)) if (loaded.has(name) || !process.moduleLoadList) patch(name, false)
  if (sync) syncESM()
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
}

// Write JSON atomically with the unrecorded fs.
export function writeJson(file, value) {
  const tmp = `${file}.${process.pid}.tmp`
  real.writeFileSync(tmp, JSON.stringify(value))
  real.renameSync(tmp, file)
}
