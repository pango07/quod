import {runtime} from './prelude.mjs'
import {AsyncLocalStorage} from 'node:async_hooks'

const policy = 'quod-managed-frozen-array-promise-slots-v1-probe'
const contract = 'quod-managed-v0-protectors-frozen-slots-investigation'
const slots = [
  [Array.prototype, 'constructor', 'Array.prototype.constructor'],
  [Array.prototype, Symbol.iterator, 'Array.prototype[Symbol.iterator]'],
  [Array, Symbol.species, 'Array[Symbol.species]'],
  [Promise.prototype, 'constructor', 'Promise.prototype.constructor'],
  [Promise.prototype, 'then', 'Promise.prototype.then'],
  [Promise, 'resolve', 'Promise.resolve'],
  [Promise, Symbol.species, 'Promise[Symbol.species]'],
].map(([target, key, name]) => ({target, key, name, original: Object.getOwnPropertyDescriptor(target, key)}))

function originalFrozen(row) {
  const current = Object.getOwnPropertyDescriptor(row.target, row.key), original = row.original
  return current && current.value === original.value && current.get === original.get && current.set === original.set &&
    current.enumerable === original.enumerable && current.configurable === false &&
    (!('value' in current) || current.writable === false)
}

export default {
  async fetch(request) {
    if (new URL(request.url).pathname === '/health') return runtime.response({ready: true})
    runtime.begin()
    runtime.lockdown()
    const checks = [], check = (name, pass) => checks.push({name, pass: Boolean(pass)})
    let completed = false
    try {
      check('explicit frozen native slot policy', runtime.intrinsicPolicy === policy)
      check('four native targets are frozen', [Array, Array.prototype, Promise, Promise.prototype].every(Object.isFrozen))
      for (const row of slots) {
        check(row.name + ' retains original native descriptor identity', originalFrozen(row))
        let writeError = false, deleteError = false
        try { row.target[row.key] = () => 'replacement' } catch (error) { writeError = error instanceof TypeError }
        check(row.name + ' strict write cannot mutate', writeError && originalFrozen(row))
        try { delete row.target[row.key] } catch (error) { deleteError = error instanceof TypeError }
        check(row.name + ' strict delete cannot mutate', deleteError && originalFrozen(row))
        check(row.name + ' Reflect.set rejects without mutation', Reflect.set(row.target, row.key, () => 'replacement') === false && originalFrozen(row))
      }
      const speciesReceiver = {}
      for (const target of [Array, Promise]) {
        const getter = Object.getOwnPropertyDescriptor(target, Symbol.species).get
        check(target.name + ' native species getter returns its receiver', Reflect.apply(getter, speciesReceiver, []) === speciesReceiver)
      }
      check('native array species operations retain results', JSON.stringify([1, 2, 3].map(value => value * 2).filter(value => value > 2)) === '[4,6]')
      const promise = Promise.resolve(7)
      check('native Promise.resolve preserves identity and value', Promise.resolve(promise) === promise && await promise === 7)
      check('native Promise chains settle normally', await Promise.resolve(2).then(value => value + 3) === 5)
      class ChildPromise extends Promise {}
      const child = ChildPromise.resolve(3).then(value => value + 1)
      check('native subclass species retains its constructor', child instanceof ChildPromise && await child === 4)

      const array = [], localPromise = Promise.resolve(1)
      let constructorError = false, iteratorError = false, thenError = false, staticError = false
      try { array.constructor = () => null } catch (error) { constructorError = error instanceof TypeError }
      try { array[Symbol.iterator] = () => null } catch (error) { iteratorError = error instanceof TypeError }
      try { localPromise.then = () => null } catch (error) { thenError = error instanceof TypeError }
      try { ChildPromise.resolve = () => null } catch (error) { staticError = error instanceof TypeError }
      check('inherited array constructor assignment follows frozen data semantics', constructorError && !Object.hasOwn(array, 'constructor'))
      check('inherited array iterator assignment follows frozen data semantics', iteratorError && !Object.hasOwn(array, Symbol.iterator))
      check('inherited promise then assignment follows frozen data semantics', thenError && !Object.hasOwn(localPromise, 'then'))
      check('inherited subclass static resolve assignment follows frozen data semantics', staticError && !Object.hasOwn(ChildPromise, 'resolve'))
      const ownConstructor = () => 'own constructor', ownThen = () => 'own then'
      Object.defineProperty(array, 'constructor', {value: ownConstructor, writable: true, configurable: true})
      Object.defineProperty(localPromise, 'then', {value: ownThen, writable: true, configurable: true})
      check('explicit own data definitions remain supported', array.constructor === ownConstructor && localPromise.then === ownThen)
      const shadow = []
      shadow.push = () => 'own push'
      check('ordinary Array.push instance shadowing remains supported', shadow.push() === 'own push' && Object.hasOwn(shadow, 'push'))
      const error = new Error('instance')
      error.name = 'LocalError'
      check('ordinary Error.name instance shadowing remains supported', error.name === 'LocalError' && Object.hasOwn(error, 'name'))
      const now = Date.now
      Date.now = () => 123
      const mocked = Date.now() === 123
      Date.now = now
      check('managed Date.now can be temporarily replaced and restored', mocked && Date.now() === runtime.options.epoch)
      const storage = new AsyncLocalStorage()
      const contexts = await Promise.all(['first', 'second'].map((name, index) => storage.run(name, async () => {
        await Promise.resolve()
        await new Promise(resolve => setTimeout(resolve, index + 1))
        return storage.getStore()
      })))
      check('private ALS facade survives native Promise policy', contexts.join(',') === 'first,second' && storage.getStore() === undefined)

      let objectDenied = false, reflectDenied = false, fetchDenied = false
      try { Object.defineProperty(Array.prototype, 'constructor', {value: () => null}) } catch (error) {
        objectDenied = String(error).includes('QUOD_MANAGED_UNSUPPORTED: intrinsic mutation constructor')
      }
      try { Reflect.defineProperty(Promise.prototype, 'then', {value: () => null}) } catch (error) {
        reflectDenied = String(error).includes('QUOD_MANAGED_UNSUPPORTED: intrinsic mutation then')
      }
      try { fetch('https://example.com') } catch (error) { fetchDenied = String(error).includes('QUOD_MANAGED_UNSUPPORTED: global fetch') }
      check('Object descriptor writes remain sticky unsupported', objectDenied)
      check('Reflect descriptor writes remain sticky unsupported', reflectDenied)
      check('host fetch remains sticky unsupported', fetchDenied)
      await runtime.complete()
      completed = true
      const data = runtime.finish({checks, policy})
      check('new contract is explicit in returned boundary', data.boundary.contract === contract)
      check('native failed writes add no undeclared unsupported marker', JSON.stringify(data.boundary.unsupported.slice().sort()) ===
        JSON.stringify(['intrinsic mutation constructor', 'intrinsic mutation then', 'global fetch'].sort()))
      check('all native and managed effects complete under the new policy', data.boundary.completion.sealed &&
        data.boundary.pendingTimers === 0 && data.boundary.completion.pendingTimerCallbacks === 0 &&
        data.boundary.completion.pendingMicrotasks === 0 && data.boundary.completion.pendingEventCallbacks === 0 &&
        data.boundary.timerErrors.length === 0 && data.boundary.unhandledErrors.length === 0)
      data.ok = checks.every(row => row.pass)
      return runtime.response(data)
    } catch (error) {
      if (!completed) await runtime.complete()
      return runtime.response(runtime.finish({ok: false, checks, policy, error: String(error?.stack || error)}))
    }
  },
}
