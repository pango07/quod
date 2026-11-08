// Remote-only finite scheduling/ownership diagnostic. Cached bytes in this
// probe have synthetic SHA authority; there is no SDK or performance claim.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'
import {createCertifiedManagedCompiler, canonicalManagedJSON as json} from './resolved-compile-certificate.mjs'
import {PROPERTY47_CANDIDATE_PROFILE as profile, PROPERTY47_CANDIDATE as target} from './matrix-candidate-profiles.mjs'
import {MATRIX_CANDIDATE_POLICY} from './matrix-candidates.mjs'
import {MANAGED_BOUNDARY_CONTRACT, MANAGED_INTRINSIC_POLICY} from './receipt.mjs'

assert.equal(process.env.GITHUB_ACTIONS, 'true', 'remote-only owned property overlap diagnostic')
const self = fileURLToPath(import.meta.url), here = path.dirname(self)
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const write = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', {flag: 'wx'})
const errorData = error => ({name: String(error?.name || 'Error'), message: String(error?.message || error), stack: String(error?.stack || '')})
const originalSha = '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75'
const modes = ['serial-control', 'cold', 'hit', 'refresh', 'decline-before-start', 'after-start-write-error', 'managed-failure', 'native-failure', 'managed-failure-write-error']
const trace = ['before-constructor', 'after-constructor/before-compile:0',
  'after-compile:0/before-capture:0', 'after-capture:0/before-publication']
const matrixTrace = ['before-constructor', 'after-constructor/before-compile:0', ...Array.from({length: 108}, (_, index) => [
  'after-compile:' + index + '/before-capture:' + index,
  index === 107 ? 'after-capture:107/before-publication' : 'after-capture:' + index + '/before-compile:' + (index + 1),
]).flat()]
const sentinel = 'QUOD owned overlap actual assertion sentinel'
const stateRows = files => files.flatMap(file => file.rows.map(row => json([file.project, file.file, file.pool, row.name, row.state]))).sort()
const fileRows = files => files.map(file => json([file.project, file.file, file.pool, file.relativeModuleId, file.state])).sort()
const logicalRows = value => value.rows.map(row => json([row.project, row.file, row.pool, row.name, row.state])).sort()
const logicalFiles = value => value.files.map(row => json([row.project, row.file, row.pool, row.name, row.state])).sort()
const common = [
  'complete original120 specifications retain only the exact property47 candidate',
  'one exact native batch supplies every actual nonhit and preserves original states',
  'all native subprocess, streams and observed descendants settle before publication or rejection',
  'whole-command failure remains sticky and never records a receipt',
  'source bytes are restored after the disposable case',
]
const extras = {
  'serial-control': ['default serial scheduling remains opt-out with full cold conformance'],
  cold: ['all four batch fences and execution policy precede the single native reservation',
    'actual native and inherited property worker lifetimes intersect with independent socket ownership',
    'one fully compared47-state result records only after both owners have settled'],
  hit: ['fresh authenticated input proof skips all candidate compilation and workers before native119'],
  refresh: ['stale source proof compiles behind four fences then refreshes without a worker or cold overlap'],
  'decline-before-start': ['real unsupported source declines before reservation and full native120 fallback executes once'],
  'after-start-write-error': ['two real result-write errors escape admission yet finally join the one reserved native batch'],
  'managed-failure': ['sealed actual managed assertion failure remains exit1 despite a passing native counterpart'],
  'native-failure': ['actual native assertion failure remains exit1 despite a passing managed counterpart'],
  'managed-failure-write-error': ['known sealed managed failure survives actual after-response result-write failure and passing native fallback'],
}
const inventories = Object.fromEntries(modes.map(mode => [mode, [...common, ...extras[mode]]]))
const matrixModes = ['matrix-cold', 'matrix-hit']
const matrixCommon = [
  'closed default108 roster preserves every original120 specification and never claims a full-job hit',
  'all original4131 project/name/states and API file states match the actual native baseline',
  'one exact native batch runs full120 cold or only the12 actual warm nonhits',
  'lookup precedes execution and no receipt or publication precedes every active owner cleanup',
  'every current candidate response is sealed and preserves its matching native names and file state',
]
const matrixInventories = {
  'matrix-cold': [...matrixCommon,
    'all108 builds and execution keys cross218 full fences before either native or pool reservation',
    'actual verified worker lifetimes overlap within fixed4 and coexist with the actual native process',
    'all108 current managed comparisons pass before exactly108 receipt records'],
  'matrix-hit': [...matrixCommon,
    'all108 current authenticated proofs skip candidate compilation and workers before native12',
    'replay materialization precedes native reservation and does not start a managed pool'],
}

function closed(data) {
  const b = data?.boundary, c = b?.completion
  return data?.ok === true && b?.contract === MANAGED_BOUNDARY_CONTRACT && b.intrinsicPolicy === MANAGED_INTRINSIC_POLICY &&
    c?.sealed === true && c.intrinsicLockdown === true && c.reason === '' && b.pendingTimers === 0 &&
    ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'].every(key => c[key] === 0) &&
    ['unsupported', 'unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors'].every(key => Array.isArray(b[key]) && b[key].length === 0)
}
function peak(intervals) {
  const events = intervals.flatMap(([start, end]) => start < end ? [[start, 1], [end, -1]] : [])
    .sort((a, b) => a[0] - b[0] || a[1] - b[1])
  let active = 0, maximum = 0
  for (const [, delta] of events) {active += delta; maximum = Math.max(maximum, active)}
  assert.equal(active, 0); return maximum
}

