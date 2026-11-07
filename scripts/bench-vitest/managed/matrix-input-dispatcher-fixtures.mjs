// Remote execution fixtures. Receipt SHA arguments here are explicit synthetic
// test authority; only matrix-sdk-probe exercises actual cache-service trust.
// Original tests remain intact; two appended controls are removed in finally.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {createCertifiedManagedCompiler} from './resolved-compile-certificate.mjs'

const [projectArg, outArg] = process.argv.slice(2)
assert(projectArg && outArg, 'usage: matrix-input-dispatcher-fixtures.mjs PROJECT EMPTY_OUTPUT')
const project = fs.realpathSync(path.resolve(projectArg)), out = path.resolve(outArg)
fs.mkdirSync(out, {recursive: true}); assert.equal(fs.readdirSync(out).length, 0)
// This is the same environment initialization as native matrix preparation.
process.env.TEST = 'true'
process.env.VITEST = 'true'
process.env.NODE_ENV ??= 'test'
await createCertifiedManagedCompiler({project})
const {dispatchManagedInputMatrix} = await import('./matrix-input-dispatcher.mjs')
const {describeManagedSpecification, prepareManagedMatrix} = await import('./preparation.mjs')
const {selectMatrixCandidates} = await import('./matrix-candidates.mjs')
const {verifyMatrixInputProofFixtures} = await import('./matrix-input-proof-fixtures.mjs')
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n')
const candidate = 'packages/convex-helpers/index.test.ts'
const ordinary = 'packages/convex-helpers/functionReferences.test.ts'
const selected = [candidate, ordinary]
const helperTS = path.join(project, 'packages/convex-helpers/__quod_matrix_resolution.ts')
const helperJS = path.join(project, 'packages/convex-helpers/__quod_matrix_resolution.js')
const ownedHelpers = []
const sourceHashes = ['06e78a6fd8e397384c1c2d8b1e38939755882d8ba0d6017d5946cdb468564b2f',
  '09544a8dd35b40754de45d058ccb5627f86c7248c12ba3ce1a55c6ad3dd65fa6']
const modeName = 'QUOD_MATRIX_DISPATCH_FIXTURE'
const previousMode = Object.getOwnPropertyDescriptor(process.env, modeName)
const originals = selected.map(file => fs.readFileSync(path.join(project, file)))
const result = {schema: 'quod-managed-matrix-input-dispatcher-fixtures-v1-research', passed: false,
  actualSdkAuthenticationClaimed: false, productionEnabled: false,
  selectedScope: {kind: 'explicit-file-selection', files: selected}, checks: [], cases: [], sources: []}
