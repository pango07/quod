// Remote-only integration draft. Authority here is synthetic fixture authority;
// this never claims actual SDK authentication or enables an Action integration.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {fileURLToPath} from 'node:url'
import {spawnSync} from 'node:child_process'
import {createCertifiedManagedCompiler, canonicalManagedJSON as json} from './resolved-compile-certificate.mjs'

assert.equal(process.env.GITHUB_ACTIONS, 'true', 'remote-only batch dispatcher fixtures')
const self = fileURLToPath(import.meta.url)
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const write = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', {flag: 'wx'})
const selected = ['packages/convex-helpers/index.test.ts', 'packages/convex-helpers/browser.test.ts',
  'packages/convex-helpers/functionReferences.test.ts']
const originalsSHA = ['06e78a6fd8e397384c1c2d8b1e38939755882d8ba0d6017d5946cdb468564b2f',
  '7ddc21e36493d08c848bea3fd195f8766df0ab5bed397083049d1832bede73e9',
  '09544a8dd35b40754de45d058ccb5627f86c7248c12ba3ce1a55c6ad3dd65fa6']
const markers = ['QUOD_BATCH_INDEX_ONCE', 'QUOD_BATCH_BROWSER_ONCE', 'QUOD_BATCH_ORDINARY_ONCE']
const modeNames = ['seed', 'hit', 'refresh', 'mixed', 'rollback', 'rollback-native-failure']
const commonChecks = [
  'original three-file scope retains two candidates and one ordinary file',
  'native selection completes once and all actual file identities remain visible',
  'full logical artifact binds exact bytes and final exit',
  'ordinary test executes exactly once',
  'every lookup finishes before records/publication',
]
const passingChecks = [
  'cold members use exactly one closed compiler batch',
  'workers remain fresh serial operations in original instance order',
  'only complete admitted replays omit their ordinary counterpart',
  'all successful logical states remain passing',
  'publication callback retains the original final-exit contract',
]
const rollbackChecks = [
  'real unsupported native import declines the actual batch before any candidate worker',
  'one previously valid proof/receipt stays provisional and is discarded',
  'one complete real native fallback executes all original specs with no hit omission',
  'batch-admission decline cannot record or publish even after native pass',
  'native exit remains authoritative without failure erasure',
  'failed reservation poisons only this disposable process',
]
const childInventory = Object.fromEntries(modeNames.map(mode => [mode, [...commonChecks,
  ...(mode.startsWith('rollback') ? rollbackChecks : passingChecks),
  ...(mode === 'refresh' ? ['conservative certificate miss retains full byte-ledger hit and refresh rule'] : []),
  ...(mode === 'mixed' ? ['one full-source miss cannot suppress another current authenticated hit'] : [])]]))
const parentInventory = ['all three reviewed original source files are present before any child',
  ...modeNames.flatMap(mode => [mode + ': fresh process preserves every own integration check',
    mode === 'seed' ? 'seed records two exact complete receipts after actual managed/native passes'
      : mode === 'rollback-native-failure' ? 'native failure is retained in the exact changed ordinary control row'
        : mode + ': all logical project/name/states and API file states equal uncached control',
    mode + ': source edits are restored before any other disposable process'])]
const stateRows = logical => logical.rows.map(row => json([row.project, row.file, row.pool, row.name, row.state])).sort()
const fileRows = logical => logical.files.map(row => json([row.project, row.file, row.pool, row.name, row.state])).sort()
const errorData = error => ({name: String(error.name), message: String(error.message), stack: String(error.stack)})