async function matrixChild(requestFile) {
  const {project, out, baselineFile, mode, authority} = read(requestFile)
  assert(matrixModes.includes(mode)); assert(!fs.existsSync(out)); fs.mkdirSync(out)
  const result = {schema: 'quod-matrix108-owned-overlap-case-v1-research', mode, passed: false,
    actualSdkAuthenticationClaimed: false, checks: [], callbacks: [], recorded: []}
  const check = (name, pass) => {result.checks.push({name, pass: Boolean(pass)}); assert(pass, name)}
  try {
    assert.equal(sha(fs.readFileSync(path.join(project, target.file))), originalSha)
    process.env.TEST = 'true'; process.env.VITEST = 'true'; process.env.NODE_ENV ??= 'test'
    await createCertifiedManagedCompiler({project})
    const {dispatchManagedInputMatrix} = await import('./matrix-input-dispatcher.mjs')
    const dispatchOut = path.join(out, 'dispatch'), byKey = new Map((authority || []).map(row => [row.candidateKey, row]))
    const dispatch = await dispatchManagedInputMatrix({project, out: dispatchOut, coldOverlap: true,
      async lookup({candidateKey}) {
        assert(!fs.existsSync(path.join(dispatchOut, 'native-once.json')) && !fs.existsSync(path.join(dispatchOut, 'managed-pool.json')))
        result.callbacks.push({operation: 'lookup', candidateKey, monoMs: performance.now()})
        const entry = byKey.get(candidateKey)
        if (!entry) return null
        const receiptBytes = fs.readFileSync(entry.file); assert.equal(sha(receiptBytes), entry.envelopeSha256)
        return {receiptBytes, trustedReceiptSha256: entry.envelopeSha256}
      },
      async record(entry) {
        const owned = read(path.join(dispatchOut, 'native/ownership.json'))
        const pool = read(path.join(dispatchOut, 'managed-pool.json'))
        assert(owned.cleanupCompleted === true && pool.completed === true && pool.active === 0)
        const bytes = fs.readFileSync(entry.file); assert.equal(sha(bytes), entry.envelopeSha256)
        result.callbacks.push({operation: 'record', candidateKey: entry.candidateKey, monoMs: performance.now()})
        result.recorded.push({candidateKey: entry.candidateKey, envelopeSha256: entry.envelopeSha256, file: entry.file})
      },
      async publish({exitCode}) {
        const owned = read(path.join(dispatchOut, 'native/ownership.json')); assert(owned.cleanupCompleted === true)
        result.callbacks.push({operation: 'publish', exitCode, monoMs: performance.now()})
        return {saved: false, reason: 'synthetic-matrix-overlap-diagnostic-no-SDK'}
      }})
    result.dispatch = dispatch
    const warm = mode === 'matrix-hit', baseline = read(baselineFile)
    const native = read(path.join(dispatchOut, 'native/native-selection.json'))
    const owner = read(path.join(dispatchOut, 'native/ownership.json')), once = read(path.join(dispatchOut, 'native-once.json'))
    const pool = fs.existsSync(path.join(dispatchOut, 'managed-pool.json')) ? read(path.join(dispatchOut, 'managed-pool.json')) : null
    const logical = read(path.join(dispatchOut, 'logical-results.json')), candidates = dispatch.instances.filter(row => row.candidate)
    const nativeById = new Map((warm ? baseline : native).executed.map(row => [row.instanceId, row]))
    const hits = new Set(candidates.map(row => row.instanceId)), workers = []
    let allResponsesEqual = true, allKeyed = true, builds = 0, certificates = 0
    for (const row of candidates) {
      const directory = path.join(dispatchOut, row.evidenceDirectory), identity = read(path.join(directory, 'execution-identity.json'))
      const actual = read(path.join(warm ? directory : path.join(dispatchOut, row.workerEvidenceDirectory), warm ? 'replayed-response.json' : 'response.json'))
      const counterpart = nativeById.get(row.instanceId)
      allResponsesEqual &&= closed(actual) && actual.files.length === 1 && counterpart &&
        actual.files[0].name === counterpart.relativeModuleId && actual.files[0].state === counterpart.state &&
        json(actual.rows.map(test => json([test.name, test.state])).sort()) === json(counterpart.rows.map(test => json([test.name, test.state])).sort())
      const supervisor = identity.identity.supervisorPolicy, runtime = identity.identity.runtimePolicy
      allKeyed &&= identity.key === row.executionKey && supervisor.overlapPolicy.coldConcurrency === 4 &&
        supervisor.overlapPolicy.managedPoolSha256 === sha(fs.readFileSync(path.join(here, 'managed-task-pool.mjs'))) &&
        runtime.supervisor.listenerProtocol.protocol === 'inherited-linux-fd3-control4-v1-research' &&
        identity.identity.nativePolicy.adapterSha256 === sha(fs.readFileSync(path.join(here, 'owned-native-adapter.mjs')))
      builds += Number(fs.existsSync(path.join(directory, 'test-build.json')))
      certificates += Number(fs.existsSync(path.join(directory, 'compile-certificate.json')))
      if (!warm) {
        const listener = read(path.join(dispatchOut, row.workerEvidenceDirectory, 'listener.json'))
        workers.push({instanceId: row.instanceId, ...listener})
      }
    }
    const ordinary = baseline.executed.filter(row => !warm || !hits.has(row.instanceId))
    check(matrixCommon[0], dispatch.candidatePolicy === MATRIX_CANDIDATE_POLICY && dispatch.selectedInstances === 120 &&
      candidates.length === dispatch.candidateInstances && candidates.length === 108 && dispatch.initialOrdinaryInstances === 12 &&
      dispatch.selectedScope.kind === 'complete-original-default-selection' && dispatch.selectedScope.files === null && dispatch.fullJobHit === false)
    check(matrixCommon[1], logical.files.length === 120 && logical.rows.length === 4131 && json(logicalRows(logical)) === json(stateRows(baseline.executed)) &&
      json(logicalFiles(logical)) === json(fileRows(baseline.executed)) && dispatch.logicalReport.sha256 === sha(fs.readFileSync(path.join(dispatchOut, 'logical-results.json'))))
    check(matrixCommon[2], once.starts === 1 && once.state === 'settled' && native.originalSelected.length === 120 && native.completed === true && native.matched === true &&
      native.executed.length === (warm ? 12 : 120) && dispatch.nativeSpecsExecuted === native.executed.length &&
      json(stateRows(native.executed)) === json(stateRows(ordinary)) && json(fileRows(native.executed)) === json(fileRows(ordinary)))
    const callbacks = result.callbacks, firstOther = callbacks.findIndex(row => row.operation !== 'lookup')
    check(matrixCommon[3], callbacks.filter(row => row.operation === 'lookup').length === 108 && firstOther >= 108 &&
      owner.cleanupCompleted === true && owner.ownershipVerified === true &&
      callbacks.filter(row => row.operation !== 'lookup').every(row => row.monoMs >= owner.joinedMonoMs && (!pool || row.monoMs >= pool.settledMonoMs)) &&
      workers.every(worker => worker.cleanupCompleted === true && worker.parentClosed === true && worker.parentAccepted === 0) &&
      dispatch.exitCode === 0 && dispatch.stickyManagedFailure === false && dispatch.observedNativeFailure === false && callbacks.at(-1).exitCode === 0)
    check(matrixCommon[4], allResponsesEqual && allKeyed && candidates.every(row => row.decision === (warm ? 'replay' : 'managed')))
    if (!warm) {
      const batch = read(path.join(dispatchOut, 'batch-compilation.json'))
      check(matrixInventories[mode][5], batch.completed === true && batch.requests === 108 && batch.diagnostics.fullFences === 218 &&
        json(batch.diagnostics.trace) === json(matrixTrace) && json(batch) === json(dispatch.compilationBatch) && builds === 108 && certificates === 108 &&
        once.reservedMonoMs >= dispatch.coldOverlap.coldReadyMonoMs && pool.startedMonoMs >= dispatch.coldOverlap.coldReadyMonoMs &&
        json(pool.starts) === json(candidates.map(row => row.instanceId)) && pool.unstarted.length === 0)
      const verified = workers.every(worker => worker.ownershipVerified === true && worker.childOwnership.pid > 0 &&
        worker.ownershipMonoMs <= worker.runMonoMs && worker.runMonoMs < worker.shutdownMonoMs && worker.shutdownMonoMs <= worker.closedMonoMs)
      const managedPeak = peak(workers.map(worker => [worker.ownershipMonoMs, worker.shutdownMonoMs]))
      const managedPeakDuringNative = peak(workers.map(worker => [Math.max(owner.spawnEventMonoMs, worker.ownershipMonoMs), Math.min(owner.rootExitMonoMs, worker.shutdownMonoMs)]))
      result.actualIntervals = {nativeSpawnEventMonoMs: owner.spawnEventMonoMs, nativeRootExitMonoMs: owner.rootExitMonoMs,
        managedPeak, managedPeakDuringNative, workers: workers.map(worker => ({instanceId: worker.instanceId,
          pid: worker.childOwnership.pid, ownershipMonoMs: worker.ownershipMonoMs, runMonoMs: worker.runMonoMs,
          shutdownMonoMs: worker.shutdownMonoMs, closedMonoMs: worker.closedMonoMs}))}
      check(matrixInventories[mode][6], verified && owner.pid > 0 && owner.spawnEventMonoMs !== null && owner.rootExitMonoMs !== null &&
        pool.maxConcurrency === 4 && pool.maxActive >= 2 && pool.maxActive <= 4 && pool.active === 0 && pool.completed === true &&
        managedPeak >= 2 && managedPeak <= 4 && managedPeakDuringNative >= 2 &&
        workers.some(worker => Math.max(owner.spawnEventMonoMs, worker.ownershipMonoMs) < Math.min(owner.rootExitMonoMs, worker.runMonoMs)))
      check(matrixInventories[mode][7], dispatch.managedExecutions === 108 && workers.length === 108 && dispatch.replayedInstances === 0 &&
        candidates.every(row => row.comparison.equal === true && row.fileMatches === true) && result.recorded.length === 108 && dispatch.recordedReceipts === 108)
    } else {
      check(matrixInventories[mode][5], dispatch.replayedInstances === 108 && dispatch.skippedCompilations === 108 &&
        dispatch.managedExecutions === 0 && dispatch.compilationBatch.attempted === false && builds === 0 && certificates === 0 &&
        !fs.readdirSync(dispatchOut).some(name => name.startsWith('managed-')) && !fs.existsSync(path.join(dispatchOut, 'batch-compilation.json')))
      check(matrixInventories[mode][6], !pool && dispatch.coldOverlap.startedBeforeCandidate === false &&
        candidates.every(row => row.replayMaterializedMonoMs <= once.reservedMonoMs) && result.recorded.length === 0)
    }
    assert.deepEqual(result.checks, matrixInventories[mode].map(name => ({name, pass: true}))); result.passed = true
  } catch (error) {result.error = errorData(error); process.exitCode = 1}
  finally {
    result.checkCount = result.checks.length; result.expectedCheckCount = matrixInventories[mode].length
    write(path.join(out, 'result.json'), result)
  }
}

