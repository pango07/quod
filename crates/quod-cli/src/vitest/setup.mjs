// quod vitest: the setup file Quod puts first in every recorded project's
// setupFiles (from node_modules/.quod-vitest, so node loads it natively).
// It runs in the test file's own worker (vitest isolates each file in a
// fresh worker), before the project's setup files and the test module, and
// records what the file reads from here on, and which functions of the
// workspace's source it executed (V8 precise coverage at function
// granularity, binary: executed or not). Nothing it does is observable to
// the test: the source is not touched (instrumenting it changed function
// source text and test names), only the inspector watches.
// Function.prototype.toString is wrapped (preload.cjs, before any module
// loads) to notice a test reading source: no narrowing for that file.
// Its record is written when the worker is torn down, after the file's
// last hook.
//
// What it cannot see it says: no record (or an `unobservable` reason) and
// the file always runs.
import { createRecorder, patchEnv, patchEscapes, patchFs, patchLoader, real, scratch, syncESM, takeChildren, writeJson } from './record.mjs'
import { executedFunctions, io, unmapped, unmappedSample } from './coverage.mjs'
import nodeModule from 'node:module'
import inspector from 'node:inspector'

const dumpDir = process.env.QUOD_VITEST_DUMP
const stateDir = process.env.QUOD_VITEST_STATE
const root = process.env.QUOD_VITEST_ROOT
const ws = globalThis.__vitest_worker__

// QUOD_VITEST_DISABLE (comma list: fs, env, loader, escapes, cov, take):
// for measuring the recorder's cost only; a disabled part records nothing,
// so the driver records no file of such a run (cov: no function-level data;
// take: coverage on but never taken, i.e. V8's precise mode alone).
const off = new Set((process.env.QUOD_VITEST_DISABLE ?? '').split(',').filter(Boolean))
// Read before the env is watched (Quod's own settings are not inputs).
const tickMs = Number(process.env.QUOD_VITEST_COV_TICK ?? 0)
const covCache = process.env.QUOD_VITEST_COVCACHE !== '0'
// The files that take coverage this run (the driver's plan.cov), or every
// file when unset. A file's first witness never replays, so its coverage
// would never be used: V8's precise mode, the costliest part of recording,
// is left off for it.
const covFiles = (() => {
  const f = process.env.QUOD_VITEST_COV_FILES
  if (!f) return null
  try {
    return new Set(JSON.parse(real.readFileSync(f, 'utf8')))
  } catch {
    return null
  }
})()

// The worker's module graph in vitest 4's shape (evaluatedModules: id ->
// { id, file, meta.code }). vitest 3 (vite-node) keeps it in moduleCache:
// normalized file path (the id vite-node evaluates the module under, its
// script's name) -> { code: the transformed code it evaluated, ... }.
function moduleGraph(ws) {
  if (ws.evaluatedModules?.idToModuleMap) return ws.evaluatedModules
  const mc = ws.moduleCache
  if (!(mc instanceof Map)) return null
  const idToModuleMap = new Map()
  for (const [id, mod] of Map.prototype.entries.call(mc)) {
    if (typeof id === 'string') idToModuleMap.set(id, { id, file: id.split('?')[0], meta: { code: mod?.code } })
  }
  return { idToModuleMap }
}

const post = (s, method, params) =>
  new Promise((resolve, reject) => s.post(method, params, (err, res) => (err ? reject(err) : resolve(res))))

