// Standalone research ledger used only by the fresh-runner replay probe.
// Production replay remains disabled.
// Preparation/compilation precede lookup; a trusted seed-job byte digest is
// mandatory. This module accesses only the explicit receipt path supplied.
import fs from 'node:fs'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'

const EXECUTION_SCHEMA = 'quod-managed-execution-v1-research'
const RECEIPT_SCHEMA = 'quod-managed-receipt-v1-research'
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const digest = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value)
const integer = value => Number.isSafeInteger(value) && value >= 0

function plain(value, label) {
  assert(value && typeof value === 'object' && !Array.isArray(value), label + ' must be an object')
  assert(Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null,
    label + ' must be plain own data')
}

// Serialize descriptors directly. No getters, symbols, inherited toJSON,
// undefined values, sparse arrays or arbitrary object prototypes participate.
export function canonicalReceiptJSON(value) {
  const ancestors = new Set()
  function encode(item) {
    if (item === null || typeof item === 'string' || typeof item === 'boolean') return JSON.stringify(item)
    if (typeof item === 'number') {
      assert(Number.isFinite(item) && !Object.is(item, -0), 'non-canonical JSON number')
      return JSON.stringify(item)
    }
    assert(item && typeof item === 'object', 'receipt requires JSON data')
    assert(!ancestors.has(item), 'cyclic receipt data')
    assert.equal(Object.getOwnPropertySymbols(item).length, 0, 'symbol receipt properties unsupported')
    ancestors.add(item)
    let result
    if (Array.isArray(item)) {
      assert.equal(Object.getOwnPropertyNames(item).length, item.length + 1, 'sparse/custom receipt array')
      const values = []
      for (let index = 0; index < item.length; index++) {
        const descriptor = Object.getOwnPropertyDescriptor(item, String(index))
        assert(descriptor?.enumerable && 'value' in descriptor, 'receipt array accessor')
        values.push(encode(descriptor.value))
      }
      result = '[' + values.join(',') + ']'
    } else {
      plain(item, 'receipt data')
      const values = []
      for (const key of Object.getOwnPropertyNames(item).sort()) {
        const descriptor = Object.getOwnPropertyDescriptor(item, key)
        assert(descriptor.enumerable && 'value' in descriptor, 'receipt object accessor/non-enumerable property')
        values.push(JSON.stringify(key) + ':' + encode(descriptor.value))
      }
      result = '{' + values.join(',') + '}'
    }
    ancestors.delete(item)
    return result
  }
  return encode(value)
}

