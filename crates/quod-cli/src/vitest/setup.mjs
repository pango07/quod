// quod vitest: the setup file Quod puts first in every recorded project's
// setupFiles (from node_modules/.quod-vitest, so node loads it natively).
// It runs in the test file's own worker (vitest isolates each file in a
// fresh worker), before the project's setup files and the test module, and
// records what the file reads from here on, and which functions of the
// workspace's source it executed, as a whole and per test: the driver
// instruments the source (instrument.mjs), and the arrays it registers here
// are read and cleared around every test.
// Its record is written when the worker is torn down, after the file's
// last hook.
//
// What it cannot see it says: no record (or an `unobservable` reason) and
// the file always runs.
import { createRecorder, patchEnv, patchEscapes, patchFs, patchLoader, writeJson } from './record.mjs'
import nodeModule from 'node:module'
import { afterEach, beforeEach } from 'vitest'

const dumpDir = process.env.QUOD_VITEST_DUMP
const stateDir = process.env.QUOD_VITEST_STATE
const root = process.env.QUOD_VITEST_ROOT
const ws = globalThis.__vitest_worker__

// QUOD_VITEST_DISABLE (comma list: fs, env, loader, escapes, cov): for
// measuring the recorder's cost only; a disabled part records nothing, so
// the driver records no file of such a run (cov: no function-level data).
const off = new Set((process.env.QUOD_VITEST_DISABLE ?? '').split(',').filter(Boolean))


if (dumpDir && stateDir && ws && !globalThis.__quod_vitest__) {
  const t0 = performance.now()
  const file = ws.current?.file ?? ws.current
  const r = createRecorder({ stateDir, codeDir: import.meta.dirname, mode: 'worker' })
  globalThis.__quod_vitest__ = r
  if (!off.has('fs')) patchFs(r)
  if (!off.has('env')) patchEnv(r)
  if (!off.has('loader')) patchLoader(r)
  if (!off.has('escapes')) patchEscapes(r)
  if ([...off].some((p) => p !== 'cov')) r.unobs(`recorder disabled: ${[...off]}`)
  // Function-level narrowing needs the functions this file executed; a test
  // that reads functions' source (Function.prototype.toString) depends on
  // more than what it ran, so it gets none.
  let usedToString = false
  // Executed functions: the instrumented modules register their arrays.
  const reg = []
  const fileHits = new Map()
  const tests = {}
  let open = null
  let concurrent = false
  const flush = (into) => {
    for (const [key, a] of reg) {
      let s = into.get(key)
      if (!s) into.set(key, (s = new Set()))
      for (let i = 0; i < a.length; i++) {
        if (a[i]) {
          s.add(i)
          a[i] = 0
        }
      }
    }
  }
  const plain = (m) => Object.fromEntries([...m].map(([k, s]) => [k, [...s].sort((x, y) => x - y)]))
  if (!off.has('cov') && root) {
    globalThis.__quod_hits = (n, key) => {
      const a = new Uint8Array(n)
      reg.push([key, a])
      return a
    }
    // What runs between tests (collection, beforeAll/afterAll, hooks Quod's
    // run outside of) is the file's: every test depends on it.
    beforeEach((ctx) => {
      if (open) concurrent = true
      flush(fileHits)
      open = ctx?.task?.id ?? null
    })
    afterEach((ctx) => {
      const id = ctx?.task?.id
      if (open && id === open) {
        const m = new Map()
        flush(m)
        tests[id] = plain(m)
      } else concurrent = true
      open = null
    })
    try {
      const fts = Function.prototype.toString
      const wrapped = {
        toString() {
          usedToString = true
          return fts.call(this)
        },
      }.toString
      Object.defineProperty(Function.prototype, 'toString', { value: wrapped, writable: true, configurable: true })
    } catch {}
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
      const tc = performance.now()
      flush(fileHits)
      const hits = reg.length && !usedToString ? plain(fileHits) : null
      const covMs = performance.now() - tc
      writeJson(`${dumpDir}/${file.id}.json`, {
        id: file.id,
        file: file.filepath,
        project: ws.ctx?.projectName ?? '',
        modules,
        keys: [...r.keys],
        writes: [...r.writes],
        watches: [...r.watches],
        unobservable: [...r.unobservable],
        hits,
        tests: hits && !concurrent ? tests : null,
        usedToString,
        ms: { setup: setupMs, dump: performance.now() - t1, cov: covMs },
      })
    } catch (e) {
      console.error('quod vitest: could not record', file?.filepath, e)
    }
  })
}
