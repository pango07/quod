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
      } else if (mode === 'global-onunhandledrejection' || mode === 'global-onerror') {
        const property = mode === 'global-onunhandledrejection' ? 'onunhandledrejection' : 'onerror'
        evidence.legacyHandlerCalls = 0
        const suppress = event => {
          evidence.legacyHandlerCalls++
          if (event && typeof event.stopImmediatePropagation === 'function') event.stopImmediatePropagation()
          return true
        }
        let denied = false
        try { globalThis[property] = suppress }
        catch (error) { denied = string(error).includes('QUOD_MANAGED_UNSUPPORTED: global ' + property) }
        check('global ' + property + ' setter is sticky unsupported', denied)
        check('global ' + property + ' getter exposes no native handler', globalThis[property] === null)
        if (mode === 'global-onunhandledrejection') {
          void Promise.reject(new Error('QUOD_LEGACY_REJECTION'))
          check('rejection cannot be suppressed by a legacy handler', true)
        } else {
          let fetchDenied = false
          try { globalThis.onfetch = suppress }
          catch (error) { fetchDenied = string(error).includes('QUOD_MANAGED_UNSUPPORTED: global onfetch') }
          check('global onfetch registration is sticky unsupported', fetchDenied && globalThis.onfetch === null)
          // The controller catches this independently of native onerror. It
          // must still report a failure after the setter denial is caught.
          setTimeout(() => { throw new Error('QUOD_LEGACY_ERROR') }, 0)
          check('error still reaches private timer accounting', true)
        }
      } else if (mode === 'local-legacy-handler') {
        const target = new EventTarget()
        evidence.legacyHandlerCalls = 0
        target.onfoo = async () => {
          evidence.legacyHandlerCalls++
          await new Promise(() => {})
        }
        let denied = false
        try { target.dispatchEvent(new Event('foo')) }
        catch (error) { denied = string(error).includes('QUOD_MANAGED_UNSUPPORTED: legacy event handler onfoo') }
        check('local legacy handler dispatch is sticky unsupported', denied)
        check('legacy handler is not invoked before denial', evidence.legacyHandlerCalls === 0)
      } else if (mode === 'local-legacy-getter') {
        const target = new EventTarget()
        evidence.legacyGetterCalls = 0
        evidence.legacyHandlerCalls = 0
        define(target, 'onfoo', {configurable: true, get() {
          evidence.legacyGetterCalls++
          if (evidence.legacyGetterCalls === 1) return null
          return async () => {
            evidence.legacyHandlerCalls++
            await new Promise(() => {})
          }
        }})
        let denied = false
        try { target.dispatchEvent(new Event('foo')) }
        catch (error) { denied = string(error).includes('QUOD_MANAGED_UNSUPPORTED: legacy event handler onfoo') }
        check('legacy accessor descriptor is denied before public reads', denied)
        check('legacy getter is never evaluated', evidence.legacyGetterCalls === 0)
        check('legacy getter callback is never invoked', evidence.legacyHandlerCalls === 0)
      } else if (mode === 'abort-handler') {
        const controller = new AbortController(), signal = controller.signal
        evidence.legacyHandlerCalls = 0
        evidence.supportedAbortCalls = 0
        let denied = false
        try { signal.onabort = () => { evidence.legacyHandlerCalls++ } }
        catch (error) { denied = string(error).includes('QUOD_MANAGED_UNSUPPORTED: AbortSignal.onabort') }
        check('AbortSignal.onabort setter is sticky unsupported', denied)
        check('AbortSignal.onabort getter exposes no native handler', signal.onabort === null)
        signal.addEventListener('abort', () => { evidence.supportedAbortCalls++ })
        controller.abort('declared abort reason')
        check('tracked addEventListener abort callback is preserved', evidence.supportedAbortCalls === 1)
        check('legacy abort callback is not invoked', evidence.legacyHandlerCalls === 0)
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
      if (mode === 'global-onunhandledrejection' || mode === 'global-onerror' || mode === 'local-legacy-handler' || mode === 'local-legacy-getter' || mode === 'abort-handler') {
        check('no legacy callback ran during completion', evidence.legacyHandlerCalls === 0)
      }
      if (mode === 'local-legacy-getter') check('no legacy getter ran during completion', evidence.legacyGetterCalls === 0)
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
