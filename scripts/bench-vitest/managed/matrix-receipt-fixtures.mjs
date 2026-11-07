// Remote data-only ledger fixtures against the actual completed matrix run.
// No receipt is published; synthetic digest arguments below exercise the ledger
// trust boundary. Actual SDK pack authentication is a separate Action gate.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {fileURLToPath} from 'node:url'
import {canonicalReceiptJSON as canonical} from './receipt.mjs'
import {MATRIX_RECEIPT_SCHEMA, MATRIX_CONFORMANCE_SCHEMA, makeMatrixExecutionIdentity,
  validateMatrixNativeControl, makeMatrixConformance, validateReusableMatrixResponse,
  createMatrixReceipt, lookupMatrixReceipt} from './matrix-receipt.mjs'

const [matrixArg, outArg] = process.argv.slice(2)
assert(matrixArg && outArg, 'usage: matrix-receipt-fixtures.mjs ACTUAL_PASSED_MATRIX EMPTY_OUTPUT')
const matrix = path.resolve(matrixArg), out = path.resolve(outArg)
fs.mkdirSync(out, {recursive: true}); assert.equal(fs.readdirSync(out).length, 0)
const here = path.dirname(fileURLToPath(import.meta.url))
const hash = value => crypto.createHash('sha256').update(value).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const copy = value => JSON.parse(canonical(value))
const bytes = value => Buffer.from(JSON.stringify(value))
const pretty = value => Buffer.from(JSON.stringify(value, null, 2) + '\n')
const result = {schema: 'quod-managed-matrix-receipt-fixtures-v2-research', passed: false,
  productionEnabled: false, cachePublished: false, actualSdkAuthenticationClaimed: false, checks: [], receipts: []}
