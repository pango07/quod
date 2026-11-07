// Unused research-only matrix ledger. The v1 ledger and production dispatcher
// are unchanged. Cache transport must authenticate exact receipt bytes before
// lookup; an embedded checksum or cached conformance is never that authority.
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {canonicalReceiptJSON, MANAGED_BOUNDARY_CONTRACT, MANAGED_INTRINSIC_POLICY} from './receipt.mjs'

export const MATRIX_EXECUTION_SCHEMA = 'quod-managed-matrix-execution-v2-research'
export const MATRIX_CONFORMANCE_SCHEMA = 'quod-managed-matrix-conformance-v2-research'
export const MATRIX_RECEIPT_SCHEMA = 'quod-managed-matrix-receipt-v2-research'
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const digest = value => typeof value === 'string' && /^[0-9a-f]{64}$/.test(value)
const integer = value => Number.isSafeInteger(value) && value >= 0
const json = canonicalReceiptJSON
const copy = value => JSON.parse(json(value))
const domainHash = (domain, value) => hash(domain + '\n' + json(value))
const controls = new WeakMap()
const conformances = new WeakMap()
const stateMap = {passed: 'pass', failed: 'fail', skipped: 'skip', todo: 'todo'}
const passing = state => ['pass', 'skip', 'todo'].includes(state)
const specKey = row => json([row.project, row.file, row.pool])
const stateKeys = rows => rows.map(row => json([row.name, row.state])).sort()

