// Remote-only finite-profile diagnostic. Synthetic envelope SHA authority here
// is deliberately separate from actual SDK transport authentication.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'
import {createCertifiedManagedCompiler, canonicalManagedJSON as json} from './resolved-compile-certificate.mjs'
import {MATRIX_CANDIDATE_POLICY, MATRIX_CANDIDATE_ROSTER} from './matrix-candidates.mjs'
import {PROPERTY47_CANDIDATE_PROFILE as profile, PROPERTY47_CANDIDATE as target,
  resolveMatrixCandidateProfile, selectMatrixProfileCandidates} from './matrix-candidate-profiles.mjs'
import {MANAGED_BOUNDARY_CONTRACT, MANAGED_INTRINSIC_POLICY} from './receipt.mjs'

assert.equal(process.env.GITHUB_ACTIONS, 'true', 'remote-only property47 profile diagnostic')
const self = fileURLToPath(import.meta.url), sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n', {flag: 'wx'})
const modes = ['seed', 'hit', 'refresh', 'mixed-routing', 'rollback', 'rollback-native-failure']
const oneBatchTrace = ['before-constructor', 'after-constructor/before-compile:0',
  'after-compile:0/before-capture:0', 'after-capture:0/before-publication']
const ordinary = 'packages/convex-helpers/functionReferences.test.ts'
const sourceFiles = [target.file, ordinary]
const originalSHA = ['9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75',
  '09544a8dd35b40754de45d058ccb5627f86c7248c12ba3ce1a55c6ad3dd65fa6']
const sentinel = 'QUOD property profile native failure sentinel'
const stateRows = files => files.flatMap(file => file.rows.map(row =>
  json([file.project, file.file, file.pool, row.name, row.state]))).sort()
const fileRows = files => files.map(file =>
  json([file.project, file.file, file.pool, file.relativeModuleId, file.state])).sort()
const logicalStates = data => data.rows.map(row => json([row.project, row.file, row.pool, row.name, row.state])).sort()
const logicalFiles = data => data.files.map(row => json([row.project, row.file, row.pool, row.name, row.state])).sort()
const errorData = error => ({name: String(error?.name || 'Error'), message: String(error?.message || error), stack: String(error?.stack || '')})
const closed = data => {
  const boundary = data?.boundary, completion = boundary?.completion
  return data?.ok === true && boundary?.contract === MANAGED_BOUNDARY_CONTRACT && boundary.intrinsicPolicy === MANAGED_INTRINSIC_POLICY &&
    completion?.sealed === true && completion.intrinsicLockdown === true && completion.reason === '' && boundary.pendingTimers === 0 &&
    ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'].every(key => completion[key] === 0) &&
    ['unsupported', 'unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors'].every(key =>
      Array.isArray(boundary[key]) && boundary[key].length === 0)
}
const common = [
  'complete original120 selection retains one exact property candidate and119 ordinary instances',
  'active profile roster and selector bytes are bound into every current execution',
  'current logical report preserves all original4131 project/name/states and API file states',
  'native selection is exact and complete for the nonhit IDs',
  'all lookup callbacks precede any record or publication',
  'only sealed current data can omit the exact property counterpart',
  'whole-command exit and publication retain sticky failure semantics',
  'this diagnostic claims synthetic authority only and never a full-job hit',
]
const extra = {
  seed: ['one guarded cold batch retains four full fences and one fresh47-state worker',
    'native120 provides actual cold conformance before one receipt record'],
  hit: ['one authenticated current-input proof skips the only candidate compile and worker'],
  refresh: ['coherent stale-source certificate triggers one guarded compile then exact byte-ledger refresh with no worker'],
  'mixed-routing': ['one unrelated historical route cannot create a second candidate or lookup'],
  rollback: ['real node:fs source import declines the reserved batch before any candidate worker',
    'full native120 fallback runs once without omitting a candidate',
    'reserved failure poisons only its disposable process'],
  'rollback-native-failure': ['real node:fs source import declines the reserved batch before any candidate worker',
    'full native120 fallback retains the actual appended native assertion failure without retry',
    'reserved failure poisons only its disposable process'],
}
const inventory = Object.fromEntries(modes.map(mode => [mode, [...common, ...extra[mode]]]))
const invalidProfiles = [null, {}, [], 47, true, new String(profile), 'unknown-property-profile']
const invalidProfileName = value => 'nonprimitive or unknown profile rejected: ' + json(typeof value === 'object' ? String(value) : value)
const parentInventory = [
  'reviewed original source bytes exist before any child',
  'default108 profile label and exact roster remain unchanged',
  'property profile has only the independently reviewed exact tuple',
  ...invalidProfiles.map(invalidProfileName),
  'missing target leaves every other actual instance ordinary',
  ...['project', 'file', 'pool'].map(key => 'same ID cannot substitute property identity: ' + key),
  'duplicate property instance rejected',
  ...modes.flatMap(mode => [mode + ': all exact fresh child controls pass',
    ...(mode === 'seed' ? ['seed retains exactly one synthetic conformed candidate'] : []),
    mode + ': original source bytes restored before next process']),
]