function exactKeys(value, keys, label) {
  plain(value, label)
  assert.deepEqual(Object.keys(value).sort(), [...keys].sort(), label + ' has unexpected/missing fields')
}
function ownData(value) {
  // Besides validating recursively, this makes later ordinary property reads
  // safe. Validation never invokes accessors or user serialization callbacks.
  canonicalReceiptJSON(value)
  return value
}
function nonemptyData(value, label) {
  plain(value, label); ownData(value)
  assert(Object.keys(value).length > 0, label + ' must explicitly describe its policy')
}
function policies({runtimePolicy: runtime, supervisorPolicy: supervisor, host}) {
  for (const [label, value] of Object.entries({runtime, supervisor, host})) nonemptyData(value, label)
  for (const key of ['schema', 'workerdSha256', 'nativeFingerprintSha256', 'compatibilityDate', 'compatibilityFlags', 'denyModuleSha256',
    'moduleName', 'services', 'globalOutbound', 'bindings', 'processEnv', 'request', 'supervisor']) {
    assert(Object.hasOwn(runtime, key), 'runtime policy omitted ' + key)
  }
  assert.equal(runtime.schema, 'quod-managed-runtime-policy-v1-research', 'obsolete runtime policy')
  assert(digest(runtime.workerdSha256) && digest(runtime.nativeFingerprintSha256) && digest(runtime.denyModuleSha256), 'runtime/native byte identities required')
  assert(typeof runtime.compatibilityDate === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(runtime.compatibilityDate), 'explicit compatibility date required')
  stringArray(runtime.compatibilityFlags, 'compatibility flags')
  assert.equal(runtime.moduleName, 'worker.mjs', 'reviewed module name required')
  assert.deepEqual(runtime.services, ['main', 'deny'], 'reviewed service topology required')
  assert.equal(runtime.globalOutbound, 'deny', 'outbound deny policy required')
  assert(Array.isArray(runtime.bindings) && runtime.bindings.length === 0, 'managed bindings unsupported')
  exactKeys(runtime.processEnv, ['PATH', 'TZ', 'LANG'], 'runtime process environment')
  assert(typeof runtime.processEnv.PATH === 'string' && runtime.processEnv.PATH.length > 0, 'explicit runtime PATH required')
  assert.equal(runtime.processEnv.TZ, 'UTC'); assert.equal(runtime.processEnv.LANG, 'C.UTF-8')
  exactKeys(runtime.request, ['method', 'path'], 'runtime request')
  assert.equal(runtime.request.method, 'GET'); assert.equal(runtime.request.path, '/run')
  const control = runtime.supervisor
  for (const key of ['sourceSha256', 'nodeVersion', 'nodeExecutableSha256', 'watchdogMs', 'startupAttempts',
    'healthTimeoutMs', 'startupRetryMs', 'shutdownGraceMs', 'responseBytesMax', 'completionBounds']) {
    assert(Object.hasOwn(control, key), 'runtime supervisor omitted ' + key)
  }
  assert(digest(control.sourceSha256) && digest(control.nodeExecutableSha256), 'supervisor byte identities required')
  assert(typeof control.nodeVersion === 'string' && control.nodeVersion.length > 0, 'supervisor Node version required')
  for (const key of ['watchdogMs', 'startupAttempts', 'healthTimeoutMs', 'startupRetryMs', 'shutdownGraceMs', 'responseBytesMax']) {
    assert(integer(control[key]) && control[key] > 0, 'invalid supervisor bound ' + key)
  }
  exactKeys(control.completionBounds, ['nativeTurnFencesMax', 'timerCallbackBudget', 'timerCallbackBudgetScope', 'quietNativeTurns'], 'completion policy')
  assert.deepEqual(control.completionBounds, {nativeTurnFencesMax: 512, timerCallbackBudget: 4096,
    timerCallbackBudgetScope: 'callbacks after completion begins; reject when exceeded', quietNativeTurns: 2}, 'unreviewed completion policy')
  assert(digest(supervisor.receiptValidatorSha256) && digest(supervisor.experimentSha256), 'receipt and experiment source identities required')
  for (const key of ['ImageOS', 'ImageVersion', 'platform', 'arch', 'kernel', 'libc', 'nodeVersion']) {
    assert(Object.hasOwn(host, key), 'host class omitted ' + key)
  }
  for (const key of ['ImageOS', 'ImageVersion', 'platform', 'arch', 'nodeVersion']) {
    assert(typeof host[key] === 'string' && host[key].length > 0, 'host class requires ' + key)
  }
  exactKeys(host.kernel, ['release', 'version'], 'host kernel')
  assert(typeof host.kernel.release === 'string' && host.kernel.release.length > 0 &&
    typeof host.kernel.version === 'string' && host.kernel.version.length > 0, 'host kernel identity required')
  exactKeys(host.libc, ['family', 'runtimeVersion', 'compilerVersion'], 'host libc')
  assert(typeof host.libc.family === 'string' && host.libc.family !== 'unknown', 'known libc class required')
  for (const key of ['runtimeVersion', 'compilerVersion']) assert(host.libc[key] === null || typeof host.libc[key] === 'string', 'invalid libc version')
}
const executionKey = identity => hash(EXECUTION_SCHEMA + '\n' + canonicalReceiptJSON(identity))
function checkedExecution(execution) {
  ownData(execution)
  exactKeys(execution, ['identity', 'key'], 'execution descriptor')
  exactKeys(execution.identity, ['schema', 'compile', 'preparation', 'runtimePolicy', 'supervisorPolicy', 'host'], 'execution identity')
  assert.equal(execution.identity.schema, EXECUTION_SCHEMA, 'obsolete execution identity')
  assert(digest(execution.key) && execution.key === executionKey(execution.identity), 'execution key mismatch')
  for (const name of ['compile', 'preparation', 'runtimePolicy', 'supervisorPolicy', 'host']) {
    nonemptyData(execution.identity[name], 'execution ' + name)
  }
  policies(execution.identity)
  return execution
}

