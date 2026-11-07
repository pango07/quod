// Research runtime only: no cache admission is implemented here.
import {AsyncLocalStorage} from 'node:async_hooks'

const options = __QUOD_CONFIG__
// This controller shares a realm with tests. Keep its bookkeeping independent
// of prototypes a test (or a dependency) is allowed to replace temporarily.
const NativeDate = Date, NativeResponse = Response, NativePromise = Promise
const NativeString = String, NativeNumber = Number, NativeError = Error
const freeze = Object.freeze, preventExtensions = Object.preventExtensions
const ownKeys = Reflect.ownKeys, getPrototype = Object.getPrototypeOf, isExtensible = Object.isExtensible
const sameValue = Object.is, nativeDefineProperties = Object.defineProperties
const nativeSetPrototype = Object.setPrototypeOf, nativeReflectDefine = Reflect.defineProperty
const nativeReflectSetPrototype = Reflect.setPrototypeOf
const arrayPush = Function.prototype.call.bind(Array.prototype.push)
const arraySort = Function.prototype.call.bind(Array.prototype.sort)
const arrayIsArray = Array.isArray
const objectKeys = Object.keys, ownDescriptor = Object.getOwnPropertyDescriptor, defineOwn = Object.defineProperty
const setAdd = Function.prototype.call.bind(Set.prototype.add)
const setForEach = Function.prototype.call.bind(Set.prototype.forEach)
const mapSet = Function.prototype.call.bind(Map.prototype.set)
const mapDelete = Function.prototype.call.bind(Map.prototype.delete)
const mapGet = Function.prototype.call.bind(Map.prototype.get)
const mapHas = Function.prototype.call.bind(Map.prototype.has)
const setHas = Function.prototype.call.bind(Set.prototype.has)
const mapForEach = Function.prototype.call.bind(Map.prototype.forEach)
const mapSize = Function.prototype.call.bind(Object.getOwnPropertyDescriptor(Map.prototype, 'size').get)
const reflectConstruct = Reflect.construct, reflectApply = Reflect.apply, imul = Math.imul, max = Math.max
const jsonStringify = JSON.stringify
const promiseResolve = Promise.resolve.bind(Promise)
const promiseThen = Function.prototype.call.bind(Promise.prototype.then)
const bindAsyncContext = AsyncLocalStorage.bind.bind(AsyncLocalStorage)
const alsRun = Function.prototype.call.bind(AsyncLocalStorage.prototype.run)
const alsGetStore = Function.prototype.call.bind(AsyncLocalStorage.prototype.getStore)
const alsExit = Function.prototype.call.bind(AsyncLocalStorage.prototype.exit)
const alsDisable = Function.prototype.call.bind(AsyncLocalStorage.prototype.disable)
const alsEnterWith = AsyncLocalStorage.prototype.enterWith && Function.prototype.call.bind(AsyncLocalStorage.prototype.enterWith)
const nativeStorage = new WeakMap()
const weakGet = Function.prototype.call.bind(WeakMap.prototype.get)
const weakSet = Function.prototype.call.bind(WeakMap.prototype.set)
const wake = globalThis.setTimeout.bind(globalThis)
const cancelWake = globalThis.clearTimeout.bind(globalThis)
const nativeMicrotask = globalThis.queueMicrotask.bind(globalThis)
const listen = globalThis.addEventListener?.bind(globalThis)
const eventAdd = Function.prototype.call.bind(EventTarget.prototype.addEventListener)
const eventRemove = Function.prototype.call.bind(EventTarget.prototype.removeEventListener)
const eventDispatch = Function.prototype.call.bind(EventTarget.prototype.dispatchEvent)
const preventEvent = Function.prototype.call.bind(Event.prototype.preventDefault)
const unsupported = new Set(), logs = [], timerErrors = [], unhandledErrors = []
const inventory = Object.getOwnPropertyNames(globalThis).sort()
let tick = 0, randomState = options.seed >>> 0, timerId = 0, pendingWake, active = false
let timerCallbacks = 0, pendingTimerCallbacks = 0, pendingMicrotasks = 0, pendingEventCallbacks = 0
let locked = false, completionRequested = false, finished = false
const integrity = [], mutableSlots = [], protectedObjects = new Set()
let completion = {sealed: false, nativeTurnFences: 0, reason: 'completion was not requested'}
const timers = new Map()
// Test modules receive this facade from the compiler, never the runtime's
// native class or constructor. Native context state remains private.
export class ManagedAsyncLocalStorage {
  constructor() { weakSet(nativeStorage, this, new AsyncLocalStorage()) }
  run(store, callback, ...args) { return alsRun(weakGet(nativeStorage, this), store, () => reflectApply(callback, undefined, args)) }
  getStore() { return alsGetStore(weakGet(nativeStorage, this)) }
  exit(callback, ...args) { return alsExit(weakGet(nativeStorage, this), () => reflectApply(callback, undefined, args)) }
  disable() { return alsDisable(weakGet(nativeStorage, this)) }
  enterWith(store) {
    if (!alsEnterWith) return deny('AsyncLocalStorage.enterWith')
    return alsEnterWith(weakGet(nativeStorage, this), store)
  }
  static bind(callback) { return bindAsyncContext(callback) }
  static snapshot() {
    const bound = bindAsyncContext((callback, args) => reflectApply(callback, undefined, args))
    return (callback, ...args) => bound(callback, args)
  }
}
Object.freeze(ManagedAsyncLocalStorage.prototype)
Object.freeze(ManagedAsyncLocalStorage)
export function deny(name) {
  const value = NativeString(name)
  setAdd(unsupported, value)
  throw new NativeError(`QUOD_MANAGED_UNSUPPORTED: ${value}`)
}