if (dumpDir && stateDir && ws && !globalThis.__quod_vitest__) {
  const t0 = performance.now()
  const steps = {}
  let ts = t0
  const step = (k) => {
    const n = performance.now()
    steps[k] = n - ts
    ts = n
  }
  const file = ws.current?.file ?? ws.current
  // The workspace root (the driver's): a directory the file creates
  // outside it is the file's own output (record.mjs `made`).
  const wsRoot = process.env.QUOD_VITEST_WS || null
  const r = createRecorder({ stateDir, codeDir: import.meta.dirname, mode: 'worker', wsRoot })
  globalThis.__quod_vitest__ = r
  // A node child process this file starts is traced (child.mjs), its
  // dumps beside this file's (QUOD_VITEST_TRACE=0: unobservable, as before).
  if (process.env.QUOD_VITEST_TRACE !== '0' && file?.id) r.trace = { codeDir: import.meta.dirname, base: `${dumpDir}/${file.id}`, n: 0 }
  if (!off.has('fs')) patchFs(r, { sync: false })
  step('fs')
  if (!off.has('env')) patchEnv(r)
  step('env')
  if (!off.has('loader')) patchLoader(r)
  step('loader')
  if (!off.has('escapes')) patchEscapes(r, { sync: false })
  if (!off.has('fs') || !off.has('escapes')) syncESM()
  step('escapes')
  const parts = ['fs', 'env', 'loader', 'escapes', 'main', 'preload', 'setup']
  if (parts.some((p) => off.has(p))) r.unobs(`recorder disabled: ${[...off]}`)
  // Function-level narrowing needs the functions this file executed; a test
  // that reads functions' source (Function.prototype.toString) depends on
  // more than what it ran, so it gets none.
  let session = null
  let usedToString = false
  // vitest's own coverage is on: V8's coverage mode is the isolate's, so
  // starting Quod's (binary, function granularity) would change what the
  // v8 provider sees. No function-level data then (the file runs whenever
  // any source it loaded changes: its recorded coverage is of that source).
  const covOn = !!ws.config?.coverage?.enabled
  const covWanted = !covFiles || covFiles.has(file?.filepath)
  if (!off.has('cov') && root && !covOn && covWanted) {
    try {
      if (globalThis.__quod_cov) session = globalThis.__quod_cov
      else {
        session = new inspector.Session()
        session.connect()
        await post(session, 'Profiler.enable')
        // Precise (never best effort, which can drop an executed function),
        // function granularity, binary counts: the cheapest exact mode.
        await post(session, 'Profiler.startPreciseCoverage', { callCount: false, detailed: false })
      }
      // The wrapper is preload.cjs's (installed before anything loaded);
      // what ran before this file's own code does not count.
      if (globalThis.__quod_toString) globalThis.__quod_toString.used = false
      else if (!off.has('preload')) usedToString = true
    } catch {
      session = null
    }
  }
  // Experiment (QUOD_VITEST_COV_TICK=<ms>): take coverage every <ms> too.
  // Binary coverage reports a function once; V8 does not inline a function
  // into optimized code before it has reported, so early takes may let hot
  // code optimize. The takes' workspace scripts are merged.
  const merged = new Map()
  const absorb = (result) => {
    for (const s of result) {
      const u = s.url.startsWith('file://') ? s.url.slice(7) : s.url
      if (!u.startsWith(root) || u.includes('/node_modules/')) continue
      let m = merged.get(s.scriptId)
      if (!m) merged.set(s.scriptId, (m = { scriptId: s.scriptId, url: s.url, fns: new Map() }))
      for (const f of s.functions) {
        const r0 = f.ranges[0]
        if (!r0) continue
        const k = `${r0.startOffset}:${r0.endOffset}`
        const o = m.fns.get(k)
        if (!o || (o.ranges[0].count === 0 && r0.count > 0)) m.fns.set(k, f)
      }
    }
  }
  let ticker = null
  let tickBusy = false
  let tickN = 0
  if (session && tickMs > 0) {
    ticker = setInterval(() => {
      if (tickBusy) return
      tickBusy = true
      post(session, 'Profiler.takePreciseCoverage')
        .then((res) => {
          absorb(res.result)
          tickN++
        })
        .catch(() => {})
        .finally(() => (tickBusy = false))
    }, tickMs)
    ticker.unref?.()
  }
  step('cov')
  const setupMs = performance.now() - t0
  if (r.loaderMissing) r.unobs('no module.registerHooks')
  if (!file?.id || typeof ws.onCleanup !== 'function') r.unobs('no worker hooks')
  if (ws.config?.isolate === false) r.unobs('not isolated')
  const req = nodeModule.createRequire(import.meta.url)
  // CJS loaded before this file ran (the environment, e.g. jsdom)
  for (const k of Object.keys(req.cache)) r.add('f', k)
  ws.onCleanup?.(async () => {
    const t1 = performance.now()
    try {
      r.on = true
      for (const k of Object.keys(req.cache)) r.add('f', k)
      const em = moduleGraph(ws)
      const modules = []
      if (em?.idToModuleMap) {
        for (const node of em.idToModuleMap.values()) {
          const f = node.file || node.id
          if (typeof f === 'string' && f.startsWith('/')) modules.push(f.split('?')[0])
        }
      } else r.unobs('no module graph')
      r.on = false
      let cov = null
      let covMs = 0
      let mapMs = 0
      if (globalThis.__quod_toString?.used) usedToString = true
      if (session && !usedToString && !off.has('take')) {
        const tc = performance.now()
        try {
          if (ticker) clearInterval(ticker)
          const res = await post(session, 'Profiler.takePreciseCoverage')
          let scripts = res.result
          if (ticker) {
            absorb(scripts)
            scripts = [...merged.values()].map((m) => ({ scriptId: m.scriptId, url: m.url, functions: [...m.fns.values()] }))
          }
          covMs = performance.now() - tc
          const tm = performance.now()
          // QUOD_VITEST_COVCACHE=0: map every module afresh (measuring)
          const cacheDir = covCache ? `${stateDir}/covmap` : null
          Object.assign(io, { read: real.readFileSync, write: real.writeFileSync, rename: real.renameSync, mkdir: real.mkdirSync })
          cov = executedFunctions(scripts, em, root, cacheDir)
          mapMs = performance.now() - tm
        } catch {
          cov = null
        }
      }
      const tw = performance.now()
      if (r.children.length) takeChildren(r)
      writeJson(`${dumpDir}/${file.id}.json`, {
        id: file.id,
        file: file.filepath,
        project: ws.ctx?.projectName ?? '',
        modules,
        keys: [...r.keys],
        writes: scratch(r),
        watches: [...r.watches],
        envKept: [...r.envKept],
        unobservable: [...r.unobservable],
        cov,
        covOn,
        covSkipped: !covWanted,
        usedToString,
        unmapped,
        unmappedSample: Object.entries(unmappedSample).slice(0, 1),
        ms: { setup: setupMs, dump: performance.now() - t1, cov: covMs, map: mapMs, keys: tw - t1 - covMs - mapMs, ticks: tickN, steps },
      })
    } catch (e) {
      console.error('quod vitest: could not record', file?.filepath, e)
    }
  })
}
