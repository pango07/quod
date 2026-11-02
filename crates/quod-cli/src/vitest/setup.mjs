// quod vitest: the setup file Quod puts first in every recorded project's
// setupFiles (from node_modules/.quod-vitest, so node loads it natively).
// It runs in the test file's own worker (vitest isolates each file in a
// fresh worker), before the project's setup files and the test module, and records what the file reads from here on. Its record is
// written when the worker is torn down, after the file's last hook.
//
// What it cannot see it says: no record (or an `unobservable` reason) and
// the file always runs.
import { createRecorder, patchEnv, patchEscapes, patchFs, patchLoader, writeJson } from './record.mjs'
import nodeModule from 'node:module'

const dumpDir = process.env.QUOD_VITEST_DUMP
const stateDir = process.env.QUOD_VITEST_STATE
const ws = globalThis.__vitest_worker__

if (dumpDir && stateDir && ws && !globalThis.__quod_vitest__) {
  const file = ws.current?.file ?? ws.current
  const r = createRecorder({ stateDir, codeDir: import.meta.dirname, mode: 'worker' })
  globalThis.__quod_vitest__ = r
  patchFs(r)
  patchEnv(r)
  patchLoader(r)
  await patchEscapes(r)
  if (r.loaderMissing) r.unobs('no module.registerHooks')
  if (!file?.id || typeof ws.onCleanup !== 'function') r.unobs('no worker hooks')
  if (ws.config?.isolate === false) r.unobs('not isolated')
  const req = nodeModule.createRequire(import.meta.url)
  // CJS loaded before this file ran (the environment, e.g. jsdom)
  for (const k of Object.keys(req.cache)) r.add('f', k)
  ws.onCleanup?.(() => {
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
      writeJson(`${dumpDir}/${file.id}.json`, {
        id: file.id,
        file: file.filepath,
        project: ws.ctx?.projectName ?? '',
        modules,
        keys: [...r.keys],
        writes: [...r.writes],
        unobservable: [...r.unobservable],
      })
    } catch (e) {
      console.error('quod vitest: could not record', file?.filepath, e)
    }
  })
}
