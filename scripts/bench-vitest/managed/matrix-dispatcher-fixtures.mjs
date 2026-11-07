// Remote execution fixtures. Receipt SHA arguments here are explicit synthetic
// test authority; only matrix-sdk-probe exercises actual cache-service trust.
// Original tests remain intact; two appended controls are removed in finally.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {dispatchManagedMatrix} from './matrix-dispatcher.mjs'

const [projectArg, outArg] = process.argv.slice(2)
assert(projectArg && outArg, 'usage: matrix-dispatcher-fixtures.mjs PROJECT EMPTY_OUTPUT')
const project = fs.realpathSync(path.resolve(projectArg)), out = path.resolve(outArg)
fs.mkdirSync(out, {recursive: true}); assert.equal(fs.readdirSync(out).length, 0)
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n')
const candidate = 'packages/convex-helpers/index.test.ts'
const ordinary = 'packages/convex-helpers/functionReferences.test.ts'
const selected = [candidate, ordinary]
const sourceHashes = ['06e78a6fd8e397384c1c2d8b1e38939755882d8ba0d6017d5946cdb468564b2f',
  '09544a8dd35b40754de45d058ccb5627f86c7248c12ba3ce1a55c6ad3dd65fa6']
const modeName = 'QUOD_MATRIX_DISPATCH_FIXTURE'
const previousMode = Object.getOwnPropertyDescriptor(process.env, modeName)
const originals = selected.map(file => fs.readFileSync(path.join(project, file)))
const result = {schema: 'quod-managed-matrix-dispatcher-fixtures-v1-research', passed: false,
  actualSdkAuthenticationClaimed: false, productionEnabled: false,
  selectedScope: {kind: 'explicit-file-selection', files: selected}, checks: [], cases: [], sources: []}