function exact(value, names, label) {
  assert(value && typeof value === 'object' && !Array.isArray(value), label + ' object required')
  assert.deepEqual(Object.keys(value).sort(), [...names].sort(), label + ' fields differ')
}
function strings(value, label, empty = false) {
  assert(Array.isArray(value) && value.every(item => typeof item === 'string'), label + ' primitive strings required')
  if (empty) assert.equal(value.length, 0, label + ' prevents reuse')
}
function errors(value, label, empty = false) {
  assert(Array.isArray(value), label + ' array required')
  if (empty) assert.equal(value.length, 0, label + ' prevents reuse')
  for (const row of value) {
    exact(row, ['name', 'message', 'stack'], label)
    assert(['name', 'message', 'stack'].every(key => typeof row[key] === 'string'), label + ' primitive diagnostics required')
  }
}
function instance(value) {
  exact(value, ['instanceId', 'project', 'file', 'pool'], 'matrix instance')
  assert(digest(value.instanceId) && typeof value.project === 'string' && value.project &&
    typeof value.file === 'string' && value.file && !value.file.startsWith('/') && !value.file.split('/').includes('..') &&
    value.pool === 'forks', 'exact project/file/forks instance required')
}
function hostClass(host) {
  for (const key of ['ImageOS', 'ImageVersion', 'platform', 'arch', 'nodeVersion']) {
    assert(typeof host?.[key] === 'string' && host[key], 'explicit native host ' + key + ' required')
  }
  exact(host.kernel, ['release', 'version'], 'native kernel')
  assert(typeof host.kernel.release === 'string' && host.kernel.release && typeof host.kernel.version === 'string' && host.kernel.version)
  exact(host.libc, ['family', 'runtimeVersion', 'compilerVersion'], 'native libc')
  assert(typeof host.libc.family === 'string' && host.libc.family !== 'unknown', 'known native libc required')
  for (const key of ['runtimeVersion', 'compilerVersion']) assert(host.libc[key] === null || typeof host.libc[key] === 'string')
}
function nativePolicy(policy, host) {
  exact(policy, ['adapterSha256', 'nodeExecutableSha256', 'nodeVersion', 'vitestVersion', 'host'], 'native conformance policy')
  assert(digest(policy.adapterSha256) && digest(policy.nodeExecutableSha256), 'native conformance byte identities required')
  assert.equal(policy.vitestVersion, '5.0.0')
  assert.equal(policy.nodeVersion, host.nodeVersion)
  assert.equal(json(policy.host), json(host), 'native control host differs')
}
function policies(identity) {
  const {runtimePolicy: runtime, supervisorPolicy: supervisor, host} = identity
  hostClass(host); nativePolicy(identity.nativePolicy, host)
  assert.equal(runtime.schema, 'quod-managed-runtime-policy-v1-research')
  assert.equal(runtime.boundaryContract, MANAGED_BOUNDARY_CONTRACT)
  assert.equal(runtime.intrinsicPolicy, MANAGED_INTRINSIC_POLICY)
  for (const name of ['workerdSha256', 'nativeFingerprintSha256', 'denyModuleSha256']) assert(digest(runtime[name]), 'actual native ' + name + ' required')
  assert.equal(runtime.moduleName, 'worker.mjs')
  assert.deepEqual(runtime.services, ['main', 'deny'])
  assert.equal(runtime.globalOutbound, 'deny')
  assert.deepEqual(runtime.bindings, [])
  assert.equal(runtime.compatibilityDate, '2026-07-01')
  assert.deepEqual(runtime.compatibilityFlags, ['nodejs_als'])
  exact(runtime.processEnv, ['PATH', 'TZ', 'LANG'], 'runtime environment')
  assert(typeof runtime.processEnv.PATH === 'string' && runtime.processEnv.PATH)
  assert.equal(runtime.processEnv.TZ, 'UTC'); assert.equal(runtime.processEnv.LANG, 'C.UTF-8')
  assert.deepEqual(runtime.request, {method: 'GET', path: '/run'})
  const control = runtime.supervisor
  for (const name of ['sourceSha256', 'nodeExecutableSha256']) assert(digest(control[name]), 'actual supervisor bytes required')
  assert.equal(control.nodeVersion, host.nodeVersion)
  assert.equal(control.nodeExecutableSha256, identity.nativePolicy.nodeExecutableSha256)
  for (const name of ['watchdogMs', 'startupAttempts', 'healthTimeoutMs', 'startupRetryMs', 'shutdownGraceMs', 'responseBytesMax']) {
    assert(integer(control[name]) && control[name] > 0, 'explicit supervisor bound required: ' + name)
  }
  assert.deepEqual(control.completionBounds, {nativeTurnFencesMax: 512, timerCallbackBudget: 4096,
    timerCallbackBudgetScope: 'callbacks after completion begins; reject when exceeded', quietNativeTurns: 2})
  for (const name of ['receiptValidatorSha256', 'experimentSha256', 'conformanceAdapterSha256']) assert(digest(supervisor[name]), 'matrix supervisor source required: ' + name)
  assert.equal(supervisor.conformanceAdapterSha256, identity.nativePolicy.adapterSha256)
}

