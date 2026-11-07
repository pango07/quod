import {runtime, deny} from './prelude.mjs'
import * as api from 'vitest'
import {startTests, setSafeTimers} from 'vitest/internal/browser'

// Capture before importFile executes any test module. A test may temporarily
// replace public Array/Object/String methods without changing this receipt.
const push = Function.prototype.call.bind(Array.prototype.push)
const string = String
const assign = Object.assign
const own = Function.prototype.call.bind(Object.prototype.hasOwnProperty)
const URLClass = URL
function errorRows(errors) {
  const result = []
  for (let index = 0; index < (errors?.length || 0); index++) {
    const error = errors[index]
    push(result, {name: string(error?.name || 'Error'), message: string(error?.message || error), stack: string(error?.stack || '')})
  }
  return result
}

export default {
  async fetch(request) {
    if (new URLClass(request.url).pathname === '/health') return runtime.response({ready: true})
    runtime.begin()
    let completed = false
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
        onCleanup: fn => push(cleanups, fn), providedContext: runtime.options.providedContext || {}, durations: {fetch: 0},
        rpc: new Proxy({snapshotSaved: () => {}}, {get(target, key) {
          if (own(target, key)) return target[key]
          return () => deny('vitest RPC ' + string(key))
        }}),
      }
      if (config.globals) assign(globalThis, api)
      setSafeTimers()
      const runner = new api.TestRunner(config)
      // startTests installs Vitest's own defaultTrace when absent. The normal
      // Node bootstrap otherwise injects an OpenTelemetry controller here;
      // this managed investigation declares no telemetry exporter.
      runner.trace = undefined
      runner.importFile = () => {
        runtime.lockdown()
        return import('__QUOD_TEST_ENTRY__')
      }
      const files = await startTests([runtime.options.file], runner)
      for (let index = 0; index < cleanups.length; index++) await cleanups[index]()
      const rows = [], fileRows = []
      let ok = true
      function visit(task, names = []) {
        // Names are primitive strings here; none of the returned data keeps
        // task objects, error getters, or user-owned toJSON hooks alive.
        const next = task.file === task ? names : names ? names + ' ' + string(task.name) : string(task.name)
        if (task.type === 'test') push(rows, {name: next, state: string(task.result?.state || task.mode), errors: errorRows(task.result?.errors)})
        for (let index = 0; index < (task.tasks?.length || 0); index++) visit(task.tasks[index], next)
      }
      for (let index = 0; index < files.length; index++) {
        const file = files[index]
        visit(file, '')
        const state = string(file.result?.state || file.mode), errors = errorRows(file.result?.errors)
        // A native all-skipped/todo module is complete without a passing body.
        // Retain its real state; the original v1 receipt validator still only
        // admits its original passing workload. Unknown/cancelled states fail.
        if (!['pass', 'skip', 'todo'].includes(state) || errors.length) ok = false
        push(fileRows, {name: string(file.name), state, errors})
      }
      // Vitest flushes its task updates but leaves a throttled reporting timer.
      // Drain it through the same bounded policy as detached user callbacks;
      // no timer is exempted by stack, delay, filename, or claimed origin.
      await runtime.complete()
      completed = true
      return runtime.response(runtime.finish({ok, rows, files: fileRows}))
    } catch (error) {
      const result = {ok: false, error: {name: string(error?.name || 'Error'), message: string(error?.message || error), stack: string(error?.stack || '')}}
      if (!completed) {
        runtime.lockdown()
        await runtime.complete()
      }
      return runtime.response(runtime.finish(result))
    }
  },
}