const check = (name, passed) => {result.checks.push({name, passed: Boolean(passed)}); assert(passed, name)}
const save = () => write(path.join(out, 'result.json'), result)
const candidateControl = `
import {afterAll as quodMatrixAfterAll} from 'vitest';
test('QUOD matrix candidate control', async () => {
  console.log('QUOD_MATRIX_' + 'CANDIDATE_ONCE');
  const mode = process.env.${modeName};
  if (mode === 'assertion-failure') expect(1).toBe(2);
  if (mode === 'unsupported-pass' || mode === 'unsupported-failure') {
    let text = 'blocked';
    try {text = await (await fetch('data:text/plain,quod-matrix-native')).text();} catch {}
    expect(text).toBe(mode === 'unsupported-pass' ? 'quod-matrix-native' : 'deliberate-native-failure');
  }
});
quodMatrixAfterAll(() => {
  if (process.env.${modeName} === 'hook-failure') throw new Error('QUOD_MATRIX_HOOK_FAILURE');
});
`
const ordinaryControl = `
import {expect as quodMatrixExpect} from 'vitest';
test('QUOD matrix ordinary control', () => {
  console.log('QUOD_MATRIX_' + 'ORDINARY_ONCE');
  quodMatrixExpect(process.env.${modeName} === 'ordinary-failure').toBe(false);
});
`
const authority = new Map()
async function run(name, mode, {lookup, captureAuthority = false} = {}) {
  const caseOut = path.join(out, name)
  process.env[modeName] = mode
  let records = 0, publishes = 0, publicationExit
  const dispatch = await dispatchManagedMatrix({project, out: caseOut, suiteFiles: selected,
    env: {NODE_ENV: 'test', [modeName]: mode}, lookup,
    async record(entry) {
      records++
      const receiptBytes = fs.readFileSync(entry.file)
      assert.equal(hash(receiptBytes), entry.receiptSha256)
      if (captureAuthority) authority.set(entry.execution.key, {receiptBytes, trustedReceiptSha256: entry.receiptSha256})
    }, async publish({exitCode}) {publishes++; publicationExit = exitCode; return {saved: false, reason: 'fixture-no-SDK'}}})
  const selection = read(path.join(caseOut, 'native/native-selection.json'))
  const nativeLog = fs.readFileSync(path.join(caseOut, 'native/native.log'), 'utf8')
  // Assertion code frames can repeat the source spelling of a nearby log.
  // Count only the emitted console line, after reporter color removal.
  const markerCount = marker => nativeLog.replace(/\x1b\[[0-9;]*m/g, '').split(/\r?\n/)
    .filter(line => line.trim() === marker).length
  const logical = read(path.join(caseOut, 'logical-results.json'))
  const item = {name, mode, exitCode: dispatch.exitCode, managedExecutions: dispatch.managedExecutions,
    replayedInstances: dispatch.replayedInstances, nativeSpecsExecuted: dispatch.nativeSpecsExecuted,
    logicalSha256: dispatch.logicalSha256, logicalStates: dispatch.logicalStates,
    records, publishes, publicationExit, evidenceDirectory: name}
  result.cases.push(item); save()
  check(name + ': the two-file scope retains one reviewed candidate and one ordinary instance',
    dispatch.selectedScope.kind === 'explicit-file-selection' && dispatch.selectedInstances === 2 &&
    dispatch.candidateInstances === 1 && dispatch.initialOrdinaryInstances === 1 && dispatch.fullJobHit === false)
  check(name + ': all thirteen original and appended states remain present', logical.rows.length === 13 && dispatch.logicalStates === 13)
  check(name + ': publication observes the final whole-command exit exactly once', publishes === 1 && publicationExit === dispatch.exitCode)
  check(name + ': actual native public selection matches the declared scope', selection.matched === true && selection.completed === true &&
    selection.originalSelected.length === 2 && selection.executed.length === dispatch.nativeSpecsExecuted &&
    selection.executed.every(row => selected.includes(row.file)))
  check(name + ': ordinary control executes exactly once in the real native batch',
    markerCount('QUOD_MATRIX_ORDINARY_ONCE') === 1)
  const instance = dispatch.instances.find(row => row.file === candidate)
  assert(instance)
  if (dispatch.replayedInstances === 1) {
    check(name + ': the real native batch omits the hit candidate', selection.omittedAsHits.length === 1 &&
      selection.omittedAsHits[0].file === candidate && selection.executed[0].file === ordinary &&
      markerCount('QUOD_MATRIX_CANDIDATE_ONCE') === 0)
    check(name + ': a hit starts no managed worker', !instance.workerEvidenceDirectory && dispatch.managedExecutions === 0 &&
      !fs.readdirSync(caseOut).some(file => file.startsWith('managed-')))
  } else {
    check(name + ': the complete nonhit native batch retains the candidate',
      selection.omittedAsHits.length === 0 && selection.executed.some(row => row.file === candidate) &&
      markerCount('QUOD_MATRIX_CANDIDATE_ONCE') === 1)
    const response = read(path.join(caseOut, instance.workerEvidenceDirectory, 'response.json'))
    check(name + ': the managed candidate control executes exactly once in its fresh realm',
      response.boundary.logs.filter(row => row[1].includes('QUOD_MATRIX_CANDIDATE_ONCE')).length === 1)
  }
  if (dispatch.exitCode !== 0) check(name + ': no receipt is recorded or materialized after any whole-command failure',
    records === 0 && dispatch.recordedReceipts === 0 && !fs.existsSync(path.join(caseOut, 'receipts')))
  return {dispatch, instance, logical, selection, item}
}

try {
  originals.forEach((bytes, index) => assert.equal(hash(bytes), sourceHashes[index], 'reviewed original fixture source changed'))
  selected.forEach((file, index) => {
    const bytes = Buffer.concat([originals[index], Buffer.from(index ? ordinaryControl : candidateControl)])
    fs.writeFileSync(path.join(project, file), bytes)
    result.sources.push({file, originalSha256: hash(originals[index]), appendedSha256: hash(bytes)})
  })
  save()
  const miss = () => null
  const seeded = await run('seed', 'pass', {lookup: miss, captureAuthority: true})
  check('a conformed cold pass records one receipt after the complete native batch',
    seeded.dispatch.exitCode === 0 && seeded.dispatch.managedExecutions === 1 && seeded.dispatch.nativeSpecsExecuted === 2 &&
    seeded.item.records === 1 && authority.size === 1)
  const fromAuthority = ({execution}) => {
    const receipt = authority.get(execution.key)
    return receipt && {...receipt, receiptBytes: Buffer.from(receipt.receiptBytes)}
  }
  const hit = await run('hit', 'pass', {lookup: fromAuthority})
  check('external synthetic authority admits the current exact execution and preserves all logical states',
    hit.dispatch.exitCode === 0 && hit.dispatch.replayedInstances === 1 && hit.dispatch.nativeSpecsExecuted === 1 &&
    hit.dispatch.logicalSha256 === seeded.dispatch.logicalSha256 && hit.item.records === 0)
  for (const [name, mutate] of [
    ['missing-authority', value => ({receiptBytes: value.receiptBytes})],
    ['wrong-authority', value => ({...value, trustedReceiptSha256: '0'.repeat(64)})],
    ['authenticated-malformed', () => {const receiptBytes = Buffer.from('{"schema":"malformed"}');
      return {receiptBytes, trustedReceiptSha256: hash(receiptBytes)}}],
  ]) {
    const checked = await run(name, 'pass', {lookup: async context => mutate(fromAuthority(context))})
    check(name + ': invalid receipt authority/schema misses and executes managed plus native controls',
      checked.dispatch.exitCode === 0 && checked.dispatch.replayedInstances === 0 && checked.dispatch.managedExecutions === 1 &&
      checked.dispatch.nativeSpecsExecuted === 2 && checked.dispatch.logicalSha256 === seeded.dispatch.logicalSha256)
  }
  const unsupported = await run('unsupported-pass', 'unsupported-pass', {lookup: miss})
  check('caught unsupported fetch makes managed failure non-authoritative and real native pass wins',
    unsupported.dispatch.exitCode === 0 && unsupported.instance.decision === 'ordinary' && unsupported.instance.supported === false &&
    unsupported.instance.unsupported.includes('global fetch') && unsupported.dispatch.stickyManagedFailure === false && unsupported.item.records === 0)
  const unsupportedFailure = await run('unsupported-failure', 'unsupported-failure', {lookup: miss})
  check('unsupported managed execution cannot hide an actual native assertion failure',
    unsupportedFailure.dispatch.exitCode === 1 && unsupportedFailure.instance.decision === 'ordinary' &&
    unsupportedFailure.dispatch.observedNativeFailure === true && unsupportedFailure.dispatch.stickyManagedFailure === false)
  for (const mode of ['assertion-failure', 'hook-failure']) {
    const failed = await run(mode, mode, {lookup: miss})
    check(mode + ': supported assertion or file hook failure stays sticky', failed.dispatch.exitCode === 1 &&
      failed.instance.supported === true && failed.instance.decision === 'supported-failure' && failed.dispatch.stickyManagedFailure === true)
    const response = read(path.join(out, mode, failed.instance.workerEvidenceDirectory, 'response.json'))
    check(mode + ': the actual failure reaches its intended row or file-only channel', mode === 'assertion-failure'
      ? response.rows.some(row => row.state === 'fail')
      : response.ok === false && response.rows.every(row => row.state === 'pass') && response.files.some(file => file.errors.length > 0))
  }
  const ordinaryFailure = await run('ordinary-failure', 'ordinary-failure', {lookup: miss})
  check('ordinary remainder failure prevents publication of an otherwise passing managed candidate',
    ordinaryFailure.dispatch.exitCode === 1 && ordinaryFailure.instance.decision === 'managed' &&
    ordinaryFailure.dispatch.observedNativeFailure === true && ordinaryFailure.dispatch.stickyManagedFailure === false && ordinaryFailure.item.records === 0)
  assert.equal(result.cases.length, 10)
  assert.equal(result.checks.length, 86)
  result.caseCount = 10; result.checkCount = 86
  result.passed = true
} catch (error) {
  result.error = {name: error.name, message: error.message, stack: error.stack}; process.exitCode = 1
} finally {
  selected.forEach((file, index) => fs.writeFileSync(path.join(project, file), originals[index]))
  if (previousMode) process.env[modeName] = previousMode.value
  else delete process.env[modeName]
  result.originalSourcesRestored = selected.every((file, index) => fs.readFileSync(path.join(project, file)).equals(originals[index]))
  if (!result.originalSourcesRestored) {result.passed = false; process.exitCode = 1}
  save(); console.log(JSON.stringify({...result, cases: result.cases.map(row => ({name: row.name, exitCode: row.exitCode}))}))
}