/**
 * runtimePolicy, supervisorPolicy and host are the complete effective policy
 * records built by the supervisor, not selected labels or measurements. Every
 * supplied field is keyed. No ambient policy or environment is read here.
 */
export function makeExecutionIdentity({built, prepared, runtimePolicy, supervisorPolicy, host}) {
  assert(typeof built?.bundle === 'string' && digest(built.sha256), 'actual compiled bundle required')
  assert.equal(hash(built.bundle), built.sha256, 'compiled bundle bytes mismatch')
  assert.equal(built.identity?.schema, 'quod-managed-compile-v1-research', 'obsolete compiler identity')
  assert.equal(built.identity.bundleSha256, built.sha256, 'compiler bundle identity mismatch')
  assert.equal(hash(canonicalReceiptJSON(built.identity)), built.identitySha256, 'compiler identity digest mismatch')
  assert.equal(prepared?.identity?.schema, 'quod-managed-preparation-v1-research', 'obsolete preparation identity')
  assert.equal(hash(canonicalReceiptJSON(prepared.config)), prepared.identity.configurationSha256,
    'resolved preparation configuration mismatch')
  policies({runtimePolicy, supervisorPolicy, host})
  const identity = JSON.parse(canonicalReceiptJSON({schema: EXECUTION_SCHEMA, compile: built.identity,
    preparation: prepared.identity, runtimePolicy, supervisorPolicy, host}))
  return {identity, key: executionKey(identity)}
}

function expectation(expected) {
  ownData(expected)
  exactKeys(expected, ['count', 'file'], 'expected workload')
  assert(Number.isSafeInteger(expected.count) && expected.count > 0, 'positive expected test count required')
  assert(typeof expected.file === 'string' && expected.file.length > 0, 'exact expected file name required')
  return expected
}
function stringArray(value, label, empty = false) {
  assert(Array.isArray(value) && value.every(item => typeof item === 'string'), label + ' must contain primitive strings')
  if (empty) assert.equal(value.length, 0, label + ' prevents reuse')
}