async function child(requestFile) {
  const {project, out, baselineFile, mode, authority} = read(requestFile)
  assert(modes.includes(mode)); assert(!fs.existsSync(out)); fs.mkdirSync(out)
  const result = {schema: 'quod-managed-property47-profile-case-v1-research', mode, passed: false,
    actualSdkAuthenticationClaimed: false, fullOriginalSelection: true, checks: [], callbacks: [], recorded: [], sources: []}
  const check = (name, value) => {result.checks.push({name, pass: Boolean(value)}); assert(value, name)}
  const originals = sourceFiles.map(file => fs.readFileSync(path.join(project, file)))
  const dispatchOut = path.join(out, 'dispatch'), rollback = mode.startsWith('rollback')
  let ownsSources = false
  try {
    originals.forEach((bytes, index) => assert.equal(sha(bytes), originalSHA[index], 'reviewed original source changed'))
    ownsSources = true
    if (rollback) fs.appendFileSync(path.join(project, target.file),
      '\nimport {readFileSync as quodProperty47Read} from "node:fs";\n' +
      'if (typeof quodProperty47Read !== "function") throw new Error("QUOD property native import unavailable");\n')
    if (mode === 'rollback-native-failure') fs.appendFileSync(path.join(project, ordinary),
      '\nimport {test as quodProperty47Test,expect as quodProperty47Expect} from "vitest";\n' +
      'quodProperty47Test(' + JSON.stringify(sentinel) + ',()=>quodProperty47Expect(false).toBe(true));\n')
    fs.mkdirSync(path.join(out, 'sources'))
    sourceFiles.forEach((file, index) => {
      const bytes = fs.readFileSync(path.join(project, file))
      fs.writeFileSync(path.join(out, 'sources', String(index) + '.test.ts'), bytes, {flag: 'wx'})
      result.sources.push({file, originalSha256: sha(originals[index]), currentSha256: sha(bytes)})
    })
    process.env.TEST = 'true'; process.env.VITEST = 'true'; process.env.NODE_ENV ??= 'test'
    await createCertifiedManagedCompiler({project})
    const {dispatchManagedInputMatrix} = await import('./matrix-input-dispatcher.mjs')
    const {decodeMatrixCandidateEnvelope, encodeMatrixCandidateEnvelope} = await import('./matrix-candidate-envelope.mjs')
    const byKey = new Map((authority || []).map(row => [row.candidateKey, row]))
    let modified
    if (mode === 'refresh') {
      assert.equal(byKey.size, 1)
      const entry = authority[0], envelopeBytes = fs.readFileSync(entry.file)
      const decoded = decodeMatrixCandidateEnvelope({candidateKey: entry.candidateKey, envelopeBytes,
        trustedEnvelopeSha256: entry.envelopeSha256})
      const certificate = JSON.parse(decoded.certificateBytes)
      assert(certificate.sourceFiles.length > 0)
      certificate.sourceFiles[0].rawSha256 = '0'.repeat(64)
      const encoded = encodeMatrixCandidateEnvelope({candidateKey: entry.candidateKey,
        certificateBytes: Buffer.from(json(certificate) + '\n'), receiptBytes: decoded.receiptBytes})
      modified = {receiptBytes: encoded.envelopeBytes, trustedReceiptSha256: encoded.envelopeSha256}
      fs.writeFileSync(path.join(out, 'synthetic-refresh-envelope.json'), encoded.envelopeBytes, {flag: 'wx'})
    }
    const dispatch = await dispatchManagedInputMatrix({project, out: dispatchOut, candidateProfile: profile,
      async lookup({candidateKey}) {
        assert(!fs.existsSync(path.join(dispatchOut, 'batch-compilation.json')), 'lookup ran after reserved batch')
        const entry = byKey.get(candidateKey)
        result.callbacks.push({operation: 'lookup', candidateKey, hit: Boolean(entry)})
        if (!entry) return null
        if (modified) return modified
        const receiptBytes = fs.readFileSync(entry.file); assert.equal(sha(receiptBytes), entry.envelopeSha256)
        return {receiptBytes, trustedReceiptSha256: entry.envelopeSha256}
      },
      async record(entry) {
        const native = read(path.join(dispatchOut, 'native/native-selection.json'))
        assert(native.completed === true && native.exitCode === 0, 'record preceded successful native completion')
        assert.equal(entry.execution.identity.instance.file, target.file)
        const bytes = fs.readFileSync(entry.file); assert.equal(sha(bytes), entry.envelopeSha256)
        result.callbacks.push({operation: 'record', candidateKey: entry.candidateKey})
        result.recorded.push({candidateKey: entry.candidateKey, envelopeSha256: entry.envelopeSha256,
          file: entry.file, executionKey: entry.execution.key, testFile: target.file})
      },
      async publish({exitCode}) {
        result.callbacks.push({operation: 'publish', exitCode})
        return {saved: false, reason: 'synthetic-profile-diagnostic-no-SDK'}
      }})
    result.dispatch = dispatch
    const baseline = read(baselineFile), logical = read(path.join(dispatchOut, 'logical-results.json'))
    assert(baseline.completed === true && baseline.executed.length === 120 && stateRows(baseline.executed).length === 4131)
    const native = read(path.join(dispatchOut, 'native/native-selection.json'))
    const candidates = dispatch.instances.filter(row => row.candidate), candidate = candidates[0]
    const workers = dispatch.phases.filter(row => row.name.startsWith('managed-'))
    const directories = fs.readdirSync(path.join(dispatchOut, 'instances'), {withFileTypes: true})
    assert.equal(directories.length, 1); assert(directories[0].isDirectory() && directories[0].name === target.instanceId)
    const artifactCounts = {
      builds: directories.filter(dir => fs.existsSync(path.join(dispatchOut, 'instances', dir.name, 'test-build.json'))).length,
      certificates: directories.filter(dir => fs.existsSync(path.join(dispatchOut, 'instances', dir.name, 'compile-certificate.json'))).length,
      workerDirectories: fs.readdirSync(dispatchOut, {withFileTypes: true}).filter(dir => dir.isDirectory() && dir.name.startsWith('managed-')).length,
    }
    result.artifactCounts = artifactCounts
    const batchFile = path.join(dispatchOut, 'batch-compilation.json')
    const batch = fs.existsSync(batchFile) ? read(batchFile) : null
    const batchMatches = batch ? json(batch) === json(dispatch.compilationBatch) : dispatch.compilationBatch.attempted === false
    const expectedRows = stateRows(baseline.executed), expectedFiles = fileRows(baseline.executed)
    if (mode === 'rollback-native-failure') {
      expectedRows.push(json([target.project, ordinary, target.pool, sentinel, 'fail'])); expectedRows.sort()
      const index = expectedFiles.findIndex(row => JSON.parse(row)[1] === ordinary)
      assert(index >= 0)
      const changed = JSON.parse(expectedFiles[index]); changed[4] = 'fail'; expectedFiles[index] = json(changed); expectedFiles.sort()
    }
    const omitted = native.omittedAsHits.map(row => row.instanceId)
    const expectedHit = ['hit', 'refresh', 'mixed-routing'].includes(mode)
    const managed = rollback ? null : read(path.join(dispatchOut, expectedHit ? candidate.evidenceDirectory : candidate.workerEvidenceDirectory,
      expectedHit ? 'replayed-response.json' : 'response.json'))
    const originalTarget = baseline.executed.find(row => row.instanceId === target.instanceId)
    assert(originalTarget && originalTarget.rows.length === 47)
    const identityFile = path.join(dispatchOut, candidate.evidenceDirectory, 'execution-identity.json')
    const identity = fs.existsSync(identityFile) ? read(identityFile) : null
    const expectedProfile = resolveMatrixCandidateProfile(profile)
    check(common[0], dispatch.selectedScope.kind === 'complete-original-default-selection' && dispatch.selectedScope.files === null &&
      dispatch.selectedInstances === 120 && dispatch.candidateInstances === 1 && dispatch.initialOrdinaryInstances === 119 &&
      candidates.length === 1 && candidate.instanceId === target.instanceId &&
      ['project', 'file', 'pool'].every(key => candidate[key] === target[key]))
    check(common[1], dispatch.candidatePolicy === profile && json(dispatch.candidateProfile) === json(expectedProfile) &&
      json(dispatch.supervisorPolicy.candidateProfile) === json(expectedProfile) &&
      dispatch.supervisorPolicy.candidateProfileSelectorSha256 === sha(fs.readFileSync(path.join(path.dirname(self), 'matrix-candidate-profiles.mjs'))) &&
      (rollback ? identity === null : json(identity.identity.supervisorPolicy) === json(dispatch.supervisorPolicy)))
    check(common[2], json(logicalStates(logical)) === json(expectedRows) && json(logicalFiles(logical)) === json(expectedFiles) &&
      logical.files.length === 120 && logical.rows.length === (mode === 'rollback-native-failure' ? 4132 : 4131) &&
      dispatch.logicalReport.sha256 === sha(fs.readFileSync(path.join(dispatchOut, 'logical-results.json'))) && logical.exitCode === dispatch.exitCode)
    const actualNativeRows = stateRows(native.executed)
    const expectedNativeRows = expectedRows.filter(row => !expectedHit || JSON.parse(row)[1] !== target.file || JSON.parse(row)[0] !== target.project)
    const expectedNativeFiles = expectedFiles.filter(row => !expectedHit || JSON.parse(row)[1] !== target.file || JSON.parse(row)[0] !== target.project)
    check(common[3], native.completed === true && native.matched === true && native.originalSelected.length === 120 &&
      native.executed.length === (expectedHit ? 119 : 120) && dispatch.nativeSpecsExecuted === native.executed.length &&
      json(omitted) === json(expectedHit ? [target.instanceId] : []) &&
      json(actualNativeRows) === json(expectedNativeRows) && json(fileRows(native.executed)) === json(expectedNativeFiles))
    check(common[4], /^lookup(?:,record)?(?:,publish)?$/.test(result.callbacks.map(row => row.operation).join(',')))
    check(common[5], dispatch.replayedInstances === (expectedHit ? 1 : 0) &&
      candidate.decision === (expectedHit ? 'replay' : mode === 'seed' ? 'managed' : 'ordinary') &&
      (rollback || closed(managed) && managed.rows.length === 47 && managed.files.length === 1 &&
        managed.files[0].name === originalTarget.relativeModuleId && managed.files[0].state === originalTarget.state &&
        json(managed.rows.map(row => json([row.name, row.state])).sort()) ===
          json(originalTarget.rows.map(row => json([row.name, row.state])).sort())) &&
      (!expectedHit || !candidate.workerEvidenceDirectory && !workers.length))
    const failure = mode === 'rollback-native-failure'
    check(common[6], dispatch.exitCode === (failure ? 1 : 0) && dispatch.observedNativeFailure === failure &&
      dispatch.stickyManagedFailure === false && (!rollback || result.recorded.length === 0 &&
        !result.callbacks.some(row => row.operation === 'publish') && !fs.existsSync(path.join(dispatchOut, 'receipts'))) &&
      (rollback || result.callbacks.filter(row => row.operation === 'publish').length === 1 &&
        result.callbacks.at(-1).operation === 'publish' && result.callbacks.at(-1).exitCode === 0))
    check(common[7], result.actualSdkAuthenticationClaimed === false && dispatch.fullJobHit === false)
    if (mode === 'seed') {
      check(extra.seed[0], dispatch.compilationBatch.requests === 1 && dispatch.compilationBatch.completed === true &&
        dispatch.compilationBatch.diagnostics.fullFences === 4 && json(dispatch.compilationBatch.diagnostics.trace) === json(oneBatchTrace) &&
        batchMatches && batch !== null && artifactCounts.builds === 1 && artifactCounts.certificates === 1 &&
        artifactCounts.workerDirectories === 1 && workers.length === 1 && dispatch.managedExecutions === 1 &&
        candidate.comparison.equal === true && candidate.comparison.managedCount === 47 && candidate.fileMatches === true)
      check(extra.seed[1], native.executed.length === 120 && result.recorded.length === 1 && dispatch.recordedReceipts === 1 &&
        result.callbacks.at(-1).operation === 'publish' && result.callbacks.at(-1).exitCode === 0)
    } else if (mode === 'hit' || mode === 'mixed-routing') {
      const positive = dispatch.skippedCompilations === 1 && dispatch.managedExecutions === 0 &&
        dispatch.compilationBatch.attempted === false && batch === null && batchMatches &&
        artifactCounts.builds === 0 && artifactCounts.certificates === 0 && artifactCounts.workerDirectories === 0 && result.recorded.length === 0 &&
        result.callbacks.filter(row => row.operation === 'lookup').length === 1
      check(extra[mode][0], positive && (mode !== 'mixed-routing' || authority.length === 2 &&
        !result.callbacks.some(row => row.candidateKey === authority[1].candidateKey)))
    } else if (mode === 'refresh') {
      check(extra.refresh[0], dispatch.compilationBatch.requests === 1 && dispatch.compilationBatch.completed === true &&
        dispatch.compilationBatch.diagnostics.fullFences === 4 && json(dispatch.compilationBatch.diagnostics.trace) === json(oneBatchTrace) &&
        batchMatches && batch !== null && artifactCounts.builds === 1 && artifactCounts.certificates === 1 &&
        artifactCounts.workerDirectories === 0 && dispatch.refreshedCandidates === 1 &&
        dispatch.managedExecutions === 0 && dispatch.skippedCompilations === 0 && result.recorded.length === 1 &&
        candidate.inputMissReason === 'current full source bytes differ')
    } else {
      check(extra[mode][0], dispatch.compilationBatch.requests === 1 && dispatch.compilationBatch.completed === false &&
        batch !== null && batchMatches && Array.isArray(batch.fenceTrace) && batch.fenceTrace.length > 0 && batch.fenceTrace.length < 4 &&
        json(batch.fenceTrace) === json(oneBatchTrace.slice(0, batch.fenceTrace.length)) &&
        artifactCounts.builds === 0 && artifactCounts.certificates === 0 && artifactCounts.workerDirectories === 0 &&
        dispatch.compilationBatch.error.message.includes('only a named AsyncLocalStorage import is supported: node:fs') &&
        dispatch.managedExecutions === 0 && workers.length === 0 && dispatch.admissionDeclined &&
        dispatch.skippedCompilations === 0 && !fs.existsSync(path.join(dispatchOut, candidate.evidenceDirectory, 'replayed-response.json')))
      check(extra[mode][1], native.executed.length === 120 && native.omittedAsHits.length === 0 &&
        native.exitCode === (failure ? 1 : 0) && actualNativeRows.filter(row => JSON.parse(row)[4] === 'fail').length === (failure ? 1 : 0))
      let poisoned
      try {await createCertifiedManagedCompiler({project})} catch (error) {poisoned = error.message}
      check(extra[mode][2], poisoned?.includes('fresh process'))
    }
    assert.deepEqual(result.checks, inventory[mode].map(name => ({name, pass: true})))
    result.passed = true
  } catch (error) {result.error = errorData(error); process.exitCode = 1}
  finally {
    if (ownsSources) sourceFiles.forEach((file, index) => fs.writeFileSync(path.join(project, file), originals[index]))
    result.originalSourcesRestored = sourceFiles.every((file, index) => fs.readFileSync(path.join(project, file)).equals(originals[index]))
    if (!result.originalSourcesRestored) {result.passed = false; process.exitCode = 1}
    result.checkCount = result.checks.length; result.expectedCheckCount = inventory[mode].length
    write(path.join(out, 'result.json'), result)
  }
}

