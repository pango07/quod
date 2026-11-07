// Research runtime only: no cache admission is implemented here.
import {AsyncLocalStorage} from 'node:async_hooks'

const options = __QUOD_CONFIG__
const NativeDate = Date, NativeResponse = Response
const wake = globalThis.setTimeout.bind(globalThis)
const cancelWake = globalThis.clearTimeout.bind(globalThis)
const unsupported = new Set(), logs = [], timerErrors = []
const inventory = Object.getOwnPropertyNames(globalThis).sort()
let tick = 0, randomState = options.seed >>> 0, timerId = 0, pendingWake, active = false
const timers = new Map()
export function deny(name) {
  unsupported.add(String(name))
  throw new Error(`QUOD_MANAGED_UNSUPPORTED: ${name}`)
}

function ManagedDate(...args) {
  if (!new.target) return new NativeDate(options.epoch + tick).toString()
  return Reflect.construct(NativeDate, args.length ? args : [options.epoch + tick], new.target)
}
ManagedDate.prototype = NativeDate.prototype
Object.defineProperty(ManagedDate.prototype, 'constructor', {value: ManagedDate, writable: true, configurable: true})
Object.assign(ManagedDate, {now: () => options.epoch + tick, parse: NativeDate.parse, UTC: NativeDate.UTC})
globalThis.Date = ManagedDate
Math.random = () => {
  randomState = (Math.imul(randomState, 1664525) + 1013904223) >>> 0
  return randomState / 0x100000000
}
Object.defineProperty(globalThis, 'performance', {value: Object.freeze({now: () => tick, timeOrigin: options.epoch}), configurable: true})

function pump() {
  pendingWake = undefined
  const next = [...timers.values()].sort((a, b) => a.at - b.at || a.id - b.id)[0]
  if (!next) return
  tick = next.at
  if (next.repeat === null) timers.delete(next.id)
  else next.at += Math.max(1, next.repeat)
  try { next.callback(...next.args) } catch (error) { timerErrors.push(String(error?.stack || error)) }
  schedule()
}
function schedule() {
  if (active && timers.size && pendingWake === undefined) pendingWake = wake(pump, 0)
}
function addTimer(callback, delay = 0, repeat = null, args = []) {
  if (typeof callback !== 'function') return deny('string timer')
  const ms = Math.max(0, Number(delay) || 0), id = ++timerId
  timers.set(id, {id, at: tick + ms, repeat, callback: AsyncLocalStorage.bind(callback), args})
  schedule()
  return id
}
function clearTimer(id) { timers.delete(id) }
Object.assign(globalThis, {
  setTimeout: (fn, ms, ...args) => addTimer(fn, ms, null, args),
  setInterval: (fn, ms = 0, ...args) => addTimer(fn, ms, Math.max(1, Number(ms) || 0), args),
  clearTimeout: clearTimer, clearInterval: clearTimer,
  setImmediate: (fn, ...args) => addTimer(fn, 0, null, args), clearImmediate: clearTimer,
})
const consoleMethods = ['log', 'info', 'warn', 'error', 'debug', 'trace', 'dir', 'assert', 'time', 'timeEnd', 'timeLog']
globalThis.console = Object.fromEntries(consoleMethods.map(name => [name, (...args) => {
  logs.push([name, args.map(value => typeof value === 'string' ? value : String(value))])
}]))
globalThis.process = Object.freeze({env: Object.freeze({...options.env}), nextTick: (fn, ...args) => queueMicrotask(() => fn(...args))})
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
  'setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'setImmediate', 'clearImmediate']
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
if (options.vitest) globalThis.__vitest_worker__ = {
  config: options.vitest, ctx: {pool: 'quod-managed'}, providedContext: {},
  environment: {name: 'quod-managed', viteEnvironment: 'ssr'},
}

export const runtime = Object.freeze({
  options,
  begin() { active = true; schedule() },
  finish(data) {
    active = false
    if (pendingWake !== undefined) cancelWake(pendingWake)
    pendingWake = undefined
    return {...data, boundary: {inventory, guarded: [...new Set(guarded)].sort(), unguardable,
      unsupported: [...unsupported], timerErrors, pendingTimers: timers.size, virtualTime: tick,
      logs, replayEligible: false, contract: 'quod-managed-v0-investigation'}}
  },
  response(data) { return NativeResponse.json(data) },
})