function describeError(error) {
  try { return NativeString(error?.stack || error) }
  catch { return 'Error with an unreadable stack or value' }
}
// These native notifications run after the microtask checkpoint. A fresh native
// turn is therefore part of the completion protocol, even when no timers remain.
// The handlers are registered privately before tests can add/remove listeners.
if (listen) {
  listen('unhandledrejection', event => {
    arrayPush(unhandledErrors, describeError(event.reason))
    preventEvent(event)
  })
  listen('error', event => {
    arrayPush(unhandledErrors, describeError(event.error || event.message))
    preventEvent(event)
  })
}

function ManagedDate(...args) {
  if (!new.target) return new NativeDate(options.epoch + tick).toString()
  return reflectConstruct(NativeDate, args.length ? args : [options.epoch + tick], new.target)
}
ManagedDate.prototype = NativeDate.prototype
Object.defineProperty(ManagedDate.prototype, 'constructor', {value: ManagedDate, writable: true, configurable: true})
Object.assign(ManagedDate, {now: () => options.epoch + tick, parse: NativeDate.parse, UTC: NativeDate.UTC})
globalThis.Date = ManagedDate
Math.random = () => {
  randomState = (imul(randomState, 1664525) + 1013904223) >>> 0
  return randomState / 0x100000000
}
Object.defineProperty(globalThis, 'performance', {value: Object.freeze({now: () => tick, timeOrigin: options.epoch}), configurable: true})