function checkedExecution(execution) {
  json(execution)
  exact(execution, ['identity', 'key'], 'matrix execution')
  const value = execution.identity
  exact(value, ['schema', 'instance', 'compile', 'preparation', 'runtimePolicy', 'supervisorPolicy', 'host', 'nativePolicy'], 'matrix execution identity')
  assert.equal(value.schema, MATRIX_EXECUTION_SCHEMA)
  assert.equal(execution.key, domainHash(MATRIX_EXECUTION_SCHEMA, value), 'matrix execution key differs')
  instance(value.instance); policies(value)
  assert.equal(value.compile.schema, 'quod-managed-compile-v1-research')
  assert(digest(value.compile.bundleSha256) && digest(value.compile.compilerSha256))
  assert.equal(value.compile.testEntry, '/project/' + value.instance.file)
  assert.equal(value.compile.policy.dynamicCode, 'private-sticky-denial-plus-runtime-constructor-guards')
  assert.equal(value.compile.policy.lazyGlob, 'actual-real-test-entry-only; all-other-matches-sticky-denied')
  assert.equal(value.compile.policy.keepNames, true, 'observable class/function names must be preserved')
  const prepared = value.preparation, specification = prepared.specification
  assert.equal(prepared.schema, 'quod-managed-matrix-preparation-v1-research')
  assert(digest(prepared.preparerSha256) && digest(prepared.configurationSha256))
  assert.equal(specification.absolute, value.compile.testEntry)
  for (const name of ['instanceId', 'project', 'file', 'pool']) assert.equal(specification[name], value.instance[name])
  const selected = {}
  for (const name of ['file', 'project', 'pool', 'testLines', 'testIds', 'testTagsFilter', 'testNamePattern']) selected[name] = specification[name]
  assert.equal(hash(json(selected)), specification.instanceId, 'selection instance identity differs')
  assert(['testLines', 'testIds', 'testTagsFilter', 'testNamePattern'].every(name => selected[name] === null), 'filtered specifications unsupported')
  const alias = /^zod-4\.[0-6]$/.test(value.instance.project) ? value.instance.project : null
  assert(alias || value.instance.project === 'convex-helpers', 'unreviewed matrix project')
  assert.equal(prepared.zodAlias, alias)
  assert.deepEqual(prepared.providedContext, alias ? {expectedZodAlias: alias} : {})
  assert.equal(value.compile.policy.zodAlias || null, alias)
  assert(Array.isArray(prepared.aliases) && prepared.aliases.length === (alias ? 3 : 2), 'exact finite resolved aliases required')
  const expectedVite = new Set(['^\\/?@vite\\/env', '^\\/?@vite\\/client'])
  let zodRules = 0
  for (const rule of prepared.aliases) {
    exact(rule, ['find', 'replacement'], 'resolved alias')
    exact(rule.find, ['source', 'flags'], 'resolved alias pattern')
    assert.equal(rule.find.flags, '')
    if (rule.find.source === '^zod(?=\\/|$)') {
      zodRules++
      assert(alias && rule.replacement === alias, 'actual Zod alias target differs')
    } else {
      assert(expectedVite.delete(rule.find.source), 'unreviewed/duplicate resolved alias')
      assert(typeof rule.replacement === 'string' && rule.replacement.startsWith('/@fs/project/node_modules/vite/dist/client/'))
    }
  }
  assert.equal(zodRules, alias ? 1 : 0); assert.equal(expectedVite.size, 0)
  return execution
}

export function makeMatrixExecutionIdentity({built, prepared, runtimePolicy, supervisorPolicy, host, nativePolicy: native}) {
  assert(typeof built?.bundle === 'string' && hash(built.bundle) === built.sha256, 'actual compiled bytes required')
  assert.equal(built.identity.bundleSha256, built.sha256)
  assert.equal(hash(json(built.identity)), built.identitySha256)
  assert.equal(hash(json(built.options)), built.identity.configurationSha256)
  assert.equal(hash(json(prepared.config)), prepared.identity.configurationSha256)
  assert.equal(json(built.options.vitest), json(prepared.config))
  assert.equal(json(built.preparation), json(prepared.identity))
  assert.equal(built.options.file, built.identity.testEntry)
  assert.equal(built.options.zodAlias || null, prepared.identity.zodAlias)
  assert.equal(json(built.options.providedContext), json(prepared.identity.providedContext))
  assert.equal(prepared.config.name, prepared.identity.specification.project)
  assert.equal(prepared.config.isolate, true)
  const spec = prepared.identity.specification
  const identity = copy({schema: MATRIX_EXECUTION_SCHEMA,
    instance: {instanceId: spec.instanceId, project: spec.project, file: spec.file, pool: spec.pool},
    compile: built.identity, preparation: prepared.identity, runtimePolicy, supervisorPolicy, host, nativePolicy: native})
  return checkedExecution({identity, key: domainHash(MATRIX_EXECUTION_SCHEMA, identity)})
}

