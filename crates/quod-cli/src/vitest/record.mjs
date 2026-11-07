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
//   pjq:<path>\0<queries>  values and proven truthiness reads of a manifest
//   st:<path>  a stat result proven to be used only as file/directory type
//   sm:<path>  every field of stat, including timestamps and permissions
//   sl:<path>  every field of lstat (the link's own metadata)
//   d:<path>   a directory's listing        r:<path>  a recursive listing
//   rj:<path>  the .json files under a directory, recursively, outside
//              node_modules and .git (tsconfck's walk for tsconfig files)
//   e:<path>   that a path exists, its type, and a symlink's target
//   rp:<path>  the complete bounded filesystem path-resolution footprint
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
import nodeUtil from 'node:util'
import { fileURLToPath, pathToFileURL } from 'node:url'

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

// 2: npm's allowance is by process (child.mjs r.npm), not by stack frame;
// a record made under -1 for a file importing ci-info lacks its env: keys.
export const VERSION = 'quod-vitest-2'

const NPM_HOME = process.env.HOME ? path.join(process.env.HOME, '.npm') : null
const NPM_OWN = NPM_HOME ? path.join(NPM_HOME, '_logs') : null
const SKIP_NM = ['/node_modules/.vite/', '/node_modules/.vitest/', '/node_modules/.cache/']