function pump() {
  pendingWake = undefined
  const ordered = []
  mapForEach(timers, timer => arrayPush(ordered, timer))
  const next = arraySort(ordered, (a, b) => a.at - b.at || a.id - b.id)[0]
  if (!next) return
  tick = next.at
  if (next.repeat === null) mapDelete(timers, next.id)
  else next.at += max(1, next.repeat)
  timerCallbacks++
  try {
    // A detached async timer callback must settle too. Do not await it here:
    // its continuation may itself need one of the managed timers to fire.
    const result = reflectApply(next.callback, undefined, next.args)
    if (result && (typeof result === 'object' || typeof result === 'function')) {
      observeAsync(result, 'timer')
    }
  } catch (error) { arrayPush(timerErrors, describeError(error)) }
  schedule()
}
function schedule() {
  if (active && mapSize(timers) && pendingWake === undefined) pendingWake = wake(pump, 0)
}
function addTimer(callback, delay = 0, repeat = null, args = []) {
  if (typeof callback !== 'function') return deny('string timer')
  const ms = max(0, NativeNumber(delay) || 0), id = ++timerId
  mapSet(timers, id, {id, at: tick + ms, repeat, callback: bindAsyncContext(callback), args,
    scheduledAt: describeError(new NativeError('Managed timer scheduled here'))})
  schedule()
  return id
}
function clearTimer(id) { mapDelete(timers, id) }
function observeAsync(result, kind, alreadyPending = false) {
  const change = delta => {
    if (kind === 'timer') pendingTimerCallbacks += delta
    else if (kind === 'event') pendingEventCallbacks += delta
    else pendingMicrotasks += delta
  }
  if (!alreadyPending) change(1)
  try {
    promiseThen(promiseResolve(result), () => change(-1), error => {
      change(-1)
      arrayPush(kind === 'timer' ? timerErrors : unhandledErrors, describeError(error))
    })
  } catch (error) {
    change(-1)
    arrayPush(kind === 'timer' ? timerErrors : unhandledErrors, describeError(error))
  }
}
function managedMicrotask(callback) {
  if (typeof callback !== 'function') return deny('non-function microtask')
  pendingMicrotasks++
  const bound = bindAsyncContext(callback)
  nativeMicrotask(() => {
    try { observeAsync(bound(), 'microtask', true) }
    catch (error) { pendingMicrotasks--; arrayPush(unhandledErrors, describeError(error)) }
  })
}
// Wrapping only globalThis.addEventListener leaves the prototype .call bypass.
// Native local EventTarget delivery remains synchronous; returned promises and
// listener exceptions join the same private completion accounting as timers.
const eventListeners = new WeakMap()
function eventWrapper(target, type, callback, capture) {
  let byType = weakGet(eventListeners, target)
  if (!byType) weakSet(eventListeners, target, byType = new Map())
  let byCallback = mapGet(byType, type)
  if (!byCallback) mapSet(byType, type, byCallback = new WeakMap())
  let pair = weakGet(byCallback, callback)
  if (!pair) weakSet(byCallback, callback, pair = [])
  const index = capture ? 1 : 0
  if (!pair[index]) pair[index] = function (event) {
    try {
      const handler = typeof callback === 'function' ? callback : callback.handleEvent
      const result = reflectApply(handler, typeof callback === 'function' ? this : callback, [event])
      if (result && (typeof result === 'object' || typeof result === 'function')) observeAsync(result, 'event')
    } catch (error) { arrayPush(unhandledErrors, describeError(error)) }
  }
  return pair[index]
}
defineOwn(EventTarget.prototype, 'addEventListener', {configurable: false, writable: false,
  value: function (type, callback, settings) {
    if (this === globalThis) return deny('global EventTarget.addEventListener')
    if (callback === null || callback === undefined) return eventAdd(this, type, callback, settings)
    if (typeof callback !== 'function' && typeof callback !== 'object') return eventAdd(this, type, callback, settings)
    const name = NativeString(type), capture = typeof settings === 'boolean' ? settings : !!settings?.capture
    return eventAdd(this, name, eventWrapper(this, name, callback, capture), settings)
  }})
defineOwn(EventTarget.prototype, 'removeEventListener', {configurable: false, writable: false,
  value: function (type, callback, settings) {
    if (this === globalThis) return deny('global EventTarget.removeEventListener')
    if (callback === null || callback === undefined) return eventRemove(this, type, callback, settings)
    if (typeof callback !== 'function' && typeof callback !== 'object') return eventRemove(this, type, callback, settings)
    const name = NativeString(type), capture = typeof settings === 'boolean' ? settings : !!settings?.capture
    const byType = weakGet(eventListeners, this), byCallback = byType && mapGet(byType, name)
    const pair = byCallback && weakGet(byCallback, callback)
    return eventRemove(this, name, pair?.[capture ? 1 : 0] || callback, settings)
  }})
defineOwn(EventTarget.prototype, 'dispatchEvent', {configurable: false, writable: false,
  value: function (event) {
    if (this === globalThis) return deny('global EventTarget.dispatchEvent')
    return eventDispatch(this, event)
  }})