function bytesOf(value, max) {
  assert(typeof value === 'string' || Buffer.isBuffer(value) || value instanceof Uint8Array, 'exact JSON bytes required')
  const bytes = Buffer.from(value)
  assert(bytes.length > 0 && bytes.length <= max, 'JSON byte bounds exceeded')
  assert(Buffer.from(bytes.toString('utf8'), 'utf8').equals(bytes), 'invalid JSON UTF-8')
  return bytes
}
function parseBytes(value, {compact = false, max = 24 * 1024 * 1024} = {}) {
  const bytes = bytesOf(value, max), text = bytes.toString('utf8'), data = JSON.parse(text)
  // Trusted native producers emit compact JSON or two-space JSON plus newline. Exact round
  // trips reject duplicate keys instead of silently accepting the last value.
  if (compact) assert.equal(JSON.stringify(data), text, 'non-compact or ambiguous response JSON')
  else assert(text === json(data) || text === JSON.stringify(data) ||
    text === JSON.stringify(data, null, 2) || text === JSON.stringify(data, null, 2) + '\n',
    'non-canonical or ambiguous native/receipt JSON')
  json(data)
  return {bytes, data}
}

/** Only the trusted producer passes bytes from its actual fresh native run.
 * Validate the whole native control once; individual conformance tokens below
 * retain compact rows, not another copy of the120-instance report. */
export function validateMatrixNativeControl({selectionBytes, reportBytes, policy}) {
  json(policy); hostClass(policy.host); nativePolicy(policy, policy.host)
  const selected = parseBytes(selectionBytes, {max: 32 * 1024 * 1024})
  const report = parseBytes(reportBytes, {max: 32 * 1024 * 1024})
  const control = selected.data
  assert.equal(control.schema, 'quod-managed-matrix-native-v1')
  assert(control.completed === true && control.matched === true && control.runStarted === true && control.exitCode === 0, 'native control incomplete/failed')
  assert.deepEqual(control.unhandledErrors, []); assert.equal(control.failedModules, 0)
  assert.equal(control.reportSha256, hash(report.bytes), 'actual native JSON report SHA differs')
  assert.equal(report.data.success, true, 'native report failed')
  assert(Array.isArray(control.selected) && control.selected.length > 0 && control.selected.length <= 128)
  assert(Array.isArray(control.executed) && control.executed.length === control.selected.length)
  assert(Array.isArray(report.data.testResults) && report.data.testResults.length === control.selected.length)
  const specifications = new Map(control.selected.map(row => [specKey(row), row]))
  assert.equal(specifications.size, control.selected.length, 'native selected duplicate instances')
  for (const specification of control.selected) {
    const selectedIdentity = {}
    for (const name of ['file', 'project', 'pool', 'testLines', 'testIds', 'testTagsFilter', 'testNamePattern']) selectedIdentity[name] = specification[name]
    assert.equal(hash(json(selectedIdentity)), specification.instanceId, 'native selected instance identity differs')
    assert.equal(specification.pool, 'forks')
    assert(['testLines', 'testIds', 'testTagsFilter', 'testNamePattern'].every(name => specification[name] === null), 'native control must retain the complete original specifications')
  }
  const modules = new Map(), apiRows = []
  for (const module of control.executed) {
    const spec = specifications.get(specKey(module))
    assert(spec && spec.instanceId === module.instanceId && spec.absolute === module.absolute, 'native execution selection differs')
    assert(!modules.has(module.instanceId), 'native executed duplicate instance')
    assert.equal(module.ok, true); assert(passing(module.state), 'native module failed/pending')
    errors(module.errors, 'native file errors', true)
    assert(Array.isArray(module.suites))
    for (const suite of module.suites) {
      assert(['passed', 'skipped'].includes(suite.state), 'native suite failed/pending')
      errors(suite.errors, 'native suite errors', true)
    }
    assert(Array.isArray(module.rows) && module.rows.length > 0, 'empty native modules unsupported by v2')
    for (const row of module.rows) {
      assert(typeof row.name === 'string' && row.name && passing(row.state), 'native test failed/pending')
      errors(row.errors, 'native test diagnostics', row.state !== 'pass')
      apiRows.push(json([module.absolute, row.name, row.state]))
    }
    modules.set(module.instanceId, copy(module))
  }
  const reporterRows = []
  for (const file of report.data.testResults) {
    assert(typeof file.name === 'string' && Array.isArray(file.assertionResults))
    for (const row of file.assertionResults) {
      assert(typeof row.fullName === 'string' && passing(stateMap[row.status]), 'native JSON test failed/pending')
      reporterRows.push(json([file.name, row.fullName, stateMap[row.status]]))
    }
  }
  assert.deepEqual(apiRows.sort(), reporterRows.sort(), 'actual native APIs differ from original JSON')
  const token = Object.freeze({reportSha256: hash(report.bytes), selectionSha256: hash(selected.bytes)})
  controls.set(token, {modules, policy: copy(policy)})
  return token
}