async function controller(projectArg, outArg, baselineArg) {
  assert(projectArg && outArg && baselineArg, 'usage: property47-profile-probe.mjs PROJECT EMPTY_OUT VERIFIED_MATRIX_ROOT')
  const project = fs.realpathSync(projectArg), out = path.resolve(outArg)
  const baselineFile = path.join(fs.realpathSync(baselineArg), 'native-selection.json')
  assert(!fs.existsSync(out)); fs.mkdirSync(out)
  const result = {schema: 'quod-managed-property47-profile-probe-v1-research', passed: false,
    actualSdkAuthenticationClaimed: false, originalDispatcherChecksModified: false, checks: [], cases: [], caseInventories: inventory}
  const check = (name, value) => {result.checks.push({name, pass: Boolean(value)}); assert(value, name)}
  const originals = sourceFiles.map(file => fs.readFileSync(path.join(project, file)))
  let ownsSources = false, seed
  try {
    check('reviewed original source bytes exist before any child', originals.every((bytes, index) => sha(bytes) === originalSHA[index]))
    ownsSources = true
    const defaultProfile = resolveMatrixCandidateProfile()
    check('default108 profile label and exact roster remain unchanged', defaultProfile.label === MATRIX_CANDIDATE_POLICY &&
      defaultProfile.roster === MATRIX_CANDIDATE_ROSTER && defaultProfile.roster.length === 108)
    check('property profile has only the independently reviewed exact tuple', json(resolveMatrixCandidateProfile(profile).roster) === json([target]))
    for (const value of invalidProfiles) {
      let rejected = false
      try {resolveMatrixCandidateProfile(value)} catch {rejected = true}
      check(invalidProfileName(value), rejected)
    }
    check('missing target leaves every other actual instance ordinary', selectMatrixProfileCandidates(MATRIX_CANDIDATE_ROSTER.filter(
      row => row.instanceId !== target.instanceId), profile).length === 0)
    for (const key of ['project', 'file', 'pool']) {
      let rejected = false
      try {selectMatrixProfileCandidates([{...target, [key]: 'wrong'}], profile)} catch {rejected = true}
      check('same ID cannot substitute property identity: ' + key, rejected)
    }
    let duplicate = false
    try {selectMatrixProfileCandidates([target, target], profile)} catch {duplicate = true}
    check('duplicate property instance rejected', duplicate)
    for (const mode of modes) {
      const directory = path.join(out, mode), request = path.join(out, mode + '-request.json')
      const authority = seed ? [...seed.recorded] : []
      if (mode === 'mixed-routing') authority.push({...seed.recorded[0], candidateKey: '0'.repeat(64), historicalRouteOnly: true})
      write(request, {project, out: directory, baselineFile, mode, authority})
      const started = performance.now(), actual = spawnSync(process.execPath, [self, '--property47-profile-child', request],
        {cwd: project, env: process.env, timeout: 480000, killSignal: 'SIGKILL', maxBuffer: 8 * 1024 * 1024})
      fs.writeFileSync(path.join(out, mode + '-stdout.log'), actual.stdout || Buffer.alloc(0), {flag: 'wx'})
      fs.writeFileSync(path.join(out, mode + '-stderr.log'), actual.stderr || Buffer.alloc(0), {flag: 'wx'})
      const value = fs.existsSync(path.join(directory, 'result.json')) ? read(path.join(directory, 'result.json')) : null
      result.cases.push({mode, status: actual.status, signal: actual.signal, seconds: (performance.now() - started) / 1000,
        error: actual.error ? errorData(actual.error) : null, passed: value?.passed === true})
      check(mode + ': all exact fresh child controls pass', !actual.error && actual.status === 0 && actual.signal === null &&
        value?.passed === true && value.originalSourcesRestored === true &&
        json(value.checks) === json(inventory[mode].map(name => ({name, pass: true}))))
      if (mode === 'seed') {seed = value; check('seed retains exactly one synthetic conformed candidate', seed.recorded.length === 1)}
      check(mode + ': original source bytes restored before next process', sourceFiles.every((file, index) =>
        sha(fs.readFileSync(path.join(project, file))) === originalSHA[index]))
    }
    assert.deepEqual(result.cases.map(row => row.mode), modes)
    assert.deepEqual(result.checks, parentInventory.map(name => ({name, pass: true})))
    result.passed = true
  } catch (error) {result.error = errorData(error); process.exitCode = 1}
  finally {
    if (ownsSources) sourceFiles.forEach((file, index) => fs.writeFileSync(path.join(project, file), originals[index]))
    result.originalSourcesRestored = sourceFiles.every((file, index) => fs.readFileSync(path.join(project, file)).equals(originals[index]))
    if (!result.originalSourcesRestored) {result.passed = false; process.exitCode = 1}
    result.caseCount = result.cases.length; result.checkCount = result.checks.length
    result.expectedCheckCount = parentInventory.length; result.parentInventory = parentInventory
    write(path.join(out, 'result.json'), result)
  }
}

if (process.argv[2] === '--property47-profile-child') await child(process.argv[3])
else await controller(...process.argv.slice(2))