export function createRecorder({ stateDir, codeDir = stateDir, mode, wsRoot = null, namesProof = null, booleanProof = null, statProof = booleanProof }) {
  const keys = new Set()
  // A made directory's other spelling (its real path, and back), when they differ.
  const alias = new Map()
  // A path under a made directory is this process's own output, not an input.
  const under = (p) => r.made.some((m) => p === m || p.startsWith(`${m}/`))
  // (kind, absolute path) pairs whose key is in: a repeated read adds
  // nothing. Marked only once added (a path under a made directory, a
  // relative path, a main-process package.json are worked out every time).
  const seen = new Set()
  // vitest's own per-run directories (the driver's QUOD_VITEST_SKIP, one per
  // line): vitest 3 hands a worker each transformed module as a file in its
  // project's temp directory, a new one every run. What is in it is a
  // module's transform, keyed where it comes from (the module graph, the
  // config); the path itself is not an input.
  const skip = (process.env.QUOD_VITEST_SKIP ?? '').split('\n').filter(Boolean).map((d) => (d.endsWith('/') ? d : `${d}/`))
  const r = {
    mode, // 'main' | 'worker'
    namesProof,
    booleanProof,
    statProof,
    keys,
    writes: new Set(),
    // paths first written while absent (scratch() drops those gone again)
    fresh: new Set(),
    watches: new Set(),
    // child processes traced (traceChild): the dump each must leave
    children: [],
    // set to trace node child processes instead of giving up on them
    trace: null,
    // package.json files kept whole in the main process, and who read them
    pjWhole: new Map(),
    // CI-provider variables (and `*`: all of them) keyed by a read outside the tools, and who read them
    envKept: new Map(),
    // this process is npm or npx itself (child.mjs, by argv[1]): its reads
    // of the CI's per-run variables are presence-keyed (envp:)
    npm: false,
    // The environment copied for a known child process (esbuild's service), by who spawned it
    envChild: new Map(),
    // main process, package.json files read whole: path -> reads, and the
    // reads parsed straight into an object whose fields are watched (pjf:)
    pjReads: new Map(),
    pjParsed: new Map(),
    pjUse: new Map(),
    pjBool: new Map(),
    // main process: installed packages read only by vite's or vitest's
    // resolvers (resolving a worker's import): the worker that loads one
    // keys it (its module graph), so it is a shared input only when no
    // worker of the run has it (the driver decides; vitest.md 3.2).
    resolved: new Set(),
    // main process: where tsconfck's walks started (rj: keys)
    rjRoots: [],
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
      if (kind === 'rp') {
        // Preserve '..' until filesystem resolution: resolving it here can
        // erase a symlink that the native realpath operation traverses.
        // Never collapse these queries to an installed package's version.
        if (process.platform !== 'linux' || typeof p !== 'string' || p.includes('\0')) {
          r.unobs('unsupported realpath argument');return
        }
        keys.add(`rp:${p.startsWith('/') ? p : `${process.cwd()}/${p}`}`)
        return
      }
      let memo = null
      if (typeof p === 'string' && p.charCodeAt(0) === 47 && !(r.mode === 'main' && p.endsWith('package.json'))) {
        memo = `${kind}\0${p}`
        if (seen.has(memo)) return
      }
      p = norm(p)
      if (p == null || p.startsWith(stateDir) || p.startsWith(codeDir)) return
      // Optional diagnostic observer. It cannot suppress a dependency if
      // collecting diagnostic evidence itself fails.
      try { r.observeRead?.(kind, p) } catch {}
      if (r.made.length && under(p)) return
      if (skip.length && skip.some((d) => p.startsWith(d))) return
      if (p.startsWith('/proc/') || p.startsWith('/dev/') || p.startsWith('/sys/')) return
      // vitest 5's API token (the UI/API server's auth: a random id it
      // creates under the user data dir the first time, so new on every
      // fresh CI runner). vitest's main process reads it; no test result
      // depends on it. Keyed, it made every real job a miss.
      if (r.mode === 'main' && p.endsWith('/vitest/.vitest-secret-token')) return
      // npm's own bookkeeping (a traced npx): its debug logs, which it lists
      // to prune, and the update notifier's stamp. New every run; no
      // result depends on them.
      if (NPM_OWN && (p === NPM_OWN || p.startsWith(`${NPM_OWN}/`) || p === `${NPM_HOME}/_update-notifier-last-checked`)) return
      // An access result depends on permissions/ACLs as well as content.
      // Keep it at the actual path even inside an installed package.
      if (/^a[0-7]$/.test(kind) || kind === 'st' || kind === 'sm' || kind === 'sl') {
        if (memo) seen.add(memo)
        keys.add(`${kind}:${p}`)
        return
      }
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
          if (r.mode === 'main' && !keys.has(`p:${pkg}`) && resolverCaller()) {
            r.resolved.add(pkg)
            return
          }
          if (memo) seen.add(memo)
          keys.add(`p:${pkg}`)
          return
        }
        keys.add(`e:${pkg}`)
        p = ws + p.slice(pkg.length)
        if (p.includes('/node_modules/')) return r.add(kind, p)
      }
      if (kind === 'f' && r.mode === 'main' && p.endsWith('/package.json')) {
        if (resolverCaller()) {
          kind = 'pj'
          // A resolver's broad key can be refined only if every read was
          // observed through the same straight JSON parse as other readers.
          if (r.booleanProof) r.pjReads.set(p, (r.pjReads.get(p) ?? 0) + 1)
        }
        else {
          if (!r.pjWhole.has(p)) r.pjWhole.set(p, callerFrame())
          r.pjReads.set(p, (r.pjReads.get(p) ?? 0) + 1)
        }
      }
      // tsconfck (vite-tsconfig-paths) walks the workspace for tsconfig
      // files: what it finds is the tsconfig files under where it started
      // (it descends into every directory but node_modules and .git, not
      // through links, and keeps files by name). Keyed as the .json files
      // under the walk's start (rj:), not each listing: a new changeset, a
      // build's output (dist/ there or not) change nothing it finds.
      if (kind === 'd' && r.mode === 'main' && callerFrame().includes('/node_modules/tsconfck/')) {
        if (r.rjRoots.some((d) => p === d || p.startsWith(`${d}/`))) return
        r.rjRoots.push(p)
        kind = 'rj'
      }
      // an rj: read is worked out every time (the same path's d: may follow)
      if (memo && kind !== 'rj') seen.add(memo)
      keys.add(`${kind}:${p}`)
    },
    write(p) {
      if (!r.on || p == null) return
      p = norm(p)
      if (p == null || p.startsWith(stateDir) || p.startsWith(codeDir)) return
      // A path that did not exist at its first write: if it is gone again
      // at the end, it was the process's scratch file (scratch()).
      if (!r.writes.has(p) && !real.existsSync(p)) r.fresh.add(p)
      r.writes.add(p)
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
// npm and its CI detection, in a node child a test starts (`npx redocly`):
// its reads of the CI's per-run variables are keyed by presence (envp:),
// never by value, and its copies of the whole environment for the program
// it runs are a package manager's (envtool:). Only in a process that IS npm
// or npx (r.npm, set by child.mjs from argv[1]), never by stack frame alone:
// ci-info is an ordinary dependency, and a test file importing it evaluates
// it with only ci-info's frames on the stack; its `isPR` is
// GITHUB_EVENT_NAME's value, so it must be keyed by value there (a record
// made on a push replayed on a pull request otherwise). npm's own copy of
// ci-info is under /node_modules/npm/. npm 10/11 behave on ci-info's isCI
// and name (GITHUB_ACTIONS' presence) and on CI (keyed by value for every
// caller), never isPR. Any other variable npm reads (npm_config_*) is
// keyed as usual, and the program it runs, traced, records its own reads.
//
// A traced child's env: keys are hashed in quod vitest's own environment,
// so a name only npm sets for its child (npm_lifecycle_event,
// npm_config_user_agent, INIT_CWD) hashes "unset" at record and replay
// alike. Each such value derives from keyed inputs (the spawning test's
// code, package.json, npm's files as f:, node's version); not a hole, and
// not to be "fixed" into a per-run key.
const NPM_HELPERS = ['/node_modules/npm/', '/node_modules/@npmcli/']
// Formatting Error.stack invokes Vite's source-map formatter. Repeated
// environment reads at the same complete call stack need the same caller
// attribution. Cache that decision, not environment values or input keys.
// The native frame ids distinguish different evaluated scripts at the
// same filename. A changed formatter, unknown API, or truncated stack
// always takes the original full-stack path.
const getCallSites = nodeUtil.getCallSites
const callerCacheOn = process.env.QUOD_VITEST_CALLER_CACHE !== '0'
const callerCache = new Map()
const enqueueCallerReset = globalThis.queueMicrotask
let callerResetPending = false
let callerFormatter
function callerCacheKey(npm) {
  if (!callerCacheOn || typeof getCallSites !== 'function') return null
  try {
    const formatter = Error.prepareStackTrace
    if (formatter !== callerFormatter) { callerCache.clear(); callerFormatter = formatter }
    const frames = getCallSites(200, { sourceMap: false })
    if (!frames.length || frames.length >= 200 || frames.some(f => typeof f.scriptId !== 'string')) return null
    return `${npm}:` + frames.map(f => `${f.scriptId}:${f.scriptName}:${f.functionName}:${f.lineNumber}:${f.columnNumber}`).join('\n')
  } catch { return null }
}
function toolCaller(npm = false) {
  const key = callerCacheKey(npm)
  if (key != null && callerCache.has(key)) return callerCache.get(key)
  const result = toolCallerUncached(npm)
  if (key != null) {
    if (callerCache.size >= 1024) callerCache.clear()
    callerCache.set(key, result)
    // Native call sites need not include async causal parents. Never
    // carry an attribution across a microtask boundary, where those
    // parents or Vite's source-map registrations may have changed.
    if (!callerResetPending) {
      callerResetPending = true
      enqueueCallerReset(() => { callerCache.clear(); callerResetPending = false })
    }
  }
  return result
}
function toolCallerUncached(npm = false) {
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
    if (HELPERS.some((h) => l.includes(h)) || RESOLVERS.some((x) => l.includes(x)) || (npm && NPM_HELPERS.some((h) => l.includes(h)))) {
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
  // Content equality does not imply equal Stats: mode, inode and all four
  // timestamps can change without changing bytes. Keep the native return
  // object untouched and key the whole observation unless a source proof
  // establishes that only its file/directory type is consumed.
  wrap(fs, 'stat', (a) => r.add('sm', a[0]))
  wrap(P, 'stat', (a) => r.add('sm', a[0]))
  const nativeStatSync = fs.statSync
  wrap(fs, 'statSync', (a) => r.add(r.mode === 'main' && r.statProof?.statType(a, nativeStatSync) ? 'st' : 'sm', a[0]))
  both(['lstat'], (a) => r.add('sl', a[0]))
  // A descriptor's metadata is not represented by the content dependency
  // recorded at open(). The descriptor may also refer to an unlinked file,
  // a reused number, or a pipe, so a later pathname lookup is not a key.
  for (const name of ['fstat', 'fstatSync']) {
    wrap(fs, name, () => r.unobs('descriptor metadata is not tracked'))
  }
  both(['access'], (a) => {
    const mode = a[1] ?? 0
    if (Number.isInteger(mode) && mode >= 0 && mode <= 7) r.add(`a${mode}`, a[0])
    else r.unobs('access with an unsupported mode')
  })
  wrap(fs, 'existsSync', (a) => r.add('e', a[0]))
  wrap(fs, 'exists', (a) => r.add('e', a[0]))
  both(['readlink'], (a) => r.add('e', a[0]))
  both(['realpath'], (a) => r.add('rp', a[0]))
  wrap(fs.realpathSync, 'native', (a) => r.add('rp', a[0]))
  wrap(fs.realpath, 'native', (a) => r.add('rp', a[0]))
  const dir = (a) => {
    const o = a[1]
    r.add(o && typeof o === 'object' && o.recursive ? 'r' : 'd', a[0])
  }
  both(['readdir', 'opendir'], dir)
  const open = (a) => {
    if (isRead(a[1])) r.add('f', a[0])
    if (isWrite(a[1])) r.write(a[0])
  }
  wrap(fs, 'open', open)
  wrap(fs, 'openSync', open)
  // FileHandle methods call internal bindings directly: wrapping fs.fstat
  // does not cover handle.stat(), nor descriptor writes through that object.
  // Until the handle's complete lifetime/effects are observed, run it plain.
  wrap(P, 'open', (a) => {
    open(a)
    r.unobs('FileHandle operations are not tracked')
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
  if (p == null || !p.endsWith('/package.json')) return
  if (resolverCaller() && (!r.booleanProof || !r.pjReads.has(p))) return
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
// Capture these before loading application configuration. The proof cannot
// call application replacements while a spread is in progress.
const fieldKeys = Object.keys
const fieldOwnKeys = Reflect.ownKeys
const fieldDescriptor = Object.getOwnPropertyDescriptor
const fieldPrototype = Object.getPrototypeOf
const fieldObjectPrototype = Object.prototype
const fieldDefine = Object.defineProperty
const fieldStringify = JSON.stringify
const fieldSetAdd = Function.prototype.call.bind(Set.prototype.add)
const dependencyFields = ['dependencies', 'devDependencies', 'peerDependencies', 'optionalDependencies']
function watchFields(r, p, obj) {
  let use = r.pjUse.get(p)
  if (!use) r.pjUse.set(p, (use = new Set()))
  r.pjParsed.set(p, (r.pjParsed.get(p) ?? 0) + 1)
  const binding = resolverCaller() ? null : r.namesProof?.bind()
  const shape = binding ? new Map() : null
  function watch(target, path) {
    for (const k of fieldKeys(target)) {
      const val = target[k]
      const next = [...path, k]
      const encoded = fieldStringify(next)
      const dep = path.length === 0 && dependencyFields.includes(k) && val && typeof val === 'object' && !Array.isArray(val)
      const primitive = binding && dep && fieldOwnKeys(val).every(key => {
        const d = fieldDescriptor(val, key)
        return d && 'value' in d && (d.value === null || typeof d.value !== 'object' && typeof d.value !== 'function')
      })
      if (dep) watch(val, next)
      const get = () => {
        if (next.length === 1 && k === 'exports' && r.booleanProof?.test(k)) {
          let truthy = r.pjBool.get(p)
          if (!truthy) r.pjBool.set(p, (truthy = new Set()))
          fieldSetAdd(truthy, k)
        } else if (!binding || !r.namesProof.ignores(binding, next, pristine)) fieldSetAdd(use, encoded)
        return val
      }
      fieldDefine(target, k, {
        enumerable: true,
        configurable: true,
        get,
        set(x) {
          fieldDefine(target, k, { value: x, writable: true, enumerable: true, configurable: true })
        },
      })
      if (binding && dep) {
        const leaves = fieldOwnKeys(val).map(key => [key, fieldDescriptor(val, key)?.get])
        // JSON.parse without a reviver yields data; nested objects may have
        // coercion behavior and are outside this bounded proof.
        shape.set(k, { get, val, leaves, primitive })
      }
    }
  }
  function pristine(fields) {
    for (const field of fields) {
      const s = shape.get(field)
      // Spreading an absent field is harmless only when no inherited
      // property/getter can supply it. Root property names are always keyed.
      if (!s && !fieldDescriptor(obj, field) && fieldPrototype(obj) === fieldObjectPrototype &&
          !fieldDescriptor(fieldObjectPrototype, field) && fieldPrototype(fieldObjectPrototype) === null) continue
      if (!s?.primitive || fieldDescriptor(obj, field)?.get !== s.get) return false
      const keys = fieldOwnKeys(s.val)
      if (keys.length !== s.leaves.length) return false
      for (let i = 0; i < keys.length; i++) {
        const [key, get] = s.leaves[i]
        const d = fieldDescriptor(s.val, key)
        if (keys[i] !== key || !get || d?.get !== get || !d.enumerable) return false
      }
    }
    return true
  }
  watch(obj, [])
  // After this check the certified next operation is reading the native
  // keys callee. No application code or asynchronous turn intervenes.
  r.namesProof?.seal(binding, pristine)
  return obj
}
// The main process's keys, with each package.json every whole read of
// which was parsed and watched keyed by what was read of it (pjf:).
export function settledKeys(r) {
  const keys = new Set(r.keys)
  for (const [p, n] of r.pjReads) {
    if (r.pjParsed.get(p) !== n || !(keys.has(`f:${p}`) || r.booleanProof && keys.has(`pj:${p}`))) continue
    keys.delete(`f:${p}`)
    const values = [...r.pjUse.get(p)].sort().map((s) => JSON.parse(s))
    if (r.booleanProof) {
      keys.delete(`pj:${p}`)
      keys.add(`pjq:${p}\0${JSON.stringify({values, truthy:[...(r.pjBool.get(p) ?? [])].sort()})}`)
    } else keys.add(`pjf:${p}\0${JSON.stringify(values)}`)
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
      const npm = r.npm && ciVarying(k)
      const c = toolCaller(npm)
      if (c.child && CHILD_ENV.test(k)) r.keys.add(`env:${k}`)
      if (c.tool) {
        // npm's read: set or unset, never the value (B2 1b)
        if (npm) r.keys.add(`envp:${k}`)
        return
      }
      if (ciVarying(k) && !r.envKept.has(k)) r.envKept.set(k, c.frame)
    }
    seen.add(k)
    r.keys.add(`env:${k}`)
  }
  // A descriptor contains the environment value. Object.keys may discard
  // it, but Object.getOwnPropertyDescriptor and Reflect's equivalent can
  // return it to application code without ever invoking this proxy's get
  // trap. Until the consumer is proved presence-only, keep the value key.
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
        const c = toolCaller(r.npm)
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
// The egress-denied witness (vitest.rs, deny.mjs): with it, using the
// network is noted (the record needs a second witness with no network)
// instead of making the file unobservable. Read before the env is watched.
const NET_WITNESS = process.env.QUOD_VITEST_NET_WITNESS === '1'
const netUse = (r) => (NET_WITNESS ? (r.net = true) : r.unobs('network'))
const ESCAPES = {
  child_process: (m, r) => {
    for (const n of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) {
      wrap(m, n, (a) => (r.trace ? traceChild(r, n, a) : r.unobs('child_process')))
    }
  },
  net: (m, r) => {
    wrap(m, 'connect', () => netUse(r))
    wrap(m, 'createConnection', () => netUse(r))
    wrap(m.Socket?.prototype, 'connect', () => netUse(r))
  },
  tls: (m, r) => wrap(m, 'connect', () => netUse(r)),
  http: (m, r) => ['request', 'get'].forEach((n) => wrap(m, n, () => netUse(r))),
  https: (m, r) => ['request', 'get'].forEach((n) => wrap(m, n, () => netUse(r))),
  http2: (m, r) => wrap(m, 'connect', () => netUse(r)),
  dgram: (m, r) => wrap(m, 'createSocket', () => netUse(r)),
  dns: (m, r) => ['lookup', 'resolve'].forEach((n) => wrap(m, n, () => netUse(r))),
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
  wrap(globalThis, 'fetch', () => netUse(r))
}

// The environment itself (not process.env, which the recorder replaces
// with a watching proxy): what a traced child is handed.
const realEnv = process.env

// The writes that remain: a path the process created and removed again
// (a test writing a spec file for a child to lint, then deleting it) was
// its own scratch, not an output.
export function scratch(r) {
  return [...r.writes].filter((p) => !(r.fresh.has(p) && !real.existsSync(p)))
}

// A child process, traced instead of making the file unobservable when it
// is node (a node binary, Quod's node shim, or a script whose #! line runs
// node), started directly or by a shell from a simple command (words and
// quotes only: no expansion, redirection or chaining). What decides which
// program runs is keyed: the shell's binary, PATH and each directory
// tried, the program file. The child runs with child.mjs preloaded
// (NODE_OPTIONS), which records what it reads, as this process does, into
// a dump of its own that this file's record takes in (setup.mjs). Anything
// else (a native program, a shell line Quod cannot read) is unobservable,
// as before.
const SHELL_SAFE = /^[A-Za-z0-9_@%+=:,./'"\s-]*$/
function words(cmd) {
  if (!SHELL_SAFE.test(cmd)) return null
  const out = []
  let cur = null
  for (let i = 0; i < cmd.length; i++) {
    const c = cmd[i]
    if (c === "'" || c === '"') {
      const j = cmd.indexOf(c, i + 1)
      if (j < 0) return null
      cur = (cur ?? '') + cmd.slice(i + 1, j)
      i = j
    } else if (c === '\\') return null
    else if (/\s/.test(c)) {
      if (cur != null) out.push(cur)
      cur = null
    } else cur = (cur ?? '') + c
  }
  if (cur != null) out.push(cur)
  return out.length ? out : null
}
function resolveProgram(r, prog, env, cwd) {
  if (prog.includes('/')) {
    const p = path.resolve(cwd, prog)
    r.add('e', p)
    return real.existsSync(p) ? p : null
  }
  for (const d of String(env.PATH ?? '').split(':')) {
    if (!d) continue
    const p = path.join(path.resolve(cwd, d), prog)
    r.add('e', p)
    try {
      fs.accessSync(p, fs.constants.X_OK)
      if (!fs.statSync(p).isDirectory()) return p
    } catch {}
  }
  return null
}
function isNode(r, file, env, cwd) {
  let rp
  try {
    rp = real.realpathSync(file)
  } catch {
    return false
  }
  r.add('f', rp)
  if (rp === real.realpathSync(process.execPath)) return true
  let head = ''
  try {
    head = real.readFileSync(rp, 'utf8').slice(0, 512)
  } catch {
    return false
  }
  if (head.includes("Quod's node shim")) return true
  const m = /^#!\s*(\S+)(?:[ \t]+(.*))?/.exec(head)
  if (!m) return false
  const [interp, rest = ''] = [m[1], m[2]?.trim()]
  if (interp.endsWith('/node')) return true
  if (interp.endsWith('/env')) {
    r.add('f', interp)
    const args = rest.split(/\s+/).filter((x) => x && x !== '-S')
    if (args[0] !== 'node') return false
    const n = resolveProgram(r, 'node', env, cwd)
    return n != null && isNode(r, n, env, cwd)
  }
  return false
}
function traceChild(r, name, a) {
  if (!r.on) return
  const fail = (why) => r.unobs(`child_process${why ? `: ${why}` : ''}`)
  const shellCall = name === 'exec' || name === 'execSync'
  let oi = shellCall ? 1 : Array.isArray(a[1]) ? 2 : 1
  if (a[oi] != null && (typeof a[oi] !== 'object' || Array.isArray(a[oi]))) {
    if (typeof a[oi] === 'function') a.splice(oi, 0, undefined)
    else return fail('unusual arguments')
  }
  const opts = a[oi] ?? {}
  const env = opts.env ?? realEnv
  // what this process passes on: PATH decides the program
  const PATH = opts.env ? opts.env.PATH : process.env.PATH
  const cwd = opts.cwd ? path.resolve(String(opts.cwd)) : process.cwd()
  let prog
  if (name === 'fork') prog = opts.execPath ?? process.execPath
  else {
    let argv
    let shell = shellCall ? (typeof opts.shell === 'string' ? opts.shell : '/bin/sh') : opts.shell ? (typeof opts.shell === 'string' ? opts.shell : '/bin/sh') : null
    let cmd = shell ? (shellCall ? String(a[0]) : [a[0], ...(Array.isArray(a[1]) ? a[1] : [])].join(' ')) : null
    // `sh -c <line>` spelled out (npm runs scripts and bins so)
    const args = Array.isArray(a[1]) ? a[1] : []
    if (!shell && /^(\/(usr\/)?bin\/)?(sh|bash|dash)$/.test(String(a[0])) && args[0] === '-c' && args.length === 2) {
      shell = String(a[0])
      cmd = String(args[1])
    }
    if (shell) {
      argv = words(cmd)
      if (!argv) return fail('a shell line Quod cannot read')
      const sh = shell.includes('/') ? shell : resolveProgram(r, shell, { PATH }, cwd)
      if (!sh) return fail('no shell')
      try {
        r.add('f', real.realpathSync(sh))
      } catch {
        return fail('no shell')
      }
    } else argv = [String(a[0])]
    prog = resolveProgram(r, argv[0], { PATH }, cwd)
    if (!prog) return fail(`${argv[0]} not found`)
  }
  if (!isNode(r, prog, { PATH }, cwd)) return fail(`${path.basename(prog)} is not node`)
  const t = r.trace
  const code = path.join(t.codeDir, 'child.mjs')
  if (/\s/.test(code)) return fail('a space in the path of child.mjs')
  const out = `${t.base}.c${t.n++}.json`
  r.children.push(out)
  const add = `--import=${pathToFileURL(code).href}`
  a[oi] = { ...opts, env: { ...env, NODE_OPTIONS: env.NODE_OPTIONS ? `${env.NODE_OPTIONS} ${add}` : add, QUOD_VITEST_CHILD: out } }
}

// A traced child's dump and its own children's, merged into r; false when
// one is missing (a child still running or killed: nothing proves what it
// read).
export function takeChildren(r, files = r.children, depth = 0) {
  for (const f of files) {
    let d
    try {
      d = JSON.parse(real.readFileSync(f, 'utf8'))
    } catch {
      r.unobservable.add('child_process: a child left no record (still running, or killed)')
      return false
    }
    for (const k of d.keys) r.keys.add(k)
    for (const w of d.writes) r.writes.add(w)
    for (const u of d.unobservable) r.unobservable.add(u)
    if (d.net) r.net = true
    if (depth > 20) {
      r.unobservable.add('child_process: too deep')
      return false
    }
    if (!takeChildren(r, d.children ?? [], depth + 1)) return false
  }
  return true
}

// Write JSON atomically with the unrecorded fs.
export function writeJson(file, value) {
  const tmp = `${file}.${process.pid}.tmp`
  real.writeFileSync(tmp, JSON.stringify(value))
  real.renameSync(tmp, file)
}