function checkedConformance(conformance, execution) {
  json(conformance)
  exact(conformance, ['schema', 'executionKey', 'instance', 'nativeReportSha256', 'nativeSelectionSha256', 'nativePolicySha256', 'file', 'rows'], 'matrix conformance')
  assert.equal(conformance.schema, MATRIX_CONFORMANCE_SCHEMA)
  assert.equal(conformance.executionKey, execution.key)
  assert.equal(json(conformance.instance), json(execution.identity.instance))
  for (const name of ['nativeReportSha256', 'nativeSelectionSha256', 'nativePolicySha256']) assert(digest(conformance[name]), 'native conformance SHA required')
  assert.equal(conformance.nativePolicySha256, hash(json(execution.identity.nativePolicy)))
  exact(conformance.file, ['name', 'state'], 'conformance file')
  assert(typeof conformance.file.name === 'string' && conformance.file.name && passing(conformance.file.state))
  const prefix = 'packages/convex-helpers/'
  assert(execution.identity.instance.file.startsWith(prefix), 'finite matrix file root required')
  assert.equal(conformance.file.name, execution.identity.instance.file.slice(prefix.length), 'actual native relative file differs')
  assert(Array.isArray(conformance.rows) && conformance.rows.length > 0)
  for (const row of conformance.rows) {
    exact(row, ['name', 'state'], 'conformance test')
    assert(typeof row.name === 'string' && row.name && passing(row.state), 'native conformance non-passing test')
  }
  assert.deepEqual(conformance.rows.map(row => json([row.name, row.state])), stateKeys(conformance.rows), 'conformance rows must be a canonical multiset')
  return conformance
}

export function makeMatrixConformance({execution, control}) {
  checkedExecution(execution)
  const verified = controls.get(control)
  assert(verified, 'validated fresh native control token required')
  assert.equal(json(verified.policy), json(execution.identity.nativePolicy), 'native conformance policy differs')
  const module = verified.modules.get(execution.identity.instance.instanceId)
  assert(module && specKey(module) === specKey(execution.identity.instance), 'native conformance instance differs')
  assert.equal(json(module.providedContext), json(execution.identity.preparation.providedContext))
  const rows = stateKeys(module.rows).map(key => {const [name, state] = JSON.parse(key); return {name, state}})
  const conformance = checkedConformance(copy({schema: MATRIX_CONFORMANCE_SCHEMA, executionKey: execution.key,
    instance: execution.identity.instance, nativeReportSha256: control.reportSha256,
    nativeSelectionSha256: control.selectionSha256, nativePolicySha256: hash(json(verified.policy)),
    file: {name: module.relativeModuleId, state: module.state}, rows}), execution)
  conformances.set(conformance, json(conformance))
  return conformance
}

