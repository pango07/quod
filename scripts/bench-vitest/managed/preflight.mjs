import {runtime} from './prelude.mjs'
import {AsyncLocalStorage} from 'node:async_hooks'

export default {
  async fetch(request) {
    if (new URL(request.url).pathname === '/health') return runtime.response({ready: true})
    const checks = [], check = (name, pass) => checks.push({name, pass: Boolean(pass)})
    try {
      check('fixed date', Date.now() === runtime.options.epoch)
      check('constructor alias stays managed', new Date.prototype.constructor().getTime() === runtime.options.epoch)
      check('constructor prototype has no native Date', Object.getPrototypeOf(Date) === Function.prototype)
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
      const failures = []
      for (const [name, invoke] of [
        ['fetch', () => fetch('https://example.com')],
        ['crypto', () => crypto.getRandomValues(new Uint8Array(4))],
        ['WeakRef', () => new WeakRef({})],
        ['WebAssembly', () => WebAssembly.compile(new Uint8Array())],
        ['AbortSignal.timeout', () => AbortSignal.timeout(1)],
      ]) {
        try { await invoke(); failures.push(name) } catch (error) { check('deny ' + name, String(error).includes('QUOD_MANAGED_UNSUPPORTED')) }
      }
      check('all attempted unsupported operations rejected', failures.length === 0)
      const data = runtime.finish({checks, random})
      check('caught rejection remains recorded', data.boundary.unsupported.length >= 5)
      check('no unguardable ambient properties', data.boundary.unguardable.length === 0)
      data.ok = checks.every(c => c.pass)
      return runtime.response(data)
    } catch (error) {
      return runtime.response(runtime.finish({ok: false, checks, error: String(error?.stack || error)}))
    }
  },
}