const check = (name, passed) => {result.checks.push({name, passed: Boolean(passed)}); assert(passed, name)}
const rejects = (name, operation) => {
  let rejection
  try { operation() } catch (error) { rejection = error }
  check(name, rejection?.name === 'AssertionError')
}
try {
  const actual = read(path.join(matrix, 'result.json'))
  assert.equal(actual.passed, true, 'a failed whole matrix must never mint/publish receipts')
  const selectionBytes = fs.readFileSync(path.join(matrix, 'native-selection.json'))
  const reportBytes = fs.readFileSync(path.join(matrix, 'native-report.json'))
  const selection = JSON.parse(selectionBytes), report = JSON.parse(reportBytes)
  const preflight = read(path.join(matrix, 'preflight-one/response.json'))
  const preflightBuild = read(path.join(matrix, 'preflight-build.json'))
  const nativeFingerprintSha256 = hash(canonical({preflightIdentity: preflightBuild.identitySha256,
    inventory: preflight.boundary.inventory, symbols: preflight.boundary.symbolInventory}))
  const adapterSha256 = hash(fs.readFileSync(path.join(here, 'matrix-probe.mjs')))
  const nativePolicy = {adapterSha256,
    nodeExecutableSha256: actual.runtimePolicy.supervisor.nodeExecutableSha256,
    nodeVersion: actual.node, vitestVersion: '5.0.0', host: actual.host}
  const control = validateMatrixNativeControl({selectionBytes, reportBytes, policy: nativePolicy})
  check('the actual full native control is validated once before minting', Boolean(control.reportSha256 && control.selectionSha256))
  const records = []
  fs.mkdirSync(path.join(out, 'receipts'))
  for (const row of actual.instances.filter(row => row.decision === 'managed')) {
    const directory = path.join(matrix, row.evidenceDirectory)
    const built = read(path.join(directory, 'test-build.json'))
    built.bundle = fs.readFileSync(path.join(directory, 'managed/worker.mjs'), 'utf8')
    const prepared = {config: built.options.vitest, identity: read(path.join(directory, 'preparation.json'))}
    const invocation = read(path.join(directory, 'managed/invocation.json'))
    const execution = makeMatrixExecutionIdentity({built, prepared,
      runtimePolicy: {...invocation.runtimePolicy, nativeFingerprintSha256}, nativePolicy,
      supervisorPolicy: {receiptValidatorSha256: hash(fs.readFileSync(path.join(here, 'matrix-receipt.mjs'))),
        experimentSha256: hash(fs.readFileSync(fileURLToPath(import.meta.url))), conformanceAdapterSha256: adapterSha256},
      host: actual.host})
    const conformance = makeMatrixConformance({execution, control})
    const responseBytes = fs.readFileSync(path.join(directory, 'managed/response.json'))
    const receipt = createMatrixReceipt({execution, conformance, responseBytes})
    const lookup = lookupMatrixReceipt({...receipt, execution, trustedReceiptSha256: receipt.receiptSha256})
    check('actual conformed per-instance round trip: ' + row.instanceId,
      lookup.hit && Buffer.from(lookup.responseBytes).equals(responseBytes))
    const file = path.join(out, 'receipts', row.instanceId + '.json')
    fs.writeFileSync(file, receipt.receiptBytes, {flag: 'wx'})
    result.receipts.push({instanceId: row.instanceId, project: row.project, file: row.file, key: execution.key,
      receiptSha256: receipt.receiptSha256, responseSha256: receipt.responseSha256, bytes: receipt.receiptBytes.length,
      count: lookup.data.rows.length, fileState: lookup.data.files[0].state})
    records.push({row, execution, conformance, receipt, data: lookup.data, responseBytes})
  }
  assert(records.length > 0)
  result.actualInstances = records.length
  result.actualStates = records.reduce((sum, record) => sum + record.data.rows.length, 0)
  result.totalReceiptBytes = records.reduce((sum, record) => sum + record.receipt.receiptBytes.length, 0)
  // The authenticated transport base64-encodes each receipt. Include an upper
  // bound for its fixed primitive metadata instead of comparing only raw bytes.
  result.transportBytesUpperBound = 512 + records.reduce((sum, record) =>
    sum + Math.ceil(record.receipt.receiptBytes.length / 3) * 4 + 512, 0)
  check('all actual managed instances were retained exactly once', records.length === actual.coverage.managedPassing)
  check('every actual receipt fits the2MiB authenticated entry bound', records.every(record => record.receipt.receiptBytes.length <= 2 * 1024 * 1024))
  check('base64 receipts and metadata fit the one-pack32MiB transport budget', result.transportBytesUpperBound <= 32 * 1024 * 1024)
  const base = records.find(record => record.data.rows.some(row => row.state === 'pass'))
  const skipped = records.find(record => record.data.files[0].state === 'skip' && record.data.rows.every(row => row.state === 'skip'))
  const duplicate = records.find(record => new Set(record.data.rows.map(row => row.name)).size < record.data.rows.length)
  check('the actual all-skipped original file is reusable without changing its state', skipped && skipped.data.rows.length > 0)
  check('the actual duplicate-named original tests retain their full multiplicity', duplicate && duplicate.data.rows.length === duplicate.conformance.rows.length)
  const hit = (receipt, execution = base.execution, trusted = receipt.receiptSha256) => lookupMatrixReceipt({
    receiptBytes: receipt.receiptBytes, trustedReceiptSha256: trusted, execution})
  check('missing external authority is a miss', !hit(base.receipt, base.execution, '').hit)
  check('wrong external byte digest is a miss', !hit(base.receipt, base.execution, '0'.repeat(64)).hit)
  check('a digest embedded inside the candidate is not an authority argument', !lookupMatrixReceipt({
    receiptBytes: base.receipt.receiptBytes, execution: base.execution}).hit)
  const other = records.find(record => record.row.project !== base.row.project)
  check('a valid receipt cannot cross actual project identity', other && !hit(base.receipt, other.execution).hit)

  const forge = mutate => {
    const encoded = JSON.parse(base.receipt.receiptBytes)
    const data = JSON.parse(Buffer.from(encoded.response.bytes, 'base64'))
    mutate(data, encoded)
    const response = bytes(data)
    encoded.response = {encoding: 'base64', bytes: response.toString('base64'), sha256: hash(response)}
    encoded.conformanceSha256 = hash(MATRIX_CONFORMANCE_SCHEMA + '\n' + canonical(encoded.conformance))
    encoded.resultDigest = hash(MATRIX_RECEIPT_SCHEMA + '\n' + encoded.execution.key + '\n' + encoded.conformanceSha256 + '\n' + response.toString('base64'))
    const receiptBytes = Buffer.from(canonical(encoded))
    return {receiptBytes, receiptSha256: hash(receiptBytes)}
  }
  const coherent = forge(data => {data.rows[0].name += ' forged'})
  check('coherently rehashed forged candidate misses the unchanged external authority', !hit(coherent, base.execution, base.receipt.receiptSha256).hit)
  check('even an authenticated malformed result must match native conformance', !hit(coherent).hit)
  for (const [name, mutate] of [
    ['actual failed state cannot reuse', data => {data.rows[0].state = 'fail'}],
    ['incomplete test rows cannot reuse', data => {data.rows.pop()}],
    ['unsupported caught capability cannot reuse', data => {data.boundary.unsupported.push('global fetch')}],
    ['unsealed completion cannot reuse', data => {data.boundary.completion.sealed = false}],
    ['nonempty completion reason cannot reuse', data => {data.boundary.completion.reason = 'incomplete'}],
    ['pending timers cannot reuse', data => {data.boundary.pendingTimers = 1}],
    ['pending timer callbacks cannot reuse', data => {data.boundary.completion.pendingTimerCallbacks = 1}],
    ['pending microtasks cannot reuse', data => {data.boundary.completion.pendingMicrotasks = 1}],
    ['pending event callbacks cannot reuse', data => {data.boundary.completion.pendingEventCallbacks = 1}],
    ['unhandled asynchronous failure cannot reuse', data => {data.boundary.unhandledErrors.push('failure')}],
    ['file-level errors cannot reuse', data => {data.files[0].errors.push({name: 'Error', message: 'failed', stack: ''})}],
    ['unknown diagnostic fields cannot reuse', data => {data.rows[0].errors.push({name: 'Error', message: 'unknown', stack: '', code: 'UNKNOWN'})}],
    ['old runtime policy cannot reuse', data => {data.boundary.contract = 'obsolete'}],
    ['old intrinsic policy cannot reuse', data => {data.boundary.intrinsicPolicy = 'obsolete'}],
  ]) check(name, !hit(forge(mutate)).hit)
  const policyChanged = copy(base.execution)
  policyChanged.identity.runtimePolicy.globalOutbound = 'allow'
  policyChanged.key = hash(policyChanged.identity.schema + '\n' + canonical(policyChanged.identity))
  check('coherently changed native capability policy cannot reuse', !hit(base.receipt, policyChanged).hit)
  const retry = copy(base.data)
  const passedRow = retry.rows.find(row => row.state === 'pass')
  passedRow.errors = [{name: 'AssertionError', message: 'earlier passing retry diagnostic', stack: 'fixture stack'}]
  const retried = createMatrixReceipt({execution: base.execution, conformance: base.conformance, responseBytes: bytes(retry)})
  check('passed retry diagnostics are preserved as primitive data', hit(retried).hit &&
    canonical(hit(retried).data.rows.find(row => row.name === passedRow.name).errors) === canonical(passedRow.errors))
  const skipError = copy(skipped.data)
  skipError.rows[0].errors = [{name: 'Error', message: 'unexpected skipped failure', stack: ''}]
  rejects('skipped failure diagnostics cannot be normalized into a pass', () => createMatrixReceipt({
    execution: skipped.execution, conformance: skipped.conformance, responseBytes: bytes(skipError)}))
  rejects('plain cached conformance cannot mint another receipt', () => createMatrixReceipt({
    execution: base.execution, conformance: copy(base.conformance), responseBytes: base.responseBytes}))
  rejects('a plain object cannot stand in for validated fresh native control', () => makeMatrixConformance({execution: base.execution, control: {}}))
  const changedConformance = makeMatrixConformance({execution: base.execution, control})
  changedConformance.rows[0].name += ' forged'
  rejects('a changed native conformance token cannot mint', () => createMatrixReceipt({
    execution: base.execution, conformance: changedConformance, responseBytes: base.responseBytes}))
  let getterCalls = 0
  const accessor = copy(base.data)
  Object.defineProperty(accessor.rows[0], 'name', {enumerable: true, get() {getterCalls++; return 'forged'}})
  rejects('response accessors are rejected before being invoked', () => validateReusableMatrixResponse({
    data: accessor, execution: base.execution, conformance: base.conformance}))
  check('the forged response getter was never invoked', getterCalls === 0)
  let serializerCalls = 0
  const serializer = copy(base.data)
  Object.defineProperty(serializer, 'toJSON', {value() {serializerCalls++; return {ok: true}}})
  rejects('response toJSON hooks cannot forge validation', () => validateReusableMatrixResponse({
    data: serializer, execution: base.execution, conformance: base.conformance}))
  check('the forged serializer was never invoked', serializerCalls === 0)
  for (const [name, mutateControl, mutateReport] of [
    ['an incomplete native producer cannot mint conformance', control => {control.completed = false}, () => {}],
    ['a failed native producer cannot mint conformance', control => {control.failedModules = 1}, () => {}],
    ['a native API/full JSON row mismatch cannot mint conformance', control => {control.executed[0].rows[0].name += ' forged'}, () => {}],
    ['a forged selected native identity cannot mint conformance', control => {control.selected[0].instanceId = '0'.repeat(64)}, () => {}],
    ['a failed original JSON report cannot mint conformance', () => {}, report => {report.success = false}],
  ]) {
    const changedControl = copy(selection), changedReport = copy(report)
    mutateControl(changedControl); mutateReport(changedReport)
    const changedBytes = pretty(changedReport)
    changedControl.reportSha256 = hash(changedBytes)
    rejects(name, () => validateMatrixNativeControl({selectionBytes: pretty(changedControl), reportBytes: changedBytes, policy: nativePolicy}))
  }
  const wrongContext = copy(selection)
  wrongContext.executed.find(module => module.instanceId === base.row.instanceId).providedContext = {forged: true}
  const wrongContextControl = validateMatrixNativeControl({selectionBytes: pretty(wrongContext), reportBytes, policy: nativePolicy})
  rejects('fresh native context must match the exact current matrix project', () => makeMatrixConformance({execution: base.execution, control: wrongContextControl}))
  result.passed = true
} catch (error) {
  result.error = {name: error.name, message: error.message, stack: error.stack}
  process.exitCode = 1
} finally {
  fs.writeFileSync(path.join(out, 'result.json'), JSON.stringify(result, null, 2) + '\n')
  console.log(JSON.stringify({...result, receipts: undefined}))
}