function boundary(value) {
  exact(value, ['inventory', 'guarded', 'unguardable', 'symbolInventory', 'symbolUnguardable', 'unsupported',
    'timerErrors', 'unhandledErrors', 'pendingTimers', 'pendingTimerDetails', 'completion', 'virtualTime', 'logs',
    'replayEligible', 'contract', 'intrinsicPolicy'], 'matrix worker boundary')
  assert.equal(value.contract, MANAGED_BOUNDARY_CONTRACT)
  assert.equal(value.intrinsicPolicy, MANAGED_INTRINSIC_POLICY)
  assert.equal(value.replayEligible, false, 'research runtime admission must remain disabled')
  strings(value.inventory, 'ambient inventory'); strings(value.guarded, 'guarded globals')
  for (const name of ['unguardable', 'symbolUnguardable', 'unsupported', 'timerErrors', 'unhandledErrors']) strings(value[name], name, true)
  assert(Array.isArray(value.symbolInventory))
  for (const row of value.symbolInventory) {
    exact(row, ['owner', 'ownerIndex', 'name', 'descriptor', 'valueType', 'primitiveValue', 'enumerable', 'configurable', 'writable', 'policy'], 'symbol inventory')
    assert(typeof row.owner === 'string' && integer(row.ownerIndex) && typeof row.name === 'string')
    assert(['data', 'accessor'].includes(row.descriptor) && typeof row.valueType === 'string' && typeof row.primitiveValue === 'string')
    assert(['enumerable', 'configurable', 'writable'].every(name => typeof row[name] === 'boolean'))
    assert(['fixed primitive data', 'guarded'].includes(row.policy), 'unreviewed native symbol')
  }
  assert.equal(value.pendingTimers, 0); assert.deepEqual(value.pendingTimerDetails, [])
  assert(typeof value.virtualTime === 'number' && Number.isFinite(value.virtualTime) && value.virtualTime >= 0)
  const completion = value.completion
  exact(completion, ['sealed', 'nativeTurnFences', 'timerCallbacks', 'pendingTimerCallbacks', 'pendingMicrotasks',
    'pendingEventCallbacks', 'intrinsicLockdown', 'protectedIntrinsicObjects', 'reason'], 'matrix completion')
  assert.equal(completion.sealed, true); assert.equal(completion.intrinsicLockdown, true)
  assert(integer(completion.protectedIntrinsicObjects) && completion.protectedIntrinsicObjects > 0)
  assert(integer(completion.nativeTurnFences) && completion.nativeTurnFences >= 2 && completion.nativeTurnFences <= 512)
  assert(integer(completion.timerCallbacks))
  for (const name of ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks']) assert.equal(completion[name], 0)
  assert.equal(completion.reason, '')
  assert(Array.isArray(value.logs))
  for (const log of value.logs) {
    assert(Array.isArray(log) && log.length === 2 && ['log', 'info', 'warn', 'error', 'debug', 'trace', 'dir', 'assert', 'time', 'timeEnd', 'timeLog'].includes(log[0]))
    strings(log[1], 'primitive console arguments')
  }
}

/** Duplicate names, all-skipped files and passed retry diagnostics are real
 * Vitest outcomes. Preserve them; actual fail/pending/error states never reuse. */
export function validateReusableMatrixResponse({data, execution, conformance}) {
  json(data); checkedExecution(execution); checkedConformance(conformance, execution)
  exact(data, ['ok', 'rows', 'files', 'boundary'], 'matrix worker response')
  assert.equal(data.ok, true, 'failed managed response cannot be reused')
  assert(Array.isArray(data.rows) && data.rows.length === conformance.rows.length)
  for (const row of data.rows) {
    exact(row, ['name', 'state', 'errors'], 'matrix test row')
    assert(typeof row.name === 'string' && row.name && passing(row.state), 'failed/pending managed test prevents reuse')
    errors(row.errors, 'managed test diagnostics', row.state !== 'pass')
  }
  assert.deepEqual(stateKeys(data.rows), stateKeys(conformance.rows), 'complete native-conformed name/state multiset differs')
  assert(Array.isArray(data.files) && data.files.length === 1)
  const file = data.files[0]
  exact(file, ['name', 'state', 'errors'], 'matrix file row')
  assert.equal(file.name, conformance.file.name)
  assert.equal(file.state, conformance.file.state)
  assert(passing(file.state), 'failed/pending file prevents reuse')
  errors(file.errors, 'managed file errors', true)
  boundary(data.boundary)
  return true
}

const resultDigest = (key, conformanceSha256, bytes) => hash(MATRIX_RECEIPT_SCHEMA + '\n' + key + '\n' + conformanceSha256 + '\n' + bytes.toString('base64'))

/** The producer must first validate its fresh complete native control. A plain
 * cached/forged conformance object cannot be passed back into this mint path. */
export function createMatrixReceipt({execution, conformance, responseBytes}) {
  checkedExecution(execution)
  assert.equal(conformances.get(conformance), json(conformance), 'fresh validated native conformance token required')
  const response = parseBytes(responseBytes, {compact: true, max: 16 * 1024 * 1024})
  validateReusableMatrixResponse({data: response.data, execution, conformance})
  const conformanceSha256 = domainHash(MATRIX_CONFORMANCE_SCHEMA, conformance)
  const receipt = {schema: MATRIX_RECEIPT_SCHEMA, researchOnly: true, execution, conformance,
    conformanceSha256, response: {encoding: 'base64', bytes: response.bytes.toString('base64'), sha256: hash(response.bytes)},
    resultDigest: resultDigest(execution.key, conformanceSha256, response.bytes)}
  const receiptBytes = Buffer.from(json(receipt))
  assert(receiptBytes.length <= 24 * 1024 * 1024, 'matrix receipt byte bound exceeded')
  return {key: execution.key, receiptBytes, receiptSha256: hash(receiptBytes),
    responseSha256: receipt.response.sha256, conformanceSha256}
}

/** trustedReceiptSha256 is supplied by the authenticated pack reader, whose
 * whole-byte SHA came from the actual SDK-returned key. It never comes from a
 * manifest field in the same mutable candidate or from the candidate itself. */
export function lookupMatrixReceipt({receiptBytes, trustedReceiptSha256, execution}) {
  try {
    assert(digest(trustedReceiptSha256), 'external authenticated receipt SHA required')
    checkedExecution(execution)
    const encoded = bytesOf(receiptBytes, 24 * 1024 * 1024)
    assert.equal(hash(encoded), trustedReceiptSha256, 'authenticated receipt bytes differ')
    const receipt = parseBytes(encoded).data
    assert.equal(json(receipt), encoded.toString('utf8'), 'canonical matrix receipt required')
    exact(receipt, ['schema', 'researchOnly', 'execution', 'conformance', 'conformanceSha256', 'response', 'resultDigest'], 'matrix receipt')
    assert.equal(receipt.schema, MATRIX_RECEIPT_SCHEMA); assert.equal(receipt.researchOnly, true)
    checkedExecution(receipt.execution)
    assert.equal(receipt.execution.key, execution.key, 'matrix execution key miss')
    assert.equal(json(receipt.execution.identity), json(execution.identity), 'complete current matrix identity miss')
    checkedConformance(receipt.conformance, execution)
    assert.equal(receipt.conformanceSha256, domainHash(MATRIX_CONFORMANCE_SCHEMA, receipt.conformance))
    exact(receipt.response, ['encoding', 'bytes', 'sha256'], 'matrix stored response')
    assert.equal(receipt.response.encoding, 'base64')
    assert(typeof receipt.response.bytes === 'string' && digest(receipt.response.sha256))
    const response = parseBytes(Buffer.from(receipt.response.bytes, 'base64'), {compact: true, max: 16 * 1024 * 1024})
    assert.equal(response.bytes.toString('base64'), receipt.response.bytes, 'non-canonical response base64')
    assert.equal(hash(response.bytes), receipt.response.sha256)
    assert.equal(resultDigest(execution.key, receipt.conformanceSha256, response.bytes), receipt.resultDigest)
    validateReusableMatrixResponse({data: response.data, execution, conformance: receipt.conformance})
    return {hit: true, key: execution.key, data: response.data, responseBytes: response.bytes,
      receiptSha256: trustedReceiptSha256, responseSha256: receipt.response.sha256, conformance: receipt.conformance}
  } catch (error) { return {hit: false, reason: error?.message || 'matrix receipt validation failed'} }
}