/** Throws on an unsupported, failed, incomplete, malformed or obsolete result. */
export function validatePassingResponse(data, expected) {
  ownData(data); expectation(expected)
  exactKeys(data, ['ok', 'rows', 'files', 'boundary'], 'worker response')
  assert.equal(data.ok, true, 'failing response cannot be reused')
  assert(Array.isArray(data.rows) && data.rows.length === expected.count, 'incomplete test rows')
  const names = new Set()
  for (const row of data.rows) {
    exactKeys(row, ['name', 'state', 'errors'], 'test row')
    assert(typeof row.name === 'string' && row.name.length > 0 && !names.has(row.name), 'invalid/duplicate test name')
    names.add(row.name)
    assert(['pass', 'skip', 'todo'].includes(row.state), 'non-passing test state prevents reuse')
    assert(Array.isArray(row.errors) && row.errors.length === 0, 'test errors prevent reuse')
  }
  assert(Array.isArray(data.files) && data.files.length === 1, 'one complete test file required')
  const file = data.files[0]
  exactKeys(file, ['name', 'state', 'errors'], 'file row')
  assert.equal(file.name, expected.file, 'unexpected test file')
  assert.equal(file.state, 'pass', 'non-passing file prevents reuse')
  assert(Array.isArray(file.errors) && file.errors.length === 0, 'file errors prevent reuse')
  const boundary = data.boundary
  exactKeys(boundary, ['inventory', 'guarded', 'unguardable', 'symbolInventory', 'symbolUnguardable', 'unsupported', 'timerErrors', 'unhandledErrors',
    'pendingTimers', 'pendingTimerDetails', 'completion', 'virtualTime', 'logs', 'replayEligible', 'contract'], 'boundary')
  assert.equal(boundary.contract, 'quod-managed-v0-investigation', 'obsolete worker contract')
  // This host-only research receipt never changes production admission flags.
  assert.equal(boundary.replayEligible, false, 'unexpected runtime admission policy')
  stringArray(boundary.inventory, 'inventory'); stringArray(boundary.guarded, 'guarded')
  for (const key of ['unguardable', 'symbolUnguardable', 'unsupported', 'timerErrors', 'unhandledErrors']) stringArray(boundary[key], key, true)
  assert(Array.isArray(boundary.symbolInventory), 'symbol inventory required')
  for (const row of boundary.symbolInventory) {
    exactKeys(row, ['owner', 'ownerIndex', 'name', 'descriptor', 'valueType', 'primitiveValue', 'enumerable', 'configurable', 'writable', 'policy'], 'symbol inventory row')
    assert(typeof row.owner === 'string' && integer(row.ownerIndex) && typeof row.name === 'string', 'invalid symbol owner/name')
    assert(['data', 'accessor'].includes(row.descriptor) && typeof row.valueType === 'string' && typeof row.primitiveValue === 'string', 'invalid symbol descriptor')
    for (const key of ['enumerable', 'configurable', 'writable']) assert(typeof row[key] === 'boolean', 'invalid symbol descriptor flag')
    assert(['fixed primitive data', 'guarded'].includes(row.policy), 'unguarded symbol prevents reuse')
  }
  assert.equal(boundary.pendingTimers, 0, 'pending timers prevent reuse')
  assert(Array.isArray(boundary.pendingTimerDetails) && boundary.pendingTimerDetails.length === 0, 'pending timer details prevent reuse')
  assert(typeof boundary.virtualTime === 'number' && Number.isFinite(boundary.virtualTime) && boundary.virtualTime >= 0,
    'invalid virtual clock')
  const completion = boundary.completion
  exactKeys(completion, ['sealed', 'nativeTurnFences', 'timerCallbacks', 'pendingTimerCallbacks', 'pendingMicrotasks',
    'pendingEventCallbacks', 'intrinsicLockdown', 'protectedIntrinsicObjects', 'reason'], 'completion')
  assert.equal(completion.sealed, true, 'unsealed completion prevents reuse')
  assert.equal(completion.intrinsicLockdown, true, 'intrinsic lockdown required')
  assert(integer(completion.protectedIntrinsicObjects) && completion.protectedIntrinsicObjects > 0, 'empty intrinsic lockdown')
  assert(integer(completion.nativeTurnFences) && completion.nativeTurnFences >= 2 && completion.nativeTurnFences <= 512,
    'invalid completion fences')
  assert(integer(completion.timerCallbacks), 'invalid timer callback count')
  for (const key of ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks']) {
    assert.equal(completion[key], 0, 'pending async effects prevent reuse: ' + key)
  }
  assert.equal(completion.reason, '', 'incomplete completion reason')
  assert(Array.isArray(boundary.logs), 'primitive console effects required')
  const methods = ['log', 'info', 'warn', 'error', 'debug', 'trace', 'dir', 'assert', 'time', 'timeEnd', 'timeLog']
  for (const log of boundary.logs) {
    assert(Array.isArray(log) && log.length === 2 && methods.includes(log[0]), 'invalid console effect')
    stringArray(log[1], 'console arguments')
  }
  return true
}

function bytesOf(value) {
  assert(typeof value === 'string' || Buffer.isBuffer(value) || value instanceof Uint8Array, 'exact response bytes required')
  const bytes = Buffer.from(value)
  assert(bytes.length > 0 && bytes.length <= 16 * 1024 * 1024, 'invalid response byte size')
  assert(Buffer.from(bytes.toString('utf8'), 'utf8').equals(bytes), 'invalid response UTF-8')
  return bytes
}
function parseResponse(bytes) {
  const text = bytes.toString('utf8'), data = JSON.parse(text)
  // The worker emits compact JSON from primitive own data. Requiring its exact
  // round trip rejects duplicate keys and ambiguous/coerced JSON spellings.
  assert.equal(JSON.stringify(data), text, 'response is not exact compact own-data JSON')
  return data
}
const resultDigest = (key, bytes) => hash(RECEIPT_SCHEMA + '\n' + key + '\n' + bytes.toString('base64'))

