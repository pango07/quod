import {runtime, deny} from './prelude.mjs'
import * as api from 'vitest'
import {startTests, setSafeTimers} from 'vitest/internal/browser'

export default {
  async fetch(request) {
    if (new URL(request.url).pathname === '/health') return runtime.response({ready: true})
    try {
      const config = runtime.options.vitest
      const cleanups = []
      config.snapshotOptions.snapshotEnvironment = {
        getVersion: () => '1', getHeader: () => '// Quod managed read-only snapshots',
        resolvePath: path => path + '.snap', resolveRawPath: () => deny('raw snapshot'),
        readSnapshotFile: () => null,
        saveSnapshotFile: () => deny('snapshot write'), removeSnapshotFile: () => deny('snapshot remove'),
      }
      globalThis.__vitest_worker__ = {
        config, ctx: {pool: 'quod-managed'}, environment: {name: 'quod-managed', viteEnvironment: 'ssr'},
        onCleanup: fn => cleanups.push(fn), providedContext: {}, durations: {fetch: 0},
        rpc: new Proxy({snapshotSaved: () => {}}, {get(target, key) {
          if (key in target) return target[key]
          return () => deny('vitest RPC ' + String(key))
        }}),
      }
      if (config.globals) Object.assign(globalThis, api)
      setSafeTimers()
      const runner = new api.TestRunner(config)
      runner.importFile = () => import('__QUOD_TEST_ENTRY__')
      const files = await startTests([runtime.options.file], runner)
      for (const fn of cleanups) await fn()
      const rows = []
      function visit(task, names = []) {
        const next = task.file === task ? names : [...names, task.name]
        if (task.type === 'test') rows.push({name: next.join(' '), state: task.result?.state || task.mode,
          errors: task.result?.errors?.map(e => ({name: e.name, message: e.message, stack: e.stack})) || []})
        for (const child of task.tasks || []) visit(child, next)
      }
      files.forEach(file => visit(file))
      return runtime.response(runtime.finish({ok: files.every(f => f.result?.state === 'pass'), rows,
        files: files.map(f => ({name: f.name, state: f.result?.state, errors: f.result?.errors}))}))
    } catch (error) {
      return runtime.response(runtime.finish({ok: false, error: {name: error?.name, message: error?.message, stack: error?.stack}}))
    }
  },
}