const check = (name, passed) => {result.checks.push({name, passed: Boolean(passed)}); assert(passed, name)}
const save = () => write(path.join(out, 'result.json'), result)
const candidateControl = `
import {afterAll as quodMatrixAfterAll} from 'vitest';
import {quodInputValue} from './__quod_matrix_resolution.js';
test('QUOD matrix candidate control', async () => {
  expect(quodInputValue).toBe(7);
  console.log('QUOD_MATRIX_' + 'CANDIDATE_ONCE');
  const mode = process.env.${modeName};
  if (mode === 'outer-cleanup-error' && globalThis.__vitest_worker__?.environment?.name === 'quod-managed') {
    globalThis.__vitest_worker__.onCleanup(() => {throw new Error('QUOD_MATRIX_OUTER_CLEANUP_FAILURE');});
  }
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
  const dispatch = await dispatchManagedInputMatrix({project, out: caseOut, suiteFiles: selected,
    env: {NODE_ENV: 'test', [modeName]: mode}, lookup,
    async record(entry) {
      records++
      const receiptBytes = fs.readFileSync(entry.file)
      assert.equal(hash(receiptBytes), entry.envelopeSha256)
      if (captureAuthority) authority.set(entry.candidateKey, {receiptBytes, trustedReceiptSha256: entry.envelopeSha256})
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
  const logicalBytes = fs.readFileSync(path.join(caseOut, 'logical-results.json'))
  check(name + ': full logical artifact binds its exact bytes and final publication exit',
    dispatch.logicalReport.file === path.join(caseOut, 'logical-results.json') &&
    dispatch.logicalReport.sha256 === hash(logicalBytes) && dispatch.logicalReport.bytes === logicalBytes.length &&
    dispatch.logicalReport.schema === logical.schema && dispatch.logicalReport.instances === 2 &&
    dispatch.logicalReport.states === 13 && logical.exitCode === dispatch.exitCode)
  check(name + ': logical file identities and states retain their actual native or replay provenance',
    logical.files.length === 2 && new Set(logical.files.map(file => file.instanceId)).size === 2 && logical.files.every(file => {
      const actual = selection.executed.find(row => row.instanceId === file.instanceId)
      const instance = dispatch.instances.find(row => row.instanceId === file.instanceId)
      if (!instance || file.project !== instance.project || file.file !== instance.file || file.pool !== instance.pool) return false
      if (actual) return file.name === actual.relativeModuleId && file.state === actual.state
      const replay = read(path.join(caseOut, instance.evidenceDirectory, 'replayed-response.json'))
      return instance.decision === 'replay' && replay.files.length === 1 &&
        file.name === replay.files[0].name && file.state === replay.files[0].state
    }))
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
  for (const file of [helperTS, helperJS]) assert(!fs.lstatSync(file, {throwIfNoEntry: false}), 'fixture helper path already exists')
  ownedHelpers.push(helperTS)
  fs.writeFileSync(helperTS, 'export const quodInputValue = 7;\n', {flag: 'wx'})
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
  const actual = seeded.selection.originalSelected.find(row => row.file === candidate)
  const specification = {...actual, moduleId: actual.absolute, project: {config: {name: actual.project}}}
  const absentLines = describeManagedSpecification({...specification, testLines: undefined}, project)
  const emptyLines = describeManagedSpecification({...specification, testLines: []}, project)
  const selectedLines = describeManagedSpecification({...specification, testLines: [1]}, project)
  check('empty and absent test-line selections share the actual unfiltered identity',
    emptyLines.testLines === null && absentLines.testLines === null &&
    emptyLines.instanceId === absentLines.instanceId && emptyLines.instanceId === actual.instanceId)
  check('nonempty test-line selection remains distinct and outside reviewed candidates',
    selectedLines.testLines.length === 1 && selectedLines.testLines[0] === 1 &&
    selectedLines.instanceId !== actual.instanceId && selectMatrixCandidates([selectedLines]).length === 0)
  let lineRejection
  try {await prepareManagedMatrix({project, seed: 42, files: [candidate + ':1']})}
  catch (error) {lineRejection = {name: error.name, code: error.code, message: error.message}}
  result.nonemptyLineFilterRejection = lineRejection
  check('real line-filter preparation remains unsupported under reviewed configuration',
    lineRejection?.code === 'VITEST_INCLUDE_TASK_LOCATION_DISABLED')
  const fromAuthority = ({candidateKey}) => {
    const receipt = authority.get(candidateKey)
    return receipt && {...receipt, receiptBytes: Buffer.from(receipt.receiptBytes)}
  }
  const hit = await run('hit', 'pass', {lookup: fromAuthority})
  check('external synthetic authority admits the current exact execution and preserves all logical states',
    hit.dispatch.exitCode === 0 && hit.dispatch.replayedInstances === 1 && hit.dispatch.nativeSpecsExecuted === 1 &&
    hit.dispatch.logicalSha256 === seeded.dispatch.logicalSha256 && hit.item.records === 0 &&
    hit.dispatch.skippedCompilations === 1 && hit.instance.compilationSkipped === true && hit.instance.compilationSeconds === 0)
  result.inputEnvelopeControls = await verifyMatrixInputProofFixtures({project, seedOut: path.join(out, 'seed'),
    selected, out: path.join(out, 'input-proof-controls.json')})
  const candidateFile = path.join(project, candidate), appendedBytes = fs.readFileSync(candidateFile)
  try {
    fs.appendFileSync(candidateFile, '\n// full-byte input proof mutation\n')
    const changed = await run('source-byte-miss', 'pass', {lookup: fromAuthority})
    check('changing source bytes while retaining all assertions declines the old input proof',
      changed.dispatch.exitCode === 0 && changed.dispatch.replayedInstances === 0 &&
      changed.dispatch.managedExecutions === 1 && changed.dispatch.skippedCompilations === 0 &&
      changed.dispatch.nativeSpecsExecuted === 2 && changed.dispatch.logicalSha256 === seeded.dispatch.logicalSha256 &&
      typeof changed.instance.inputMissReason === 'string')
  } finally {fs.writeFileSync(candidateFile, appendedBytes)}
  try {
    ownedHelpers.push(helperJS)
    fs.writeFileSync(helperJS, 'export const quodInputValue = 7;\n', {flag: 'wx'})
    const shadowed = await run('fresh-resolution-miss', 'pass', {lookup: fromAuthority})
    check('a new higher-priority JS resolution target declines the old proof despite unchanged prior source bytes',
      shadowed.dispatch.exitCode === 0 && shadowed.dispatch.replayedInstances === 0 &&
      shadowed.dispatch.managedExecutions === 1 && shadowed.dispatch.skippedCompilations === 0 &&
      shadowed.dispatch.nativeSpecsExecuted === 2 && shadowed.dispatch.logicalSha256 === seeded.dispatch.logicalSha256 &&
      typeof shadowed.instance.inputMissReason === 'string')
  } finally {fs.rmSync(helperJS, {force: true})}
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
  const outerFailure = await run('outer-cleanup-error', 'outer-cleanup-error', {lookup: miss})
  const outerResponse = read(path.join(out, 'outer-cleanup-error', outerFailure.instance.workerEvidenceDirectory, 'response.json'))
  check('outer-cleanup-error: sealed managed exception remains sticky without reusable row/file shape',
    outerFailure.dispatch.exitCode === 1 && outerFailure.instance.closedFailure === true &&
    outerFailure.instance.supported === false && outerFailure.instance.decision === 'ordinary' &&
    outerFailure.dispatch.stickyManagedFailure === true && outerFailure.item.records === 0)
  check('outer-cleanup-error: the actual runner catch retains the exception and closed empty-denial boundary',
    outerResponse.ok === false && outerResponse.error?.message.includes('QUOD_MATRIX_OUTER_CLEANUP_FAILURE') &&
    !Object.hasOwn(outerResponse, 'rows') && !Object.hasOwn(outerResponse, 'files') &&
    outerResponse.boundary.completion.sealed === true && outerResponse.boundary.unsupported.length === 0)
  check('outer-cleanup-error: the exact ordinary counterpart passes all thirteen states',
    outerFailure.dispatch.observedNativeFailure === false && outerFailure.dispatch.native.exitCode === 0 &&
    outerFailure.selection.exitCode === 0 && outerFailure.selection.failedModules === 0 &&
    outerFailure.selection.unhandledErrors.length === 0 && outerFailure.logical.rows.length === 13 &&
    outerFailure.logical.rows.every(row => ['pass', 'skip', 'todo'].includes(row.state)))
  const ordinaryFailure = await run('ordinary-failure', 'ordinary-failure', {lookup: miss})
  check('ordinary remainder failure prevents publication of an otherwise passing managed candidate',
    ordinaryFailure.dispatch.exitCode === 1 && ordinaryFailure.instance.decision === 'managed' &&
    ordinaryFailure.dispatch.observedNativeFailure === true && ordinaryFailure.dispatch.stickyManagedFailure === false && ordinaryFailure.item.records === 0)
  assert.equal(result.cases.length, 13)
  result.caseCount = result.cases.length; result.checkCount = result.checks.length
  result.passed = true
} catch (error) {
  result.error = {name: error.name, message: error.message, stack: error.stack}; process.exitCode = 1
} finally {
  selected.forEach((file, index) => fs.writeFileSync(path.join(project, file), originals[index]))
  for (const file of ownedHelpers) fs.rmSync(file, {force: true})
  if (previousMode) process.env[modeName] = previousMode.value
  else delete process.env[modeName]
  result.originalSourcesRestored = selected.every((file, index) => fs.readFileSync(path.join(project, file)).equals(originals[index]))
  if (!result.originalSourcesRestored) {result.passed = false; process.exitCode = 1}
  save(); console.log(JSON.stringify({...result, cases: result.cases.map(row => ({name: row.name, exitCode: row.exitCode}))}))
}