Object.assign(globalThis, {
  setTimeout: (fn, ms, ...args) => addTimer(fn, ms, null, args),
  setInterval: (fn, ms = 0, ...args) => addTimer(fn, ms, max(1, NativeNumber(ms) || 0), args),
  clearTimeout: clearTimer, clearInterval: clearTimer,
  setImmediate: (fn, ...args) => addTimer(fn, 0, null, args), clearImmediate: clearTimer,
  queueMicrotask: managedMicrotask,
})
const consoleMethods = ['log', 'info', 'warn', 'error', 'debug', 'trace', 'dir', 'assert', 'time', 'timeEnd', 'timeLog']
globalThis.console = Object.fromEntries(consoleMethods.map(name => [name, (...args) => {
  const values = []
  for (let index = 0; index < args.length; index++) arrayPush(values, NativeString(args[index]))
  arrayPush(logs, [name, values])
}]))
globalThis.process = Object.freeze({env: Object.freeze({...options.env}), nextTick: (fn, ...args) => managedMicrotask(() => reflectApply(fn, undefined, args))})
Object.defineProperty(globalThis, 'navigator', {value: Object.freeze({userAgent: 'QuodManaged/0',
  platform: 'QuodManaged', language: 'en-US', languages: Object.freeze(['en-US']), hardwareConcurrency: 1}), configurable: true})
