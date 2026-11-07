import {runtime} from './prelude.mjs'
import {AsyncLocalStorage} from 'node:async_hooks'

export default {
  async fetch(request) {
    if (new URL(request.url).pathname === '/health') return runtime.response({ready: true})
    runtime.begin()
    runtime.lockdown()
    const checks = [], check = (name, pass) => checks.push({name, pass: Boolean(pass)})
    try {
      check('fixed date', Date.now() === runtime.options.epoch)
      check('constructor alias stays managed', new Date.prototype.constructor().getTime() === runtime.options.epoch)
      check('constructor prototype has no native Date', Object.getPrototypeOf(Date) === Function.prototype)
      let aliasesManaged = true
      for (let owner = Object.getPrototypeOf(globalThis); owner && owner !== Object.prototype; owner = Object.getPrototypeOf(owner)) {
        for (const key of ['Date', 'performance', 'navigator', 'console', 'process',
          'setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'setImmediate', 'clearImmediate']) {
          const descriptor = Object.getOwnPropertyDescriptor(owner, key)
          if (descriptor && (descriptor.get || descriptor.value !== globalThis[key])) aliasesManaged = false
        }
      }
      check('managed global aliases have no native prototype value', aliasesManaged)
      const random = [Math.random(), Math.random(), Math.random()]
      const local = new AsyncLocalStorage(), order = []
      const result = await Promise.all(['first', 'second'].map((name, index) => local.run(name, async () => {
        await Promise.resolve()
        await new Promise(resolve => setTimeout(resolve, index ? 2 : 5))
        order.push(name)
        return local.getStore()
      })))
      check('ALS separates concurrent stores', result.join(',') === 'first,second' && local.getStore() === undefined)
      check('timers use virtual order', order.join(',') === 'second,first' && Date.now() === runtime.options.epoch + 5)
      const target = new EventTarget(), event = new Event('local')
      let deliveries = 0
      target.addEventListener('local', () => { deliveries++ }, {once: true})
      target.dispatchEvent(event); target.dispatchEvent(event)
      check('local synchronous events preserve once semantics', deliveries === 1)
      // Keep these aliases indirect. The compiler rejects residual direct
      // eval/Function calls even in reviewed adapters; this checks the runtime
      // guards that must still reject an alias acquired before invocation.
      const evaluate = globalThis.eval, construct = globalThis.Function
      const constructAsync = Object.getPrototypeOf(async function () {}).constructor
      const constructGenerator = Object.getPrototypeOf(function* () {}).constructor
      const constructAsyncGenerator = Object.getPrototypeOf(async function* () {}).constructor
      const failures = []
      for (const [name, invoke] of [
        ['fetch', () => fetch('https://example.com')],
        ['crypto', () => crypto.getRandomValues(new Uint8Array(4))],
        ['WeakRef', () => new WeakRef({})],
        ['FinalizationRegistry', () => new FinalizationRegistry(() => {})],
        ['Event.timeStamp', () => event.timeStamp],
        ['WebAssembly', () => WebAssembly.compile(new Uint8Array())],
        ['AbortSignal.timeout', () => AbortSignal.timeout(1)],
        ['eval alias', () => evaluate('1')],
        ['Function alias', () => construct('return 1')],
        ['AsyncFunction alias', () => constructAsync('return 1')],
        ['GeneratorFunction alias', () => constructGenerator('yield 1')],
        ['AsyncGeneratorFunction alias', () => constructAsyncGenerator('yield 1')],
        ['global EventTarget.addEventListener', () => EventTarget.prototype.addEventListener.call(globalThis, 'quod-private-fixture', () => {})],
        ['global EventTarget.removeEventListener', () => EventTarget.prototype.removeEventListener.call(globalThis, 'quod-private-fixture', () => {})],
        ['global EventTarget.dispatchEvent', () => EventTarget.prototype.dispatchEvent.call(globalThis, new Event('quod-private-fixture'))],
      ]) {
        try { await invoke(); failures.push(name) } catch (error) { check('deny ' + name, String(error).includes('QUOD_MANAGED_UNSUPPORTED')) }
      }
      check('all attempted unsupported operations rejected', failures.length === 0)
      await runtime.complete()
      const data = runtime.finish({checks, random})
      check('caught rejection remains recorded', data.boundary.unsupported.length >= 15)
      check('no unguardable ambient properties', data.boundary.unguardable.length === 0)
      check('completion drains every managed effect', data.boundary.completion.sealed &&
        data.boundary.pendingTimers === 0 && data.boundary.completion.pendingTimerCallbacks === 0 &&
        data.boundary.completion.pendingMicrotasks === 0)
      data.ok = checks.every(c => c.pass)
      return runtime.response(data)
    } catch (error) {
      await runtime.complete()
      return runtime.response(runtime.finish({ok: false, checks, error: String(error?.stack || error)}))
    }
  },
}
