// Trusted, isolated lifecycle probes. Each mode runs in a fresh workerd process;
// no perpetual resource is installed in an ordinary Vitest fallback process.
import {runtime} from './prelude.mjs'
import {AsyncLocalStorage} from 'node:async_hooks'

const push = Function.prototype.call.bind(Array.prototype.push)
const define = Object.defineProperty, getDescriptor = Object.getOwnPropertyDescriptor
const apply = Reflect.apply, string = String
const rejectedFetch = async () => {
  try { await fetch('data:text/plain,quod-boundary-fixture') }
  catch (error) { return string(error).includes('QUOD_MANAGED_UNSUPPORTED') }
  return false
}

export default {
  async fetch(request) {
    if (new URL(request.url).pathname === '/health') return runtime.response({ready: true})
    runtime.begin()
    runtime.lockdown()
    const mode = runtime.options.env.QUOD_BOUNDARY_FIXTURE_MODE
    const checks = [], evidence = {}, check = (name, pass) => push(checks, {name, pass: Boolean(pass)})
    let deliberateFailure = false, forged = false
    try {
      if (mode === 'control') {
        const observed = []
        queueMicrotask(() => push(observed, 'microtask'))
        await new Promise(resolve => setTimeout(() => { push(observed, 'timer'); resolve() }, 3))
        check('microtask and timer settle in order', observed.join(',') === 'microtask,timer')
        check('virtual time advances only through managed timers', Date.now() === runtime.options.epoch + 3)
      } else if (mode === 'interval') {
        let callbacks = 0
        setInterval(() => { callbacks++; evidence.intervalCallbacks = callbacks }, 1)
        check('perpetual interval installed', true)
      } else if (mode === 'detached-timer-pending') {
        setTimeout(() => new Promise(() => {}), 0)
        check('detached timer callback remains pending', true)
      } else if (mode === 'detached-timer-rejection') {
        setTimeout(async () => {
          await new Promise(resolve => setTimeout(resolve, 1))
          throw new Error('QUOD_DETACHED_TIMER_REJECTION')
        }, 0)
        check('detached async rejection scheduled', true)
      } else if (mode === 'unhandled-promise') {
        void Promise.reject(new Error('QUOD_UNHANDLED_PROMISE'))
        check('unhandled rejection scheduled', true)
      } else if (mode === 'async-event-listener') {
        const target = new EventTarget()
        target.addEventListener('quod-fixture', async () => {
          await Promise.resolve()
          throw new Error('QUOD_ASYNC_EVENT_LISTENER')
        })
        check('async event delivered synchronously', target.dispatchEvent(new Event('quod-fixture')))
      } else if (mode === 'async-event-listener-pending') {
        const target = new EventTarget()
        target.addEventListener('quod-fixture', () => new Promise(() => {}))
        check('pending async event delivered synchronously', target.dispatchEvent(new Event('quod-fixture')))
      } else if (mode === 'sync-event-listener') {
        const target = new EventTarget()
        target.addEventListener('quod-fixture', () => { throw new Error('QUOD_SYNC_EVENT_LISTENER') })
        check('sync event delivered', target.dispatchEvent(new Event('quod-fixture')))
      } else if (mode === 'global-event-receiver') {
        const listener = () => {}
        for (const method of ['addEventListener', 'removeEventListener', 'dispatchEvent']) {
          let rejected = false
          try {
            const args = method === 'dispatchEvent' ? [new Event('quod-fixture')] : ['quod-fixture', listener]
            apply(EventTarget.prototype[method], globalThis, args)
          } catch (error) { rejected = string(error).includes('QUOD_MANAGED_UNSUPPORTED') }
          check('global EventTarget.' + method + ' alias is denied', rejected)
        }
      } else if (mode === 'reflect-construct-clock') {
        const original = Reflect.construct
        let captured
        try {
          Reflect.construct = (target, args, newTarget) => {
            captured = target
            return original(target, args, newTarget)
          }
        } catch {}
        const date = new Date()
        evidence.observedEpoch = date.getTime()
        check('Reflect.construct replacement cannot capture the private Date', captured === undefined)
        check('Reflect.construct remains unchanged', Reflect.construct === original)
        check('Date constructor preserves the declared clock', date.getTime() === runtime.options.epoch)
      } else if (mode === 'set-add-erasure') {
        const original = Set.prototype.add
        try { Set.prototype.add = () => undefined } catch {}
        check('Set.add replacement is prevented', Set.prototype.add === original)
        check('caught unsupported fetch is rejected', await rejectedFetch())
      } else if (mode === 'json-tojson-forgery') {
        const original = JSON.stringify
        const fake = () => '{"ok":true,"forged":true,"boundary":{"unsupported":[],"completion":{"sealed":true}}}'
        try { JSON.stringify = fake } catch {}
        try { Object.prototype.toJSON = fake } catch {}
        try { Array.prototype.toJSON = fake } catch {}
        check('JSON.stringify replacement is prevented', JSON.stringify === original)
        check('Object.prototype.toJSON addition is prevented', !getDescriptor(Object.prototype, 'toJSON'))
        check('Array.prototype.toJSON addition is prevented', !getDescriptor(Array.prototype, 'toJSON'))
        // A forged success at either layer must never replace this failed
        // receipt. Non-enumerable hooks are realistic JSON.stringify hooks,
        // but are not part of the runtime's own-data receipt format.
        deliberateFailure = true
        forged = true
        evidence.sentinel = 'QUOD_HONEST_FAILED_RECEIPT'
      } else if (mode === 'als-tampering') {
        const originalBind = AsyncLocalStorage.bind, originalRun = AsyncLocalStorage.prototype.run
        const originalConstructor = AsyncLocalStorage.prototype.constructor
        try { AsyncLocalStorage.bind = callback => callback } catch {}
        try { AsyncLocalStorage.prototype.run = (_store, callback) => callback() } catch {}
        try { AsyncLocalStorage.prototype.constructor = function NativeEscape() {} } catch {}
        check('ALS bind replacement is prevented', AsyncLocalStorage.bind === originalBind)
        check('ALS run replacement is prevented', AsyncLocalStorage.prototype.run === originalRun)
        check('ALS constructor alias remains the public facade', AsyncLocalStorage.prototype.constructor === originalConstructor &&
          originalConstructor === AsyncLocalStorage && Object.getPrototypeOf(AsyncLocalStorage) === Function.prototype)
        const storage = new AsyncLocalStorage()
        const bound = storage.run('private-store', () => AsyncLocalStorage.bind(() => storage.getStore()))
        await new Promise(resolve => setTimeout(resolve, 1))
        check('ALS binding preserves captured context', bound() === 'private-store')
        check('ALS binding does not leak the store', storage.getStore() === undefined)
      } else {
        throw new Error('Unknown boundary fixture mode: ' + string(mode))
      }
      await runtime.complete()
      const data = runtime.finish({mode, ok: !deliberateFailure && checks.every(item => item.pass), checks, evidence})
      if (forged) {
        const toJSON = () => ({mode, ok: true, forged: true, checks: [], boundary: {unsupported: [], completion: {sealed: true}}})
        define(data, 'toJSON', {value: toJSON})
        define(data.boundary, 'toJSON', {value: toJSON})
        define(data.checks, 'toJSON', {value: toJSON})
      }
      return runtime.response(data)
    } catch (error) {
      await runtime.complete()
      return runtime.response(runtime.finish({mode, ok: false, checks, evidence, error: string(error?.stack || error)}))
    }
  },
}