async function child(requestFile) {
  const {project, out, baselineFile, mode, authority} = read(requestFile)
  assert(modes.includes(mode)); assert(!fs.existsSync(out)); fs.mkdirSync(out)
  const result = {schema: 'quod-property47-owned-overlap-case-v1-research', mode, passed: false,
    actualSdkAuthenticationClaimed: false, checks: [], callbacks: [], recorded: []}
  const check = (name, pass) => {result.checks.push({name, pass: Boolean(pass)}); assert(pass, name)}
  const source = path.join(project, target.file), original = fs.readFileSync(source)
  assert.equal(sha(original), originalSha)
  const dispatchOut = path.join(out, 'dispatch'), actualWrite = fs.writeFileSync
  let dispatch, escaped, writeErrors = 0
  try {
    if (mode === 'decline-before-start') fs.appendFileSync(source,
      '\nimport {readFileSync as quodOverlapRead} from "node:fs";\n' +
      'if (typeof quodOverlapRead !== "function") throw new Error("native import unavailable");\n')
    if (mode === 'managed-failure' || mode === 'native-failure' || mode === 'managed-failure-write-error') fs.appendFileSync(source,
      '\nimport {test as quodOverlapTest,expect as quodOverlapExpect} from "vitest";\n' +
      'quodOverlapTest(' + JSON.stringify(sentinel) + ',()=>quodOverlapExpect(' +
      'globalThis.__vitest_worker__?.environment?.name === "quod-managed").toBe(' +
      (mode === 'native-failure' ? 'true' : 'false') + '));\n')
    fs.writeFileSync(path.join(out, 'current-property.test.ts'), fs.readFileSync(source), {flag: 'wx'})
    process.env.TEST = 'true'; process.env.VITEST = 'true'; process.env.NODE_ENV ??= 'test'
    await createCertifiedManagedCompiler({project})
    const {dispatchManagedInputMatrix} = await import('./matrix-input-dispatcher.mjs')
    const {decodeMatrixCandidateEnvelope, encodeMatrixCandidateEnvelope} = await import('./matrix-candidate-envelope.mjs')
    let modified
    if (mode === 'refresh') {
      assert.equal(authority.length, 1)
      const entry = authority[0], envelopeBytes = fs.readFileSync(entry.file)
      const decoded = decodeMatrixCandidateEnvelope({candidateKey: entry.candidateKey, envelopeBytes, trustedEnvelopeSha256: entry.envelopeSha256})
      const certificate = JSON.parse(decoded.certificateBytes); certificate.sourceFiles[0].rawSha256 = '0'.repeat(64)
      const encoded = encodeMatrixCandidateEnvelope({candidateKey: entry.candidateKey,
        certificateBytes: Buffer.from(json(certificate) + '\n'), receiptBytes: decoded.receiptBytes})
      modified = {receiptBytes: encoded.envelopeBytes, trustedReceiptSha256: encoded.envelopeSha256}
      fs.writeFileSync(path.join(out, 'synthetic-stale-envelope.json'), encoded.envelopeBytes, {flag: 'wx'})
    }
    if (mode === 'after-start-write-error') fs.writeFileSync = function(file, ...args) {
      if (path.resolve(String(file)) === path.join(dispatchOut, 'result.json') && writeErrors < 2 &&
        fs.existsSync(path.join(dispatchOut, 'native-once.json')) && read(path.join(dispatchOut, 'native-once.json')).starts === 1) {
        writeErrors++; throw new Error('QUOD actual after-start result write failure')
      }
      return actualWrite.call(fs, file, ...args)
    }
    if (mode === 'managed-failure-write-error') fs.writeFileSync = function(file, ...args) {
      const response = path.join(dispatchOut, 'managed-' + target.instanceId, 'response.json')
      if (path.resolve(String(file)) === path.join(dispatchOut, 'result.json') && writeErrors === 0 &&
        fs.existsSync(response) && read(response).ok === false) {
        writeErrors++; throw new Error('QUOD actual result write failure after sealed managed assertion failure')
      }
      return actualWrite.call(fs, file, ...args)
    }
    try {
      dispatch = await dispatchManagedInputMatrix({project, out: dispatchOut, candidateProfile: profile,
        control: mode === 'serial-control', coldOverlap: mode !== 'serial-control',
        async lookup({candidateKey}) {
          result.callbacks.push({operation: 'lookup', candidateKey, monoMs: performance.now()})
          const entry = (authority || []).find(row => row.candidateKey === candidateKey)
          if (!entry) return null
          if (modified) return modified
          const receiptBytes = fs.readFileSync(entry.file); assert.equal(sha(receiptBytes), entry.envelopeSha256)
          return {receiptBytes, trustedReceiptSha256: entry.envelopeSha256}
        },
        async record(entry) {
          const native = read(path.join(dispatchOut, 'native/native-selection.json'))
          const owned = read(path.join(dispatchOut, 'native/ownership.json'))
          assert(native.completed === true && native.exitCode === 0 && owned.cleanupCompleted === true)
          const bytes = fs.readFileSync(entry.file); assert.equal(sha(bytes), entry.envelopeSha256)
          result.callbacks.push({operation: 'record', monoMs: performance.now()})
          result.recorded.push({candidateKey: entry.candidateKey, envelopeSha256: entry.envelopeSha256, file: entry.file})
        },
        async publish({exitCode}) {
          const owned = read(path.join(dispatchOut, 'native/ownership.json')); assert(owned.cleanupCompleted === true)
          result.callbacks.push({operation: 'publish', exitCode, monoMs: performance.now()})
          return {saved: false, reason: 'synthetic-overlap-diagnostic-no-SDK'}
        }})
    } catch (error) {escaped = errorData(error)}
    fs.writeFileSync = actualWrite
    result.escaped = escaped || null
    dispatch ||= read(path.join(dispatchOut, 'result.json')); result.dispatch = dispatch
    const native = read(path.join(dispatchOut, 'native/native-selection.json'))
    const baseline = read(baselineFile), candidate = dispatch.instances.find(row => row.candidate)
    const expectedHit = mode === 'hit' || mode === 'refresh', failure = ['managed-failure', 'native-failure', 'managed-failure-write-error'].includes(mode)
    const owner = mode === 'serial-control' ? null : read(path.join(dispatchOut, 'native/ownership.json'))
    const once = mode === 'serial-control' ? null : read(path.join(dispatchOut, 'native-once.json'))
    const workerListener = mode !== 'serial-control' && candidate.workerEvidenceDirectory
      ? read(path.join(dispatchOut, candidate.workerEvidenceDirectory, 'listener.json')) : null
    const files = fs.readdirSync(path.join(dispatchOut, 'instances', target.instanceId))
    const workers = fs.readdirSync(dispatchOut).filter(name => name.startsWith('managed-'))
    const expectedRows = stateRows(baseline.executed), expectedFiles = fileRows(baseline.executed)
    if (failure) {
      expectedRows.push(json([target.project, target.file, target.pool, sentinel, mode === 'native-failure' ? 'fail' : 'pass'])); expectedRows.sort()
      if (mode === 'native-failure') {
        const index = expectedFiles.findIndex(row => JSON.parse(row)[1] === target.file && JSON.parse(row)[0] === target.project)
        const changed = JSON.parse(expectedFiles[index]); changed[4] = 'fail'; expectedFiles[index] = json(changed); expectedFiles.sort()
      }
    }
    check(common[0], dispatch.selectedScope.kind === 'complete-original-default-selection' && dispatch.selectedScope.files === null &&
      dispatch.selectedInstances === 120 && dispatch.candidateInstances === 1 && candidate.instanceId === target.instanceId &&
      ['project', 'file', 'pool'].every(key => candidate[key] === target[key]))
    const expectedNativeRows = expectedRows.filter(row => !expectedHit || JSON.parse(row)[1] !== target.file || JSON.parse(row)[0] !== target.project)
    const expectedNativeFiles = expectedFiles.filter(row => !expectedHit || JSON.parse(row)[1] !== target.file || JSON.parse(row)[0] !== target.project)
    const logical = fs.existsSync(path.join(dispatchOut, 'logical-results.json')) ? read(path.join(dispatchOut, 'logical-results.json')) : null
    check(common[1], native.completed === true && native.originalSelected.length === 120 &&
      native.executed.length === (expectedHit ? 119 : 120) && json(stateRows(native.executed)) === json(expectedNativeRows) &&
      json(fileRows(native.executed)) === json(expectedNativeFiles) &&
      (mode === 'after-start-write-error' ? logical === null : logical.files.length === 120 &&
        json(logicalRows(logical)) === json(expectedRows) && json(logicalFiles(logical)) === json(expectedFiles)))
    const ownedClosed = !owner || owner.cleanupCompleted === true && owner.ownershipVerified === true &&
      owner.groupSettledMonoMs <= owner.joinedMonoMs && owner.streamsClosedMonoMs <= owner.joinedMonoMs &&
      once.starts === 1 && once.state === 'settled'
    check(common[2], ownedClosed && (!workerListener || workerListener.cleanupCompleted === true && workerListener.parentClosed === true && workerListener.parentAccepted === 0) &&
      result.callbacks.filter(row => ['record', 'publish'].includes(row.operation)).every(row => row.monoMs >= owner.joinedMonoMs &&
        (candidate.workerSettledMonoMs === undefined || row.monoMs >= candidate.workerSettledMonoMs)) &&
      (mode !== 'after-start-write-error' || escaped?.message.includes('after-start result write failure')))
    check(common[3], dispatch.exitCode === (failure || escaped ? 1 : 0) &&
      (!failure && !escaped || result.recorded.length === 0 && !fs.existsSync(path.join(dispatchOut, 'receipts'))) &&
      dispatch.stickyManagedFailure === failure &&
      (escaped || dispatch.observedNativeFailure === (mode === 'native-failure')) && dispatch.fullJobHit === false)
    // Written after source restoration below; still retain this exact case
    // assertion in the fixed inventory rather than changing original tests.
    fs.writeFileSync(source, original)
    check(common[4], sha(fs.readFileSync(source)) === originalSha)
    const batch = fs.existsSync(path.join(dispatchOut, 'batch-compilation.json')) ? read(path.join(dispatchOut, 'batch-compilation.json')) : null
    if (mode === 'serial-control') {
      check(extras[mode][0], !dispatch.coldOverlap && native.executed.length === 120 && dispatch.managedExecutions === 1 && !escaped && !owner &&
        !fs.existsSync(path.join(dispatchOut, candidate.workerEvidenceDirectory, 'listener.json')))
    } else if (mode === 'cold') {
      const worker = read(path.join(dispatchOut, candidate.workerEvidenceDirectory, 'listener.json'))
      const identity = read(path.join(dispatchOut, candidate.evidenceDirectory, 'execution-identity.json'))
      check(extras.cold[0], batch.completed === true && batch.diagnostics.fullFences === 4 && json(batch.diagnostics.trace) === json(trace) &&
        json(batch) === json(dispatch.compilationBatch) && once.requestSha256 === sha(fs.readFileSync(path.join(dispatchOut, 'native-once-request.json'))) &&
        once.reservedMonoMs >= dispatch.coldOverlap.coldReadyMonoMs &&
        identity.key === candidate.executionKey && identity.identity.supervisorPolicy.overlapPolicy.ownedAdapterSha256 ===
          sha(fs.readFileSync(path.join(here, 'owned-native-adapter.mjs'))))
      check(extras.cold[1], dispatch.coldOverlap.startedBeforeCandidate === true && worker.ownershipVerified === true &&
        worker.parentClosed === true && worker.parentAccepted === 0 && worker.cleanupCompleted === true &&
        owner.pid > 0 && owner.spawnEventMonoMs !== null && owner.rootExitMonoMs !== null &&
        Math.max(owner.spawnEventMonoMs, worker.ownershipMonoMs) < Math.min(owner.rootExitMonoMs, worker.runMonoMs))
      check(extras.cold[2], candidate.comparison.equal === true && candidate.comparison.managedCount === 47 && candidate.fileMatches === true &&
        result.recorded.length === 1 && result.callbacks.find(row => row.operation === 'record').monoMs >= candidate.workerSettledMonoMs)
    } else if (mode === 'hit' || mode === 'refresh') {
      check(extras[mode][0], dispatch.replayedInstances === 1 && dispatch.managedExecutions === 0 && workers.length === 0 &&
        dispatch.coldOverlap.startedBeforeCandidate === false && !files.includes('worker.mjs') && native.executed.length === 119 &&
        once.reservedMonoMs >= candidate.replayMaterializedMonoMs &&
        (mode === 'hit' ? !batch && !files.includes('test-build.json') && !files.includes('compile-certificate.json') && dispatch.skippedCompilations === 1
          : batch.completed === true && batch.diagnostics.fullFences === 4 && json(batch.diagnostics.trace) === json(trace) &&
            dispatch.refreshedCandidates === 1 && result.recorded.length === 1))
    } else if (mode === 'decline-before-start') {
      check(extras[mode][0], dispatch.admissionDeclined && !dispatch.coldOverlap.startedBeforeCandidate && batch.completed === false &&
        batch.error.message.includes('only a named AsyncLocalStorage import is supported: node:fs') &&
        workers.length === 0 && result.recorded.length === 0 && !result.callbacks.some(row => row.operation === 'publish') && once.starts === 1)
    } else if (mode === 'after-start-write-error') {
      check(extras[mode][0], writeErrors === 2 && escaped && once.starts === 1 && once.state === 'settled' &&
        owner.cleanupCompleted === true && workers.length === 0 && result.recorded.length === 0 &&
        !result.callbacks.some(row => row.operation === 'publish'))
    } else if (mode === 'managed-failure-write-error') {
      const execution = read(path.join(dispatchOut, candidate.workerEvidenceDirectory, 'execution.json'))
      const managed = read(path.join(dispatchOut, candidate.workerEvidenceDirectory, 'response.json'))
      check(extras[mode][0], writeErrors === 1 && !escaped && managed.ok === false && managed.boundary.completion.sealed === true &&
        managed.boundary.unsupported.length === 0 && managed.rows.some(row => row.name === sentinel && row.state === 'fail') &&
        candidate.decision === 'ordinary' && candidate.error.message.includes('after sealed managed assertion failure') &&
        execution.phaseCallbackFailed === true && execution.completed === false && execution.listenerCleanupCompleted === true &&
        native.exitCode === 0 && dispatch.exitCode === 1 && dispatch.stickyManagedFailure === true && dispatch.observedNativeFailure === false &&
        result.recorded.length === 0 && once.starts === 1 && once.state === 'settled' && result.callbacks.at(-1).exitCode === 1)
    } else {
      const managed = read(path.join(dispatchOut, candidate.workerEvidenceDirectory, 'response.json'))
      check(extras[mode][0], dispatch.stickyManagedFailure === true && managed.ok === (mode === 'native-failure') &&
        dispatch.observedNativeFailure === (mode === 'native-failure') && native.exitCode === (mode === 'native-failure' ? 1 : 0) &&
        candidate.comparison.equal === false && result.callbacks.at(-1).exitCode === 1)
    }
    if (owner && candidate.workerEvidenceDirectory) {
      const worker = read(path.join(dispatchOut, candidate.workerEvidenceDirectory, 'listener.json'))
      result.completionOrdering = {nativeJoinedMonoMs: owner.joinedMonoMs, managedClosedMonoMs: worker.closedMonoMs,
        managedSettledMonoMs: candidate.workerSettledMonoMs ?? null,
        observedOrder: Number.isFinite(candidate.workerSettledMonoMs)
          ? owner.joinedMonoMs < candidate.workerSettledMonoMs ? 'native-first' : 'managed-first' : null}
    }
    assert.deepEqual(result.checks, inventories[mode].map(name => ({name, pass: true})))
    result.passed = true
  } catch (error) {result.error = errorData(error); process.exitCode = 1}
  finally {
    fs.writeFileSync = actualWrite; fs.writeFileSync(source, original)
    result.originalSourcesRestored = sha(fs.readFileSync(source)) === originalSha
    if (!result.originalSourcesRestored) {result.passed = false; process.exitCode = 1}
    result.checkCount = result.checks.length; result.expectedCheckCount = inventories[mode].length
    write(path.join(out, 'result.json'), result)
  }
}