// Local synchronous events are used by Chai and Vitest. Their native timestamp
// would expose host time; guard that observation before any framework import.
Object.defineProperty(Event.prototype, 'timeStamp', {configurable: false, get: () => deny('Event.timeStamp')})
// Closed-world candidate. An unlisted global is guarded at each exposed
// prototype as well, to avoid leaving a callable alias on WorkerGlobalScope.
const allowed = new Set([
  'globalThis', 'self', 'global', 'undefined', 'NaN', 'Infinity',
  'Object', 'Function', 'Array', 'Number', 'Boolean', 'String', 'Symbol', 'BigInt',
  'Date', 'Math', 'JSON', 'Reflect', 'Proxy', 'Promise', 'Map', 'Set', 'WeakMap', 'WeakSet',
  'RegExp', 'Error', 'EvalError', 'RangeError', 'ReferenceError', 'SyntaxError', 'TypeError', 'URIError', 'AggregateError',
  'ArrayBuffer', 'DataView', 'Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array',
  'Int32Array', 'Uint32Array', 'Float16Array', 'Float32Array', 'Float64Array', 'BigInt64Array', 'BigUint64Array',
  'parseInt', 'parseFloat', 'isNaN', 'isFinite', 'decodeURI', 'decodeURIComponent', 'encodeURI', 'encodeURIComponent',
  'escape', 'unescape', 'eval', 'Iterator', 'AsyncIterator',
  'URL', 'URLSearchParams', 'TextEncoder', 'TextDecoder', 'atob', 'btoa', 'structuredClone',
  'AbortController', 'AbortSignal', 'DOMException', 'Event', 'EventTarget', 'queueMicrotask',
  'setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'setImmediate', 'clearImmediate',
  'performance', 'console', 'process', 'navigator',
])
const managedGlobals = new Map(['Date', 'performance', 'console', 'process', 'navigator',
  'setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'setImmediate', 'clearImmediate', 'queueMicrotask']
  .map(key => [key, globalThis[key]]))
const guarded = [], unguardable = []
for (let owner = globalThis; owner && owner !== Object.prototype; owner = Object.getPrototypeOf(owner)) {
  for (const key of Object.getOwnPropertyNames(owner)) {
    // Installing an own property must not leave the original host API
    // reachable through a descriptor on WorkerGlobalScope's prototype.
    if (owner !== globalThis && managedGlobals.has(key)) {
      const descriptor = Object.getOwnPropertyDescriptor(owner, key)
      if (!descriptor?.configurable) { unguardable.push('native alias ' + key); continue }
      Object.defineProperty(owner, key, {value: managedGlobals.get(key), configurable: false})
      continue
    }
    if (owner === EventTarget.prototype && (key === 'addEventListener' || key === 'removeEventListener' || key === 'dispatchEvent')) continue
    if (allowed.has(key) || key === 'constructor') continue
    const descriptor = Object.getOwnPropertyDescriptor(owner, key)
    if (!descriptor?.configurable) { unguardable.push(key); continue }
    Object.defineProperty(owner, key, {configurable: false, get: () => deny(`global ${key}`)})
    guarded.push(key)
  }
}
if (typeof AbortSignal === 'function') {
  Object.defineProperty(AbortSignal, 'timeout', {value: () => deny('AbortSignal.timeout'), configurable: false})
}
for (const key of ['fetch', 'crypto', 'WeakRef', 'FinalizationRegistry', 'WebAssembly', 'SharedArrayBuffer', 'Atomics']) {
  if (!Object.getOwnPropertyDescriptor(globalThis, key)) {
    Object.defineProperty(globalThis, key, {configurable: false, get: () => deny(`global ${key}`)})
  }
}
globalThis.global = globalThis
// Disabling the global names is insufficient: each kind of function exposes a
// constructor alias through its prototype. Seal all four before module imports.
const functionKinds = [
  [Function, function Function() { return deny('Function constructor') }],
  [Object.getPrototypeOf(async function () {}).constructor, function AsyncFunction() { return deny('AsyncFunction constructor') }],
  [Object.getPrototypeOf(function* () {}).constructor, function GeneratorFunction() { return deny('GeneratorFunction constructor') }],
  [Object.getPrototypeOf(async function* () {}).constructor, function AsyncGeneratorFunction() { return deny('AsyncGeneratorFunction constructor') }],
]
for (const [NativeFunction, ManagedFunction] of functionKinds) {
  Object.defineProperty(ManagedFunction, 'prototype', {value: NativeFunction.prototype})
  Object.defineProperty(NativeFunction.prototype, 'constructor', {value: ManagedFunction, configurable: false, writable: false})
}
globalThis.Function = functionKinds[0][1]
globalThis.eval = () => deny('eval')
if (options.vitest) globalThis.__vitest_worker__ = {
  config: options.vitest, ctx: {pool: 'quod-managed'}, providedContext: {},
  environment: {name: 'quod-managed', viteEnvironment: 'ssr'},
}

// Lock the supported public intrinsics only after framework bootstrap. Tests
// cannot mutate/use/restore one of these methods between receipt checks. The
// narrow mutable slots are used by the pinned Vitest spy/stack implementation;
// their original descriptors must be restored before completion can seal.
function mutableSlot(target, key) {
  return target === ManagedDate && key === 'now' || target === NativeError &&
    (key === 'stackTraceLimit' || key === 'prepareStackTrace')
}
function protectedMutation(target, key) {
  if (setHas(protectedObjects, target) && !mutableSlot(target, key)) deny('intrinsic mutation ' + NativeString(key))
}
// Instrument the public descriptor mutation entry points too: a caught attempt
// to replace a locked method remains unsupported, rather than losing the flag
// when the engine's TypeError is caught. Private captures remain unexposed.
Object.defineProperty = (target, key, descriptor) => {
  protectedMutation(target, key)
  return defineOwn(target, key, descriptor)
}
Object.defineProperties = (target, descriptors) => {
  if (setHas(protectedObjects, target)) {
    const keys = ownKeys(descriptors)
    for (let index = 0; index < keys.length; index++) protectedMutation(target, keys[index])
  }
  return nativeDefineProperties(target, descriptors)
}
Reflect.defineProperty = (target, key, descriptor) => {
  protectedMutation(target, key)
  return nativeReflectDefine(target, key, descriptor)
}
Object.setPrototypeOf = (target, prototype) => {
  if (setHas(protectedObjects, target) && getPrototype(target) !== prototype) deny('intrinsic prototype mutation')
  return nativeSetPrototype(target, prototype)
}
Reflect.setPrototypeOf = (target, prototype) => {
  if (setHas(protectedObjects, target) && getPrototype(target) !== prototype) deny('intrinsic prototype mutation')
  return nativeReflectSetPrototype(target, prototype)
}
function descriptorEqual(left, right) {
  if (!left || !right) return left === right
  return sameValue(left.value, right.value) && left.get === right.get && left.set === right.set &&
    left.writable === right.writable && left.configurable === right.configurable && left.enumerable === right.enumerable
}
function lockdown() {
  if (locked) return
  // Workerd may omit one of V8's stack-formatting properties. Reserve it before
  // preventing extensions so the pinned framework can restore an undefined slot.
  for (const key of ['stackTraceLimit', 'prepareStackTrace']) {
    if (!ownDescriptor(NativeError, key)) defineOwn(NativeError, key, {value: undefined, writable: true, configurable: true})
  }
  const queue = []
  function discover(value) {
    if (!value || (typeof value !== 'object' && typeof value !== 'function') || value === globalThis || setHas(protectedObjects, value)) return
    setAdd(protectedObjects, value)
    arrayPush(queue, value)
  }
  setForEach(allowed, name => discover(globalThis[name]))
  discover(ManagedAsyncLocalStorage)
  for (let index = 0; index < functionKinds.length; index++) {
    discover(functionKinds[index][0].prototype)
    discover(functionKinds[index][1])
  }
  // Some intrinsics have no global name (TypedArray and async/generator
  // prototypes). Reach them through the declared constructors and prototypes.
  for (let index = 0; index < queue.length; index++) {
    const target = queue[index], keys = ownKeys(target)
    discover(getPrototype(target))
    for (let keyIndex = 0; keyIndex < keys.length; keyIndex++) {
      const descriptor = ownDescriptor(target, keys[keyIndex])
      discover(descriptor.value); discover(descriptor.get); discover(descriptor.set)
    }
  }
  for (let index = 0; index < queue.length; index++) {
    const target = queue[index], keys = ownKeys(target)
    for (let keyIndex = 0; keyIndex < keys.length; keyIndex++) {
      const key = keys[keyIndex], descriptor = ownDescriptor(target, key)
      if (mutableSlot(target, key)) {
        arrayPush(mutableSlots, {target, key, descriptor})
      } else if (descriptor.configurable && 'value' in descriptor && descriptor.writable) {
        const value = descriptor.value
        const get = freeze(() => value), set = freeze(() => deny('intrinsic mutation ' + NativeString(key)))
        defineOwn(target, key, {get, set, enumerable: descriptor.enumerable, configurable: false})
      } else if (descriptor.configurable || descriptor.writable) {
        defineOwn(target, key, 'value' in descriptor ? {...descriptor, writable: false, configurable: false}
          : {...descriptor, configurable: false})
      }
    }
    preventExtensions(target)
    arrayPush(integrity, {target, prototype: getPrototype(target), keys: ownKeys(target)})
  }
  // Keep bindings to intrinsics and deterministic host replacements fixed too.
  // Vitest's own worker/matcher state is deliberately outside this public
  // intrinsic policy; v0 therefore still grants no replay permission.
  setForEach(allowed, name => {
    const descriptor = ownDescriptor(globalThis, name)
    if (descriptor && !descriptor.configurable) return
    const value = globalThis[name]
    defineOwn(globalThis, name, {get: freeze(() => value), set: freeze(() => deny('global mutation ' + name)),
      enumerable: descriptor?.enumerable || false, configurable: false})
  })
  locked = true
}
function checkIntegrity() {
  if (!locked) return 'intrinsic lockdown was not requested'
  for (let index = 0; index < integrity.length; index++) {
    const row = integrity[index], keys = ownKeys(row.target)
    if (getPrototype(row.target) !== row.prototype || isExtensible(row.target) || keys.length !== row.keys.length) return 'intrinsic shape changed'
    for (let keyIndex = 0; keyIndex < keys.length; keyIndex++) if (keys[keyIndex] !== row.keys[keyIndex]) return 'intrinsic keys changed'
  }
  for (let index = 0; index < mutableSlots.length; index++) {
    const row = mutableSlots[index]
    if (!descriptorEqual(ownDescriptor(row.target, row.key), row.descriptor)) return 'mutable intrinsic slot was not restored: ' + NativeString(row.key)
  }
  return ''
}

// Native Response.json consults toJSON on values and prototypes. A test can
// modify those hooks. Serialize own data descriptors ourselves, invoking the
// captured JSON implementation only for primitive strings/numbers.
function serialize(value, ancestors = []) {
  if (value === null || typeof value === 'boolean' || typeof value === 'number' || typeof value === 'string') return jsonStringify(value)
  if (value === undefined) return 'null'
  if (typeof value !== 'object') throw new NativeError('Non-data value in managed receipt')
  for (let index = 0; index < ancestors.length; index++) if (ancestors[index] === value) throw new NativeError('Cyclic managed receipt')
  arrayPush(ancestors, value)
  let result = ''
  const keys = objectKeys(value), array = arrayIsArray(value)
  for (let index = 0; index < keys.length; index++) {
    const key = keys[index], descriptor = ownDescriptor(value, key)
    if (!descriptor || !('value' in descriptor)) throw new NativeError('Accessor in managed receipt')
    if (!array && descriptor.value === undefined) continue
    if (result) result += ','
    result += (array ? '' : jsonStringify(key) + ':') + serialize(descriptor.value, ancestors)
  }
  ancestors.length--
  return (array ? '[' : '{') + result + (array ? ']' : '}')
}

export const runtime = freeze({
  options,
  begin() {
    if (active || completionRequested || finished) return deny('runtime execution reuse')
    active = true
    schedule()
  },
  lockdown,
  async complete() {
    if (completionRequested) return deny('runtime completion reuse')
    completionRequested = true
    const started = timerCallbacks
    let quiet = 0, nativeTurnFences = 0, reason = 'completion budget exhausted'
    // A perpetual interval, detached callback awaiting forever, or unbounded
    // work is unsupported. The supervising process also enforces wall time.
    for (; nativeTurnFences < 512; nativeTurnFences++) {
      const fence = new NativePromise(resolve => wake(resolve, 0))
      // Await's PromiseResolve checks .constructor before taking the intrinsic
      // fast path. Own immutable data prevents replacement of its native fence.
      defineOwn(fence, 'constructor', {value: NativePromise})
      await fence
      if (timerCallbacks - started > 4096) { reason = 'completion callback budget exhausted'; break }
      if (mapSize(timers) || pendingTimerCallbacks || pendingMicrotasks || pendingEventCallbacks) quiet = 0
      else if (++quiet === 2) { reason = ''; nativeTurnFences++; break }
    }
    const integrityReason = checkIntegrity()
    if (integrityReason) reason = integrityReason
    if (timerErrors.length || unhandledErrors.length) reason = 'asynchronous errors'
    if (!listen) reason = 'native asynchronous error observation unavailable'
    completion = {sealed: !reason, nativeTurnFences, timerCallbacks,
      pendingTimerCallbacks, pendingMicrotasks, pendingEventCallbacks,
      intrinsicLockdown: locked, protectedIntrinsicObjects: integrity.length, reason}
    if (reason) setAdd(unsupported, 'incomplete async effects: ' + reason)
    return freeze({...completion})
  },
  finish(data) {
    if (finished) return deny('runtime finish reuse')
    finished = true
    active = false
    if (pendingWake !== undefined) cancelWake(pendingWake)
    pendingWake = undefined
    const integrityReason = checkIntegrity()
    if (integrityReason || !completionRequested || mapSize(timers) || pendingTimerCallbacks || pendingMicrotasks || pendingEventCallbacks || timerErrors.length || unhandledErrors.length) {
      const reason = integrityReason || completion.reason || 'effects changed after completion or completion was not requested'
      completion = {...completion, sealed: false, reason}
      setAdd(unsupported, 'incomplete async effects: ' + reason)
    }
    const unsupportedValues = [], pendingTimerDetails = []
    setForEach(unsupported, value => arrayPush(unsupportedValues, value))
    mapForEach(timers, timer => arrayPush(pendingTimerDetails, {id: timer.id, at: timer.at,
      repeat: timer.repeat, scheduledAt: timer.scheduledAt}))
    const asyncFailure = timerErrors.length || unhandledErrors.length
    return {...data, ok: asyncFailure ? false : data.ok, boundary: {inventory, guarded, unguardable,
      unsupported: unsupportedValues, timerErrors, unhandledErrors, pendingTimers: mapSize(timers),
      pendingTimerDetails, completion, virtualTime: tick,
      logs, replayEligible: false, contract: 'quod-managed-v0-investigation'}}
  },
  response(data) { return new NativeResponse(serialize(data), {status: 200, statusText: '', headers: {'content-type': 'application/json'}}) },
})