/** Writes a new receipt path only. Returns a byte digest for trusted job output. */
export function recordReceipt({file, execution, responseBytes, expected}) {
  assert(typeof file === 'string' && file.length > 0, 'explicit receipt file path required')
  checkedExecution(execution); expectation(expected)
  const bytes = bytesOf(responseBytes), data = parseResponse(bytes)
  validatePassingResponse(data, expected)
  const receipt = {schema: RECEIPT_SCHEMA, researchOnly: true, execution,
    expected: {...expected}, response: {encoding: 'base64', bytes: bytes.toString('base64'), sha256: hash(bytes)},
    resultDigest: resultDigest(execution.key, bytes)}
  const encoded = Buffer.from(canonicalReceiptJSON(receipt), 'utf8')
  fs.writeFileSync(file, encoded, {flag: 'wx', mode: 0o600})
  return {key: execution.key, receiptSha256: hash(encoded), responseSha256: hash(bytes), bytes: encoded.length}
}

/** A digest from the same mutable receipt is never accepted as trusted output. */
export function lookupReceipt({file, execution, trustedReceiptSha256, expected}) {
  try {
    assert(typeof file === 'string' && file.length > 0, 'explicit receipt file path required')
    assert(digest(trustedReceiptSha256), 'trusted seed-job receipt byte SHA required')
    checkedExecution(execution); expectation(expected)
    const handle = fs.openSync(file, 'r')
    let encoded
    try {
      const stat = fs.fstatSync(handle)
      assert(stat.isFile() && stat.size > 0 && stat.size <= 24 * 1024 * 1024, 'invalid receipt file/byte size')
      encoded = fs.readFileSync(handle)
    } finally { fs.closeSync(handle) }
    assert(encoded.length > 0 && encoded.length <= 24 * 1024 * 1024, 'invalid receipt byte size')
    assert.equal(hash(encoded), trustedReceiptSha256, 'trusted receipt byte SHA mismatch')
    const receipt = JSON.parse(encoded.toString('utf8'))
    assert.equal(canonicalReceiptJSON(receipt), encoded.toString('utf8'), 'non-canonical or duplicate receipt JSON')
    exactKeys(receipt, ['schema', 'researchOnly', 'execution', 'expected', 'response', 'resultDigest'], 'receipt')
    assert.equal(receipt.schema, RECEIPT_SCHEMA, 'obsolete receipt schema')
    assert.equal(receipt.researchOnly, true, 'invalid research policy')
    checkedExecution(receipt.execution)
    assert.equal(receipt.execution.key, execution.key, 'execution key miss')
    assert.equal(canonicalReceiptJSON(receipt.execution.identity), canonicalReceiptJSON(execution.identity), 'execution identity miss')
    expectation(receipt.expected)
    assert.equal(canonicalReceiptJSON(receipt.expected), canonicalReceiptJSON(expected), 'workload expectation miss')
    exactKeys(receipt.response, ['encoding', 'bytes', 'sha256'], 'stored response')
    assert.equal(receipt.response.encoding, 'base64', 'invalid response encoding')
    assert(typeof receipt.response.bytes === 'string' && digest(receipt.response.sha256), 'invalid stored response')
    const responseBytes = bytesOf(Buffer.from(receipt.response.bytes, 'base64'))
    assert.equal(responseBytes.toString('base64'), receipt.response.bytes, 'non-canonical response base64')
    assert.equal(hash(responseBytes), receipt.response.sha256, 'response byte digest mismatch')
    assert.equal(resultDigest(execution.key, responseBytes), receipt.resultDigest, 'result domain digest mismatch')
    const data = parseResponse(responseBytes)
    validatePassingResponse(data, expected)
    return {hit: true, key: execution.key, data, responseBytes,
      receiptSha256: trustedReceiptSha256, responseSha256: receipt.response.sha256}
  } catch (error) {
    return {hit: false, reason: error?.message || 'receipt validation failed'}
  }
}
