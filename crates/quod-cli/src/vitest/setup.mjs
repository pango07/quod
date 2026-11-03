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
import { createRecorder, patchEnv, patchEscapes, patchFs, patchLoader, writeJson } from './record.mjs'
import { executedFunctions, unmapped, unmappedSample } from './coverage.mjs'
import nodeModule from 'node:module'
import inspector from 'node:inspector'

const dumpDir = process.env.QUOD_VITEST_DUMP
const stateDir = process.env.QUOD_VITEST_STATE
const root = process.env.QUOD_VITEST_ROOT
const ws = globalThis.__vitest_worker__

// QUOD_VITEST_DISABLE (comma list: fs, env, loader, escapes, cov): for
// measuring the recorder's cost only; a disabled part records nothing, so
// the driver records no file of such a run (cov: no function-level data).
const off = new Set((process.env.QUOD_VITEST_DISABLE ?? '').split(',').filter(Boolean))

const post = (s, method, params) =>
  new Promise((resolve, reject) => s.post(method, params, (err, res) => (err ? reject(err) : resolve(res))))

if (dumpDir && stateDir && ws && !globalThis.__quod_vitest__) {
  const t0 = performance.now()
  const file = ws.current?.file ?? ws.current
  // The workspace root (the driver's): a directory the file creates
  // outside it is the file's own output (record.mjs `made`).
  const wsRoot = process.env.QUOD_VITEST_WS || null
  const r = createRecorder({ stateDir, codeDir: import.meta.dirname, mode: 'worker', wsRoot })
  globalThis.__quod_vitest__ = r
  if (!off.has('fs')) patchFs(r)
  if (!off.has('env')) patchEnv(r)
  if (!off.has('loader')) patchLoader(r)
  if (!off.has('escapes')) patchEscapes(r)
  if ([...off].some((p) => p !== 'cov')) r.unobs(`recorder disabled: ${[...off]}`)
  // Function-level narrowing needs the functions this file executed; a test
  // that reads functions' source (Function.prototype.toString) depends on
  // more than what it ran, so it gets none.
  let session = null
  let usedToString = false
  if (!off.has('cov') && root) {
    try {
      session = new inspector.Session()
      session.connect()
      await post(session, 'Profiler.enable')
      // Precise (never best effort, which can drop an executed function),
      // function granularity, binary counts: the cheapest exact mode.
      await post(session, 'Profiler.startPreciseCoverage', { callCount: false, detailed: false })
      // The wrapper is preload.cjs's (installed before anything loaded);
      // what ran before this file's own code does not count.
      if (globalThis.__quod_toString) globalThis.__quod_toString.used = false
      else usedToString = true
    } catch {
      session = null
    }
  }
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
      const em = ws.evaluatedModules
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
      if (globalThis.__quod_toString?.used) usedToString = true
      if (session && !usedToString) {
        const tc = performance.now()
        try {
          const res = await post(session, 'Profiler.takePreciseCoverage')
          cov = executedFunctions(res.result, em, root)
        } catch {
          cov = null
        }
        covMs = performance.now() - tc
      }
      writeJson(`${dumpDir}/${file.id}.json`, {
        id: file.id,
        file: file.filepath,
        project: ws.ctx?.projectName ?? '',
        modules,
        keys: [...r.keys],
        writes: [...r.writes],
        watches: [...r.watches],
        envKept: [...r.envKept],
        unobservable: [...r.unobservable],
        cov,
        usedToString,
        unmapped,
        unmappedSample: Object.entries(unmappedSample).slice(0, 1),
        ms: { setup: setupMs, dump: performance.now() - t1, cov: covMs },
      })
    } catch (e) {
      console.error('quod vitest: could not record', file?.filepath, e)
    }
  })
}