async function fixtureChild(requestFile) {
  const {project, out, mode, authority = []} = read(requestFile)
  assert(modeNames.includes(mode)); assert(!fs.existsSync(out)); fs.mkdirSync(out)
  process.env.TEST = 'true'; process.env.VITEST = 'true'; process.env.NODE_ENV ??= 'test'
  await createCertifiedManagedCompiler({project})
  const {dispatchManagedInputMatrix} = await import('./matrix-input-dispatcher.mjs')
  const {decodeMatrixCandidateEnvelope, encodeMatrixCandidateEnvelope} = await import('./matrix-candidate-envelope.mjs')
  const originals = selected.map(file => fs.readFileSync(path.join(project, file)))
  const result = {schema: 'quod-managed-matrix-batch-dispatcher-case-v1', mode, passed: false,
    actualSdkAuthenticationClaimed: false, checks: [], callbacks: [], recorded: [], sources: []}
  const check = (name, pass) => {result.checks.push({name, pass: Boolean(pass)}); assert(pass, name)}
  const dispatchOut = path.join(out, 'dispatch'), rollback = mode.startsWith('rollback')
  let mutatedAuthority
  try {
    originals.forEach((bytes, index) => assert.equal(sha(bytes), originalsSHA[index], 'reviewed fixture source changed'))
    fs.mkdirSync(path.join(out, 'sources'))
    for (const [index, relative] of selected.entries()) {
      const nativeImport = rollback && index === 1 ? 'import {readFileSync as quodBatchRead} from "node:fs";\n' : ''
      const assertion = rollback && index === 1 ? 'quodBatchExpect(typeof quodBatchRead).toBe("function");'
        : mode === 'rollback-native-failure' && index === 2 ? 'quodBatchExpect(false).toBe(true);'
          : 'quodBatchExpect(true).toBe(true);'
      const suffix = '\n' + nativeImport + 'import {test as quodBatchTest,expect as quodBatchExpect} from "vitest";\n' +
        'quodBatchTest(' + JSON.stringify('QUOD batch control ' + index) + ',()=>{console.log(' + JSON.stringify(markers[index]) + ');' + assertion + '});\n' +
        (mode === 'mixed' && index === 1 ? '// QUOD_BATCH_CURRENT_SOURCE_MISS\n' : '')
      const bytes = Buffer.concat([originals[index], Buffer.from(suffix)])
      fs.writeFileSync(path.join(project, relative), bytes)
      fs.writeFileSync(path.join(out, 'sources', String(index) + '.test.ts'), bytes, {flag: 'wx'})
      result.sources.push({file: relative, originalSha256: sha(originals[index]), currentSha256: sha(bytes)})
    }
    const byKey = new Map(authority.map(entry => [entry.candidateKey, entry]))
    if (mode === 'refresh') {
      const original = authority.find(entry => entry.testFile === selected[1]); assert(original)
      const envelopeBytes = fs.readFileSync(original.file)
      const decoded = decodeMatrixCandidateEnvelope({candidateKey: original.candidateKey, envelopeBytes,
        trustedEnvelopeSha256: original.envelopeSha256})
      const certificate = JSON.parse(decoded.certificateBytes)
      assert(certificate.sourceFiles.length > 0)
      certificate.sourceFiles[0].rawSha256 = '0'.repeat(64)
      const mutated = encodeMatrixCandidateEnvelope({candidateKey: original.candidateKey,
        certificateBytes: Buffer.from(json(certificate) + '\n'), receiptBytes: decoded.receiptBytes})
      mutatedAuthority = {receiptBytes: mutated.envelopeBytes, trustedReceiptSha256: mutated.envelopeSha256}
      fs.writeFileSync(path.join(out, 'refresh-authenticated-test-envelope.json'), mutated.envelopeBytes, {flag: 'wx'})
    }
    const dispatch = await dispatchManagedInputMatrix({project, out: dispatchOut, suiteFiles: selected,
      env: {NODE_ENV: 'test'},
      async lookup({candidateKey}) {
        assert(!fs.existsSync(path.join(dispatchOut, 'batch-compilation.json')), 'lookup ran after reserved compilation')
        const entry = byKey.get(candidateKey)
        result.callbacks.push({operation: 'lookup', candidateKey, hit: Boolean(entry)})
        if (!entry) return null
        if (mutatedAuthority && entry.testFile === selected[1]) return mutatedAuthority
        const receiptBytes = fs.readFileSync(entry.file)
        assert.equal(sha(receiptBytes), entry.envelopeSha256)
        return {receiptBytes, trustedReceiptSha256: entry.envelopeSha256}
      },
      async record(entry) {
        const native = read(path.join(dispatchOut, 'native/native-selection.json'))
        assert(native.completed === true && native.exitCode === 0, 'record preceded successful native completion')
        const bytes = fs.readFileSync(entry.file)
        assert.equal(sha(bytes), entry.envelopeSha256)
        result.callbacks.push({operation: 'record', candidateKey: entry.candidateKey})
        result.recorded.push({candidateKey: entry.candidateKey, envelopeSha256: entry.envelopeSha256,
          file: entry.file, testFile: entry.execution.identity.instance.file, executionKey: entry.execution.key})
      },
      async publish({exitCode}) {
        result.callbacks.push({operation: 'publish', exitCode})
        return {saved: false, reason: 'synthetic-fixture-no-SDK'}
      }})
    result.dispatch = dispatch
    const logical = read(path.join(dispatchOut, 'logical-results.json'))
    const native = read(path.join(dispatchOut, 'native/native-selection.json'))
    const nativeLog = fs.readFileSync(path.join(dispatchOut, 'native/native.log'), 'utf8').replace(/\x1b\[[0-9;]*m/g, '')
    const markerCount = marker => nativeLog.split(/\r?\n/).filter(line => line.trim() === marker).length
    const candidates = dispatch.instances.filter(row => row.candidate)
    const workers = dispatch.phases.filter(phase => phase.name.startsWith('managed-'))
    check('original three-file scope retains two candidates and one ordinary file', dispatch.selectedInstances === 3 &&
      dispatch.candidateInstances === 2 && dispatch.initialOrdinaryInstances === 1 && dispatch.fullJobHit === false)
    check('native selection completes once and all actual file identities remain visible', native.completed === true &&
      native.matched === true && native.originalSelected.length === 3 && native.executed.length === dispatch.nativeSpecsExecuted &&
      logical.files.length === 3 && new Set(logical.files.map(file => file.instanceId)).size === 3)
    check('full logical artifact binds exact bytes and final exit', dispatch.logicalReport.sha256 === sha(fs.readFileSync(
      path.join(dispatchOut, 'logical-results.json'))) && logical.exitCode === dispatch.exitCode)
    check('ordinary test executes exactly once', markerCount(markers[2]) === 1)
    check('every lookup finishes before records/publication', result.callbacks.map(event => event.operation).join(',').match(
      /^lookup,lookup(?:,record)*(?:,publish)?$/) !== null)
    if (rollback) {
      const batch = read(path.join(dispatchOut, 'batch-compilation.json'))
      check('real unsupported native import declines the actual batch before any candidate worker', batch.attempted === true &&
        batch.requests === 1 && batch.completed === false && batch.error.message.includes('only a named AsyncLocalStorage import is supported: node:fs') &&
        dispatch.managedExecutions === 0 && workers.length === 0 && dispatch.phases.every(phase => phase.name === 'preflight'))
      check('one previously valid proof/receipt stays provisional and is discarded', dispatch.inputCandidates === 2 &&
        read(path.join(dispatchOut, candidates.find(row => row.file === selected[0]).evidenceDirectory, 'input-decision.json')).hit === true &&
        dispatch.replayedInstances === 0 && dispatch.skippedCompilations === 0 && candidates.every(row => row.decision === 'ordinary' &&
          !row.compilationSkipped && !row.workerEvidenceDirectory && !row.receiptSha256 &&
          !fs.existsSync(path.join(dispatchOut, row.evidenceDirectory, 'replayed-response.json'))))
      check('one complete real native fallback executes all original specs with no hit omission', native.executed.length === 3 &&
        native.omittedAsHits.length === 0 && selected.every((file, index) => markerCount(markers[index]) === 1))
      check('batch-admission decline cannot record or publish even after native pass', result.recorded.length === 0 &&
        !result.callbacks.some(event => event.operation === 'publish') && dispatch.recordedReceipts === 0 &&
        !fs.existsSync(path.join(dispatchOut, 'receipts')) && dispatch.publication.saved === false)
      check('native exit remains authoritative without failure erasure', dispatch.exitCode === (mode === 'rollback' ? 0 : 1) &&
        dispatch.observedNativeFailure === (mode !== 'rollback') && dispatch.stickyManagedFailure === false)
      let poisoned
      try {await createCertifiedManagedCompiler({project})} catch (error) {poisoned = error.message}
      check('failed reservation poisons only this disposable process', poisoned?.includes('fresh process'))
    } else {
      const expectedCold = mode === 'seed' ? 2 : mode === 'hit' ? 0 : 1
      const expectedWorkers = mode === 'seed' ? 2 : mode === 'mixed' ? 1 : 0
      const expectedReplays = 2 - expectedWorkers
      check('cold members use exactly one closed compiler batch', dispatch.compilationBatch.requests === expectedCold &&
        dispatch.compilationBatch.attempted === (expectedCold > 0) && (expectedCold === 0 ||
          dispatch.compilationBatch.completed === true && dispatch.compilationBatch.diagnostics.fullFences === 2 * expectedCold + 2))
      check('workers remain fresh serial operations in original instance order', dispatch.managedExecutions === expectedWorkers &&
        workers.length === expectedWorkers && json(workers.map(phase => phase.name)) === json(dispatch.instances.filter(
          row => row.workerEvidenceDirectory).map(row => row.workerEvidenceDirectory)))
      check('only complete admitted replays omit their ordinary counterpart', dispatch.replayedInstances === expectedReplays &&
        native.executed.length === 3 - expectedReplays && native.omittedAsHits.length === expectedReplays)
      check('all successful logical states remain passing', dispatch.exitCode === 0 && logical.rows.every(row => ['pass', 'skip', 'todo'].includes(row.state)))
      check('publication callback retains the original final-exit contract', result.callbacks.filter(event => event.operation === 'publish').length === 1 &&
        result.callbacks.at(-1).operation === 'publish' && result.callbacks.at(-1).exitCode === 0)
      if (mode === 'refresh') check('conservative certificate miss retains full byte-ledger hit and refresh rule',
        dispatch.refreshedCandidates === 1 && result.recorded.length === 1 && dispatch.managedExecutions === 0 &&
        dispatch.skippedCompilations === 1 && candidates.find(row => row.file === selected[1]).inputMissReason === 'current full source bytes differ')
      if (mode === 'mixed') check('one full-source miss cannot suppress another current authenticated hit',
        dispatch.recordedReceipts === 1 && dispatch.skippedCompilations === 1 &&
        typeof candidates.find(row => row.file === selected[1]).inputMissReason === 'string')
    }
    result.logicalStateRows = stateRows(logical); result.logicalFileRows = fileRows(logical)
    assert.deepEqual(result.checks, childInventory[mode].map(name => ({name, pass: true})), 'exact named child check inventory differs')
    result.passed = true
  } catch (error) {result.error = errorData(error); process.exitCode = 1}
  finally {
    originals.forEach((bytes, index) => fs.writeFileSync(path.join(project, selected[index]), bytes))
    result.originalSourcesRestored = originals.every((bytes, index) => fs.readFileSync(path.join(project, selected[index])).equals(bytes))
    if (!result.originalSourcesRestored) {result.passed = false; process.exitCode = 1}
    result.checkCount = result.checks.length; result.expectedCheckCount = childInventory[mode].length
    write(path.join(out, 'result.json'), result)
  }
}

async function controller(projectArg, outArg) {
  assert(projectArg && outArg, 'usage: matrix-batch-dispatcher-fixtures.mjs PROJECT EMPTY_OUTPUT')
  const project = fs.realpathSync(projectArg), out = path.resolve(outArg)
  assert(!fs.existsSync(out)); fs.mkdirSync(out)
  const result = {schema: 'quod-managed-matrix-batch-dispatcher-fixtures-v1', passed: false,
    actualSdkAuthenticationClaimed: false, originalDispatcherChecksModified: false, cases: [], checks: []}
  const check = (name, pass) => {result.checks.push({name, pass: Boolean(pass)}); assert(pass, name)}
  const originals = selected.map(file => fs.readFileSync(path.join(project, file)))
  let ownsSources = false
  let seed
  try {
    check('all three reviewed original source files are present before any child', originals.every((bytes, index) => sha(bytes) === originalsSHA[index]))
    ownsSources = true
    for (const mode of modeNames) {
      const requestFile = path.join(out, mode + '-request.json'), directory = path.join(out, mode)
      write(requestFile, {project, out: directory, mode, authority: seed?.recorded || []})
      const started = performance.now(), child = spawnSync(process.execPath, [self, '--child', requestFile],
        {cwd: project, env: process.env, timeout: 180000, killSignal: 'SIGKILL', maxBuffer: 8 * 1024 * 1024})
      fs.writeFileSync(path.join(out, mode + '-stdout.log'), child.stdout || Buffer.alloc(0), {flag: 'wx'})
      fs.writeFileSync(path.join(out, mode + '-stderr.log'), child.stderr || Buffer.alloc(0), {flag: 'wx'})
      const actual = fs.existsSync(path.join(directory, 'result.json')) ? read(path.join(directory, 'result.json')) : null
      result.cases.push({mode, passed: actual?.passed === true, status: child.status, signal: child.signal,
        error: child.error ? errorData(child.error) : null, seconds: (performance.now() - started) / 1000})
      check(mode + ': fresh process preserves every own integration check', !child.error && child.status === 0 && child.signal === null &&
        actual?.passed === true && actual.originalSourcesRestored === true && actual.checkCount === childInventory[mode].length &&
        json(actual.checks) === json(childInventory[mode].map(name => ({name, pass: true}))))
      if (mode === 'seed') {
        seed = actual; check('seed records two exact complete receipts after actual managed/native passes', seed.recorded.length === 2)
      } else if (mode !== 'rollback-native-failure') {
        check(mode + ': all logical project/name/states and API file states equal uncached control',
          json(actual.logicalStateRows) === json(seed.logicalStateRows) && json(actual.logicalFileRows) === json(seed.logicalFileRows))
      } else check('native failure is retained in the exact changed ordinary control row',
        actual.logicalStateRows.filter(row => JSON.parse(row)[4] === 'fail').length === 1 &&
        actual.logicalFileRows.filter(row => JSON.parse(row)[4] === 'fail').length === 1)
      check(mode + ': source edits are restored before any other disposable process', selected.every((file, index) =>
        sha(fs.readFileSync(path.join(project, file))) === originalsSHA[index]))
    }
    assert.deepEqual(result.cases.map(row => row.mode), modeNames, 'exact six fresh-process case inventory differs')
    assert.deepEqual(result.checks, parentInventory.map(name => ({name, pass: true})), 'exact named parent check inventory differs')
    result.passed = true
  } catch (error) {result.error = errorData(error); process.exitCode = 1}
  finally {
    // A watchdog may prevent the disposable child's finally from running.
    // The controller owns only these three already-verified original paths.
    if (ownsSources) originals.forEach((bytes, index) => fs.writeFileSync(path.join(project, selected[index]), bytes))
    result.caseCount = result.cases.length; result.checkCount = result.checks.length
    result.expectedCheckCount = parentInventory.length
    result.caseInventories = Object.fromEntries(modeNames.map(mode => [mode, childInventory[mode]]))
    result.originalSourcesRestored = selected.every((file, index) => fs.readFileSync(path.join(project, file)).equals(originals[index]))
    if (!result.originalSourcesRestored) {result.passed = false; process.exitCode = 1}
    write(path.join(out, 'result.json'), result)
  }
}
if (process.argv[2] === '--child') await fixtureChild(process.argv[3])
else await controller(...process.argv.slice(2))
