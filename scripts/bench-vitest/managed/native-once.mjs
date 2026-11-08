// Opt-in research scheduling only. This builtin-only controller never executes
// a second native request, including after a rejected or unsuccessful first run.
import assert from 'node:assert/strict'
import crypto from 'node:crypto'

const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const requestBytesMax = 4 * 1024 * 1024

function canonicalRequest(text) {
  assert(typeof text === 'string' && Buffer.byteLength(text) > 0 &&
    Buffer.byteLength(text) <= requestBytesMax, 'bounded canonical native request required')
  const value = JSON.parse(text)
  function visit(item, depth = 0) {
    assert(depth <= 64, 'native request nesting exceeds bound')
    if (item === null || typeof item === 'string' || typeof item === 'boolean') return item
    if (typeof item === 'number') {
      assert(Number.isFinite(item) && !Object.is(item, -0), 'noncanonical native request number')
      return item
    }
    if (Array.isArray(item)) return item.map(value => visit(value, depth + 1))
    assert(item && typeof item === 'object', 'native request must contain JSON values')
    const result = Object.create(null)
    for (const key of Object.keys(item).sort()) result[key] = visit(item[key], depth + 1)
    return result
  }
  assert(value && typeof value === 'object' && !Array.isArray(value), 'native request object required')
  assert.equal(JSON.stringify(visit(value)), text, 'native request is not exact canonical JSON')
}

export function createNativeOnce({run}) {
  assert(typeof run === 'function', 'native once requires one trusted runner')
  let request, promise, state = 'unstarted', status = null, joins = 0
  let reservedMonoMs = null, settledMonoMs = null
  const snapshot = () => ({schema: 'quod-native-once-v1-research', state,
    starts: request === undefined ? 0 : 1, joins, requestSha256: request === undefined ? null : sha(request),
    reservedMonoMs, settledMonoMs, status})
  function start(text) {
    canonicalRequest(text)
    if (request !== undefined) {
      assert.equal(text, request, 'native once cannot change its reserved request')
      return promise
    }
    // Reserve synchronously. Defer run to a microtask so even a synchronous
    // exception/reentrant caller sees the one already-installed handled promise.
    request = text; state = 'running'; reservedMonoMs = performance.now()
    promise = Promise.resolve().then(() => run(request)).then(value => {
      state = 'settled'; status = 'fulfilled'; settledMonoMs = performance.now()
      return {status: 'fulfilled', value}
    }, reason => {
      state = 'settled'; status = 'rejected'; settledMonoMs = performance.now()
      return {status: 'rejected', reason}
    })
    return promise
  }
  function join() {joins++; return promise || null}
  return Object.freeze({start, join, snapshot})
}