async function controller(projectArg, outArg, baselineArg) {
  assert(projectArg && outArg && baselineArg, 'usage: property47-overlap-probe.mjs PROJECT EMPTY_OUT VERIFIED_MATRIX_ROOT')
  const project = fs.realpathSync(projectArg), out = path.resolve(outArg)
  const baselineFile = path.join(fs.realpathSync(baselineArg), 'native-selection.json')
  assert(!fs.existsSync(out)); fs.mkdirSync(out)
  const result = {schema: 'quod-property47-owned-overlap-probe-v1-research', passed: false, actualSdkAuthenticationClaimed: false,
    performanceClaimed: false, checks: [], cases: [], caseInventories: inventories,
    matrixCases: [], matrixCaseInventories: matrixInventories}
  const check = (name, pass) => {result.checks.push({name, pass: Boolean(pass)}); assert(pass, name)}
  let authority = []
  try {
    const baseline = read(baselineFile)
    check('reviewed baseline retains all original120 instances and4131 names/states', baseline.completed === true &&
      baseline.executed.length === 120 && stateRows(baseline.executed).length === 4131 && sha(fs.readFileSync(path.join(project, target.file))) === originalSha)
    for (const mode of modes) {
      const caseOut = path.join(out, mode), request = path.join(out, mode + '-request.json')
      write(request, {project, out: caseOut, baselineFile, mode, authority: ['hit', 'refresh'].includes(mode) ? authority : []})
      const log = fs.openSync(path.join(out, mode + '.log'), 'wx')
      let childRun
      try {childRun = spawnSync(process.execPath, [self, '--overlap-child', request], {cwd: project,
        env: {...process.env, QUOD_SHIM_ACTIVE: '1'}, stdio: ['ignore', log, log], timeout: 600000, killSignal: 'SIGTERM'})}
      finally {fs.closeSync(log)}
      assert(!childRun.error && childRun.status === 0 && childRun.signal === null, mode + ': child failed')
      const actual = read(path.join(caseOut, 'result.json'))
      check(mode + ': exact fresh-process assertion inventory passes', actual.passed === true &&
        json(actual.checks) === json(inventories[mode].map(name => ({name, pass: true}))) && actual.originalSourcesRestored === true)
      result.cases.push({mode, out: caseOut, checkCount: actual.checkCount})
      if (mode === 'cold') {authority = actual.recorded; assert.equal(authority.length, 1)}
    }
    let matrixAuthority = []
    for (const mode of matrixModes) {
      const caseOut = path.join(out, mode), request = path.join(out, mode + '-request.json')
      write(request, {project, out: caseOut, baselineFile, mode, authority: mode === 'matrix-hit' ? matrixAuthority : []})
      const log = fs.openSync(path.join(out, mode + '.log'), 'wx')
      let childRun
      try {childRun = spawnSync(process.execPath, [self, '--matrix-overlap-child', request], {cwd: project,
        env: {...process.env, QUOD_SHIM_ACTIVE: '1'}, stdio: ['ignore', log, log], timeout: 600000, killSignal: 'SIGTERM'})}
      finally {fs.closeSync(log)}
      assert(!childRun.error && childRun.status === 0 && childRun.signal === null, mode + ': child failed')
      const actual = read(path.join(caseOut, 'result.json'))
      check(mode + ': exact full108 cold/warm assertion inventory passes', actual.passed === true &&
        json(actual.checks) === json(matrixInventories[mode].map(name => ({name, pass: true}))))
      result.matrixCases.push({mode, out: caseOut, checkCount: actual.checkCount})
      if (mode === 'matrix-cold') {matrixAuthority = actual.recorded; assert.equal(matrixAuthority.length, 108)}
    }
    // Primitive routing controls never start a worker or native process.
    process.env.TEST = 'true'; process.env.VITEST = 'true'; process.env.NODE_ENV ??= 'test'
    await createCertifiedManagedCompiler({project})
    const {dispatchManagedInputMatrix} = await import('./matrix-input-dispatcher.mjs')
    for (const [index, value] of [null, 1, 'true', {}, new Boolean(true)].entries()) {
      let denied
      try {await dispatchManagedInputMatrix({project, out: path.join(out, 'invalid-' + result.checks.length), coldOverlap: value})}
      catch (error) {denied = error.message}
      check('nonboolean overlap rejected before any dispatch: ' + index + ':' + json(String(value)), denied?.includes('primitive boolean'))
    }
    for (const args of [{candidateProfile: 'unreviewed-overlap-profile'}, {candidateProfile: profile, suiteFiles: [target.file]}]) {
      let denied
      try {await dispatchManagedInputMatrix({project, out: path.join(out, 'invalid-scope-' + result.checks.length), coldOverlap: true, ...args})}
      catch (error) {denied = error.message}
      check('overlap declines a scope outside the full closed candidate profile: ' + json(args), denied?.includes('closed candidate profile'))
    }
    // Owned-process failure controls are defined below only after the public
    // process API has been loaded; they execute no project source.
    const {runOwnedNativeProcess} = await import('./owned-native-adapter.mjs')
    fs.mkdirSync(path.join(out, 'owned-process-controls'))
    const missingInterpreter = path.join(out, 'owned-process-controls', 'missing-interpreter-executable')
    fs.writeFileSync(missingInterpreter, '#!/definitely/missing/quod-interpreter\n', {flag: 'wx', mode: 0o755})
    const processCases = [
      ['spawn-error', null, 5000],
      ['startup-exit', 'process.stderr.write("QUOD_OWNED_STARTUP_FAILURE\\n");process.exitCode=7;', 5000],
      ['watchdog', 'process.on("SIGTERM",()=>{});setInterval(()=>{},1000);', 250],
      ['pipe-overflow', 'process.stdout.write("x".repeat(65536));setInterval(()=>{},1000);', 5000],
      ['lingering-descendant', 'require("node:child_process").spawn(process.execPath,["-e","setInterval(()=>{},1000)"],{stdio:"inherit"}).unref();process.exit(0);', 5000],
    ]
    result.processCases = []
    for (const [name, code, watchdogMs] of processCases) {
      const value = await runOwnedNativeProcess({executable: name === 'spawn-error' ? missingInterpreter : process.execPath,
        args: code === null ? [] : ['-e', code], cwd: project,
        env: {...process.env, QUOD_SHIM_ACTIVE: '1'}, logFile: path.join(out, 'owned-process-controls', name + '.log'),
        watchdogMs, termGraceMs: 1000, killGraceMs: 1000, logBytesMax: 16384})
      write(path.join(out, 'owned-process-controls', name + '.json'), value)
      check(name + ': actual failed process and pipes/descendants are joined', value.exitCode !== 0 &&
        value.ownership.cleanupCompleted === true && value.ownership.joinedMonoMs >= value.ownership.groupSettledMonoMs &&
        value.ownership.finalLive.length === 0 && value.ownership.streams.stdout.closed === true && value.ownership.streams.stderr.closed === true &&
        (name !== 'watchdog' || value.timedOut === true && value.ownership.exit.signal === 'SIGKILL' &&
          ['watchdog', 'term-grace'].every(kind => value.ownership.failures.some(row => row.kind === kind)) &&
          ['SIGTERM', 'SIGKILL'].every(signal => value.ownership.signals.some(row => row.signal === signal && row.delivered === true))) &&
        (name !== 'spawn-error' || value.ownership.pid === null && value.ownership.spawnError.code === 'ENOENT' && value.ownership.closedMonoMs !== null) &&
        (name !== 'startup-exit' || value.ownership.exit.code === 7 && value.ownership.exit.signal === null &&
          fs.readFileSync(value.logFile, 'utf8').includes('QUOD_OWNED_STARTUP_FAILURE')) &&
        (name !== 'pipe-overflow' || value.ownership.failures.some(row => row.kind === 'log-bound') && value.ownership.retainedLogBytes <= 16384) &&
        (name !== 'lingering-descendant' || value.ownership.tracked.length >= 2 && value.ownership.failures.some(row => row.kind === 'descendant-after-root')))
      result.processCases.push({name, exitCode: value.exitCode})
    }
    const {createManagedTaskPool} = await import('./managed-task-pool.mjs')
    const failurePool = createManagedTaskPool({maxConcurrency: 4}), joinedProcesses = []
    const absentFile = path.join(out, 'owned-process-controls', 'absent-pool-read'); assert(!fs.existsSync(absentFile))
    const poolTasks = Array.from({length: 7}, (_, index) => ({id: 'actual-pool-' + index, async run() {
      if (index === 0) {await Promise.resolve(); fs.readFileSync(absentFile)}
      const value = await runOwnedNativeProcess({executable: process.execPath, args: ['-e', 'setTimeout(()=>{},75);'],
        cwd: project, env: {...process.env, QUOD_SHIM_ACTIVE: '1'},
        logFile: path.join(out, 'owned-process-controls', 'pool-' + index + '.log'), watchdogMs: 5000})
      joinedProcesses.push(value); return value
    }}))
    failurePool.start(poolTasks)
    const rejected = await failurePool.join(), snapshot = failurePool.snapshot()
    write(path.join(out, 'owned-process-controls', 'fatal-pool.json'), {status: rejected.status,
      error: errorData(rejected.reason), snapshot, joinedProcesses})
    check('actual filesystem fatal stops dequeue and joins every already-started owned child before rejection',
      rejected.status === 'rejected' && rejected.reason.code === 'ENOENT' && snapshot.completed === true && snapshot.active === 0 &&
      json(snapshot.starts) === json(poolTasks.slice(0, 4).map(task => task.id)) && snapshot.unstarted.length === 3 &&
      joinedProcesses.length === 3 && joinedProcesses.every(value => value.exitCode === 0 && value.ownership.cleanupCompleted === true &&
        value.ownership.joinedMonoMs <= snapshot.settledMonoMs))
    assert.equal(result.cases.length, 9); assert.equal(result.matrixCases.length, 2); assert.equal(result.processCases.length, 5)
    result.passed = true
  } catch (error) {result.error = errorData(error); process.exitCode = 1}
  finally {
    result.checkCount = result.checks.length; result.expectedCheckCount = 25
    write(path.join(out, 'result.json'), result)
  }
}

if (process.argv[2] === '--overlap-child') await child(process.argv[3])
else if (process.argv[2] === '--matrix-overlap-child') await matrixChild(process.argv[3])
else await controller(...process.argv.slice(2))
