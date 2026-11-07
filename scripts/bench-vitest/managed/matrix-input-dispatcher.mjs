// Separate research API: logical project/file/full-name/state fidelity only.
// It does not replace the Action bridge or reconstruct native timing/metadata.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {fileURLToPath} from 'node:url'
import {MATRIX_CANDIDATE_POLICY, selectMatrixCandidates} from './matrix-candidates.mjs'
import {createManagedExecutor} from './executor.mjs'
import {MANAGED_BOUNDARY_CONTRACT, MANAGED_INTRINSIC_POLICY,
  canonicalReceiptJSON as canonicalManagedJSON} from './receipt.mjs'
import {makeMatrixExecutionIdentity, validateMatrixNativeControl, makeMatrixConformance,
  makeMatrixExecutionIdentityFromProof, createMatrixReceipt, lookupMatrixReceipt} from './matrix-receipt.mjs'
import {captureResolvedCompileCertificate, validateResolvedCompileCertificates,
  createCertifiedManagedCompiler} from './resolved-compile-certificate.mjs'
import {makeMatrixCandidateKey, encodeMatrixCandidateEnvelope, decodeMatrixCandidateEnvelope} from './matrix-candidate-envelope.mjs'

const self = fileURLToPath(import.meta.url), here = path.dirname(self)
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n')
const errorData = error => ({name: String(error?.name || 'Error'), message: String(error?.message || error), stack: String(error?.stack || '')})
const moduleKey = row => canonicalManagedJSON([row.project, row.file, row.pool])

function complete(data) {
  const boundary = data?.boundary, completion = boundary?.completion
  return boundary?.contract === MANAGED_BOUNDARY_CONTRACT && boundary.intrinsicPolicy === MANAGED_INTRINSIC_POLICY &&
    completion?.sealed === true && completion.intrinsicLockdown === true && completion.reason === '' &&
    boundary.pendingTimers === 0 && ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'].every(key => completion[key] === 0) &&
    ['unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors'].every(key => Array.isArray(boundary[key]) && boundary[key].length === 0)
}
function supported(data) {
  return !data?.error && complete(data) && Array.isArray(data.boundary.unsupported) && !data.boundary.unsupported.length &&
    Array.isArray(data.rows) && data.rows.every(row => typeof row.name === 'string' && ['pass', 'fail', 'skip', 'todo'].includes(row.state) && Array.isArray(row.errors)) &&
    Array.isArray(data.files) && data.files.length === 1
}

/** Separate unwired input-proof investigation. lookup routes by candidateKey;
 * its exact envelope bytes must be authenticated by the actual SDK pack first.
 * One fresh shared validation can skip compilation; a miss still compiles.
 * The complete current execution ledger independently validates every receipt. */
export async function dispatchManagedInputMatrix({project: projectArg, out: outArg, epoch = 1700000000000, seed = 42,
  env = {NODE_ENV: 'test'}, control = false, suiteFiles, lookup, record, publish} = {}) {
  assert(projectArg && outArg, 'explicit research project/output required')
  assert(Number.isSafeInteger(epoch) && epoch >= 0 && Number.isSafeInteger(seed) && seed >= 0 && seed <= 0xffffffff)
  assert.equal(typeof control, 'boolean')
  for (const callback of [lookup, record, publish]) assert(callback === undefined || typeof callback === 'function')
  canonicalManagedJSON(env)
  assert(env && !Array.isArray(env) && [Object.prototype, null].includes(Object.getPrototypeOf(env)) &&
    Object.values(env).every(value => typeof value === 'string'), 'declared environment must be a flat plain string map')
  const project = fs.realpathSync(path.resolve(projectArg)), out = path.resolve(outArg)
  if (suiteFiles !== undefined) {
    assert(Array.isArray(suiteFiles) && suiteFiles.length > 0 && new Set(suiteFiles).size === suiteFiles.length)
    for (const file of suiteFiles) {
      assert(typeof file === 'string' && file && !path.isAbsolute(file) && !file.includes('\\') && !file.split('/').includes('..'), 'explicit relative selected files required')
      const real = fs.realpathSync(path.resolve(project, file))
      assert(real.startsWith(project + path.sep) && fs.statSync(real).isFile(), 'selected scope must name actual project files')
    }
  }
  fs.mkdirSync(out, {recursive: true}); assert.equal(fs.readdirSync(out).length, 0, 'research dispatch output must be empty')
  process.chdir(project)
  const started = performance.now(), base = {epoch, seed, env}
  const result = {schema: 'quod-managed-matrix-input-dispatch-v1-research', productionEnabled: false, fullJobHit: false,
    reportFidelity: 'logical actual project/file/pool/full-name/state multiset; no native timing or reporter-metadata reconstruction',
    mode: control ? 'control' : 'research', exitCode: 1, instances: [], phases: [],
    selectedScope: suiteFiles ? {kind: 'explicit-file-selection', files: [...suiteFiles]} : {kind: 'complete-original-default-selection', files: null},
    managedExecutions: 0, replayedInstances: 0, recordedReceipts: 0, skippedCompilations: 0,
    refreshedCandidates: 0, candidatePolicy: MATRIX_CANDIDATE_POLICY}
  const save = () => write(path.join(out, 'result.json'), result)
  const contexts = new Map(), dataById = new Map()
  let suite, stickyManagedFailure = false, native, nativePolicy, admissionFailure, observedNativeFailure = false
  let runMatrixNativeBatch, compareManagedMatrixStates, logicalDocument
  save()
  try {
    // This module's static imports use only builtins. Initialize and pin the
    // tool epoch before preparation/planner/native helpers load the compiler.
    // Match preparation's existing public CLI bootstrap before the snapshot.
    process.env.TEST = 'true'
    process.env.VITEST = 'true'
    process.env.NODE_ENV ??= 'test'
    const compile = await createCertifiedManagedCompiler({project})
    const {prepareManagedMatrix} = await import('./preparation.mjs')
    const planner = await import('./matrix-planner.mjs')
    const {createManagedMatrixPlan} = planner
    compareManagedMatrixStates = planner.compareManagedMatrixStates
    ;({runMatrixNativeBatch} = await import('./matrix-native.mjs'))
    suite = await prepareManagedMatrix({project, seed, files: suiteFiles})
    write(path.join(out, 'preparation.json'), suite.evidence)
    const plan = createManagedMatrixPlan(suite)
    write(path.join(out, 'matrix-plan.json'), plan)
    const candidates = new Set(selectMatrixCandidates(suite.selected).map(row => row.instanceId))
    result.selectedInstances = suite.selected.length; result.candidateInstances = candidates.size
    result.initialOrdinaryInstances = suite.selected.length - candidates.size; result.planKey = plan.key
    result.instances = suite.selected.map(row => ({instanceId: row.instanceId, project: row.project, file: row.file,
      pool: row.pool, candidate: candidates.has(row.instanceId), decision: 'ordinary', nativeExecuted: false,
      reason: candidates.has(row.instanceId) ? 'candidate not yet admitted' : 'outside reviewed research candidate selection'}))
    const phaseRows = new Map()
    const onPhase = phase => {
      result.phases.push({name: phase.name, seconds: phase.seconds, responseSha256: phase.responseSha256})
      const row = phaseRows.get(phase.name)
      if (row) {row.workerSeconds = phase.seconds; row.responseSha256 = phase.responseSha256}
      save()
    }
    // Factory policy discovery hashes native executables. Reuse that immutable
    // current-host class; every execute() still starts a fresh independent
    // workerd process and verifies its actual binary bytes again.
    const preflightExecutor = createManagedExecutor({out, watchdogMs: 30000, onPhase})
    let propertyExecutor
    const preflightBuild = await compile('preflight.mjs', base)
    write(path.join(out, 'preflight-build.json'), {...preflightBuild, bundle: undefined, options: base})
    const preflight = await preflightExecutor.execute('preflight', preflightBuild.bundle)
    assert(preflight.ok === true && preflight.checks.length === 31 && new Set(preflight.checks.map(row => row.name)).size === 31 &&
      preflight.checks.every(row => row.pass === true) && complete(preflight), 'private current-host boundary preflight failed')
    const nativeFingerprintSha256 = sha(canonicalManagedJSON({preflightIdentity: preflightBuild.identitySha256,
      inventory: preflight.boundary.inventory, symbols: preflight.boundary.symbolInventory}))
    const adapterSha256 = sha(fs.readFileSync(path.join(here, 'matrix-native.mjs')))
    nativePolicy = {adapterSha256, nodeExecutableSha256: preflightExecutor.runtimePolicy.supervisor.nodeExecutableSha256,
      nodeVersion: process.version, vitestVersion: '5.0.0', host: preflightExecutor.host}
    const supervisorPolicy = {receiptValidatorSha256: sha(fs.readFileSync(path.join(here, 'matrix-receipt.mjs'))),
      experimentSha256: sha(fs.readFileSync(self)), conformanceAdapterSha256: adapterSha256,
      candidateSelectorSha256: sha(fs.readFileSync(path.join(here, 'matrix-candidates.mjs'))),
      inputValidatorSha256: sha(fs.readFileSync(path.join(here, 'resolved-compile-certificate.mjs'))),
      candidateEnvelopeSha256: sha(fs.readFileSync(path.join(here, 'matrix-candidate-envelope.mjs')))}
    result.nativeFingerprintSha256 = nativeFingerprintSha256; result.host = preflightExecutor.host
    result.preflightPassed = true; result.nativePolicy = nativePolicy; result.supervisorPolicy = supervisorPolicy
    const prepared = new Map(suite.prepared.map(row => [row.instanceId, row]))
    const declined = new Map(suite.declined.map(row => [row.instanceId, row]))
    fs.mkdirSync(path.join(out, 'instances'))
    const inputCandidates = [], inputRequests = []
    for (const row of result.instances) {
      if (!row.candidate) continue
      const candidate = prepared.get(row.instanceId)
      if (!candidate) {row.reason = declined.get(row.instanceId)?.reason || 'preparation declined'; continue}
      const directory = path.join(out, 'instances', row.instanceId)
      fs.mkdirSync(directory)
      row.evidenceDirectory = 'instances/' + row.instanceId
      write(path.join(directory, 'preparation.json'), candidate.identity)
      const options = {...base, vitest: candidate.config, file: '/project/' + candidate.file,
        providedContext: candidate.providedContext, ...(candidate.zodAlias ? {zodAlias: candidate.zodAlias} : {})}
      const executor = candidate.file.endsWith('/stream.property.test.ts')
        ? (propertyExecutor ||= createManagedExecutor({out, watchdogMs: 180000, onPhase})) : preflightExecutor
      const policy = {runtimePolicy: {...executor.runtimePolicy, nativeFingerprintSha256},
        supervisorPolicy, host: executor.host, nativePolicy}
      const candidateKey = makeMatrixCandidateKey({prepared: candidate, options, ...policy})
      row.candidateKey = candidateKey
      const context = {row, candidate, options, policy, executor, directory, candidateKey,
        phaseDirectory: path.join(out, 'managed-' + row.instanceId)}
      contexts.set(row.instanceId, context)
      if (!control && lookup) {
        try {
          const cached = await lookup({candidateKey})
          if (cached) {
            const decoded = decodeMatrixCandidateEnvelope({candidateKey,
              envelopeBytes: cached.receiptBytes, trustedEnvelopeSha256: cached.trustedReceiptSha256})
            fs.writeFileSync(path.join(directory, 'candidate-envelope.json'), cached.receiptBytes)
            write(path.join(directory, 'candidate-authentication.json'), {candidateKey,
              trustedEnvelopeSha256: cached.trustedReceiptSha256,
              trustedCertificateSha256: decoded.trustedCertificateSha256, trustedReceiptSha256: decoded.trustedReceiptSha256})
            context.cached = decoded
            inputCandidates.push(context)
            inputRequests.push({certificateBytes: decoded.certificateBytes,
              trustedCertificateSha256: decoded.trustedCertificateSha256, options, testFile: candidate.absolute})
          }
        } catch (error) {row.cacheMissReason = error.message}
      }
    }
    const validationStarted = performance.now()
    const decisions = inputRequests.length ? await validateResolvedCompileCertificates({project, requests: inputRequests}) : []
    assert.equal(decisions.length, inputCandidates.length, 'input proof omitted a requested candidate')
    for (let index = 0; index < inputCandidates.length; index++) {
      const context = inputCandidates[index], decision = decisions[index]
      write(path.join(context.directory, 'input-decision.json'), {...decision, proof: undefined, identity: undefined})
      if (decision.hit) context.proof = decision.proof
      else context.row.inputMissReason = decision.reason
    }
    result.inputValidationSeconds = (performance.now() - validationStarted) / 1000
    result.inputCandidates = inputCandidates.length
    for (const row of result.instances) {
      if (!row.candidate) continue
      const context = contexts.get(row.instanceId)
      if (!context) {save(); continue}
      const {candidate, options, policy, executor, directory, phaseDirectory} = context
      try {
        if (context.proof) {
          const execution = await makeMatrixExecutionIdentityFromProof({proof: context.proof, options, prepared: candidate, ...policy})
          const checked = lookupMatrixReceipt({...context.cached, execution})
          if (checked.hit) {
            write(path.join(directory, 'execution-identity.json'), execution)
            row.executionKey = execution.key; row.bundleSha256 = execution.identity.compile.bundleSha256
            row.compilationSeconds = 0; row.compilationSkipped = true; result.skippedCompilations++
            row.logicalFileState = checked.data.files[0].state; row.logicalFileName = checked.data.files[0].name
            row.decision = 'replay'; row.reason = null; row.stage = 'complete'; row.receiptSha256 = checked.receiptSha256
            fs.writeFileSync(path.join(directory, 'replayed-response.json'), checked.responseBytes)
            dataById.set(row.instanceId, checked.data); result.replayedInstances++; save(); continue
          }
          row.cacheMissReason = checked.reason
        }
        row.stage = 'compilation'
        const produced = await compile('runner.mjs', options, candidate.absolute)
        const built = {...produced, options, preparation: candidate.identity}
        write(path.join(directory, 'test-build.json'), {...built, bundle: undefined})
        row.compilationSeconds = built.seconds; row.bundleSha256 = built.sha256
        context.built = built
        try {
          // Only the actual factory-issued object can seed the certificate.
          context.certificate = captureResolvedCompileCertificate({project, built: produced, options, testFile: candidate.absolute})
          fs.writeFileSync(path.join(directory, 'compile-certificate.json'), context.certificate.bytes)
        } catch (error) {row.certificateDeclined = error.message}
        const execution = makeMatrixExecutionIdentity({built, prepared: candidate, ...policy})
        context.execution = execution
        write(path.join(directory, 'execution-identity.json'), execution)
        row.executionKey = execution.key
        const phaseName = 'managed-' + row.instanceId
        row.stage = 'lookup'
        if (context.cached) {
          const checked = lookupMatrixReceipt({...context.cached, execution})
          if (checked.hit) {
            row.decision = 'replay'; row.reason = null; row.stage = 'complete'
            row.receiptSha256 = checked.receiptSha256
            row.logicalFileState = checked.data.files[0].state; row.logicalFileName = checked.data.files[0].name
            fs.writeFileSync(path.join(directory, 'replayed-response.json'), checked.responseBytes)
            context.refresh = context.certificate ? {receiptBytes: context.cached.receiptBytes} : undefined
            dataById.set(row.instanceId, checked.data); result.replayedInstances++; save(); continue
          }
          row.cacheMissReason = checked.reason
        }
        row.stage = 'execution'; row.workerEvidenceDirectory = phaseName; result.managedExecutions++; save()
        phaseRows.set(phaseName, row)
        const data = await executor.execute(phaseName, built.bundle)
        dataById.set(row.instanceId, data); row.supported = supported(data)
        row.unsupported = data.boundary?.unsupported || []
        // A closed runner exception can omit reusable rows/files (for example,
        // an onCleanup callback after startTests). Its failure remains real
        // even when the ordinary counterpart passes and supplies report rows.
        row.closedFailure = complete(data) && Array.isArray(data.boundary.unsupported) &&
          data.boundary.unsupported.length === 0 && data.ok !== true
        stickyManagedFailure ||= row.closedFailure
        if (row.supported) {
          const failed = data.ok !== true || data.rows.some(test => test.state === 'fail') ||
            data.files.some(file => file.state === 'fail' || Array.isArray(file.errors) && file.errors.length > 0)
          stickyManagedFailure ||= failed
          row.decision = failed ? 'supported-failure' : data.ok === true ? 'managed' : 'ordinary'
          row.reason = failed ? 'supported actual managed test failed' : data.ok === true ? null : 'managed file did not pass'
        } else if (row.closedFailure) {
          row.decision = 'ordinary'; row.reason = 'sealed managed runner failed'
        } else row.reason = 'managed boundary unsupported/incomplete'
        row.stage = 'complete'
      } catch (error) {row.error = errorData(error); row.reason = error.message}
      save()
    }
  } catch (error) {
    // No native test has run yet. Discard any partial replay decisions and
    // perform one complete ordinary fallback; never repeat a native failure.
    admissionFailure = errorData(error); result.admissionDeclined = admissionFailure
    for (const row of result.instances) {row.decision = 'ordinary'; row.reason = 'research admission declined'}
    result.replayedInstances = 0; save()
  }
  try {
    if (!runMatrixNativeBatch) ({runMatrixNativeBatch} = await import('./matrix-native.mjs'))
    const nonhits = suite?.selected.filter(selected => !result.instances.some(row => row.instanceId === selected.instanceId && row.decision === 'replay'))
    native = await runMatrixNativeBatch({project, out: path.join(out, 'native'), selected: suite?.selected,
      suiteFiles,
      ...(nonhits ? {instanceIds: nonhits.map(row => row.instanceId)} : {})})
    result.native = {...native, data: undefined}; result.nativeSpecsExecuted = native.data?.executed?.length || 0
    observedNativeFailure = native.exitCode !== 0 || native.timedOut === true || (!native.skipped &&
      (native.data?.completed !== true || native.data.matched !== true || native.data.exitCode !== 0 ||
        native.data.unhandledErrors.length > 0 || native.data.failedModules > 0))
    if (!suite) {
      assert(native.data?.originalSelected, 'ordinary fallback did not report actual original selection')
      result.selectedInstances = native.data.originalSelected.length; result.candidateInstances = 0
      result.initialOrdinaryInstances = result.selectedInstances
      result.instances = native.data.originalSelected.map(row => ({instanceId: row.instanceId, project: row.project,
        file: row.file, pool: row.pool, candidate: false, decision: 'ordinary', reason: admissionFailure?.message, nativeExecuted: false}))
    }
    const actual = new Map((native.data?.executed || []).map(row => [row.instanceId, row]))
    assert.equal(actual.size, native.data?.executed?.length || 0)
    const logical = []
    for (const row of result.instances) {
      if (row.decision === 'replay') {
        assert(!actual.has(row.instanceId), 'native ordinary batch repeated a replayed instance')
        for (const test of dataById.get(row.instanceId).rows) logical.push({project: row.project, file: row.file, pool: row.pool, name: test.name, state: test.state})
        continue
      }
      const ordinary = actual.get(row.instanceId)
      assert(ordinary && moduleKey(ordinary) === moduleKey(row), 'ordinary batch omitted/changed an actual nonhit specification')
      row.nativeExecuted = true; row.nativeCount = ordinary.rows.length; row.nativeState = ordinary.state
      row.logicalFileState = ordinary.state; row.logicalFileName = ordinary.relativeModuleId
      const managed = dataById.get(row.instanceId)
      if (row.decision === 'managed' || row.decision === 'supported-failure') {
        row.comparison = compareManagedMatrixStates(managed.rows, ordinary.rows)
        row.fileMatches = managed.files[0].name === ordinary.relativeModuleId && managed.files[0].state === ordinary.state
        if (!row.comparison.equal || !row.fileMatches) {stickyManagedFailure = true; row.disagreement = true}
      }
      for (const test of ordinary.rows) logical.push({project: row.project, file: row.file, pool: row.pool, name: test.name, state: test.state})
    }
    assert.equal(result.instances.filter(row => row.nativeExecuted).length + result.replayedInstances, result.selectedInstances)
    result.exitCode = stickyManagedFailure || observedNativeFailure ? 1 : 0
    result.logicalStates = logical.length
    result.logicalSha256 = sha(canonicalManagedJSON(logical.map(row => [row.project, row.file, row.pool, row.name, row.state]).sort((left, right) => {
      const a = canonicalManagedJSON(left), b = canonicalManagedJSON(right); return a < b ? -1 : a > b ? 1 : 0
    })))
    logicalDocument = {schema: 'quod-managed-matrix-input-logical-results-v1', instances: result.instances,
      files: result.instances.map(row => ({instanceId: row.instanceId, project: row.project, file: row.file,
        pool: row.pool, name: row.logicalFileName, state: row.logicalFileState})),
      rows: logical, sha256: result.logicalSha256, fidelity: result.reportFidelity}
    // One fresh complete native batch supplies conformance only for managed
    // misses. Authenticated hits never need another native conformance run.
    if (result.exitCode === 0 && !admissionFailure && !control && record) {
      const misses = result.instances.filter(row => row.decision === 'managed')
      const pending = []
      const queueCandidate = (context, receiptBytes) => {
        if (!context.certificate) return
        const envelope = encodeMatrixCandidateEnvelope({candidateKey: context.candidateKey,
          certificateBytes: context.certificate.bytes, receiptBytes})
        const file = path.join(context.directory, 'recorded-candidate.json')
        fs.writeFileSync(file, envelope.envelopeBytes, {flag: 'wx'})
        pending.push({candidateKey: context.candidateKey, execution: context.execution,
          file, envelopeSha256: envelope.envelopeSha256})
        context.row.envelopeSha256 = envelope.envelopeSha256
        context.row.envelopeBytes = envelope.envelopeBytes.length
      }
      if (misses.length) {
        const proof = validateMatrixNativeControl({selectionBytes: fs.readFileSync(native.evidenceFile),
          reportBytes: fs.readFileSync(native.reportFile), policy: nativePolicy})
        fs.mkdirSync(path.join(out, 'receipts'))
        for (const row of misses) {
          const context = contexts.get(row.instanceId)
          const conformance = makeMatrixConformance({execution: context.execution, control: proof})
          const receipt = createMatrixReceipt({execution: context.execution, conformance,
            responseBytes: fs.readFileSync(path.join(context.phaseDirectory, 'response.json'))})
          const file = path.join(out, 'receipts', row.instanceId + '.json')
          fs.writeFileSync(file, receipt.receiptBytes, {flag: 'wx'})
          row.receiptSha256 = receipt.receiptSha256
          queueCandidate(context, receipt.receiptBytes)
        }
      }
      for (const context of contexts.values()) if (context.refresh) {
        queueCandidate(context, context.refresh.receiptBytes); result.refreshedCandidates++
      }
      for (const candidate of pending) {await record(candidate); result.recordedReceipts++}
    }
  } catch (error) {result.error = errorData(error); result.exitCode = 1}
  // There is no retry or native rerun after an observed failure or cache error.
  if (publish && !control) {
    try {result.publication = await publish({exitCode: result.exitCode})}
    catch (error) {result.publicationError = errorData(error); result.exitCode = 1}
  }
  result.stickyManagedFailure = stickyManagedFailure; result.observedNativeFailure = observedNativeFailure
  result.ordinaryInstances = result.instances.filter(row => row.decision === 'ordinary').length
  result.nativeConformanceInstances = result.instances.filter(row => row.nativeExecuted && row.decision !== 'ordinary').length
  result.allReviewedCandidatesReplayed = result.replayedInstances === result.candidateInstances && result.candidateInstances > 0
  if (logicalDocument) {
    const file = path.join(out, 'logical-results.json')
    write(file, {...logicalDocument, exitCode: result.exitCode, stickyManagedFailure, observedNativeFailure})
    const bytes = fs.readFileSync(file)
    result.logicalReport = {file, sha256: sha(bytes), bytes: bytes.length,
      schema: logicalDocument.schema, instances: logicalDocument.files.length, states: logicalDocument.rows.length}
  }
  result.passed = result.exitCode === 0; result.seconds = (performance.now() - started) / 1000
  save(); return result
}

if (process.argv[1] && path.resolve(process.argv[1]) === self) {
  const [project, out, flag] = process.argv.slice(2)
  assert(flag === undefined || flag === '--control', 'only the declared --control research flag is supported')
  dispatchManagedInputMatrix({project, out, control: flag === '--control'}).then(result => {
    console.log('QUOD_MANAGED_MATRIX_INPUT_RESULT ' + JSON.stringify(result)); process.exitCode = result.exitCode
  }).catch(error => {console.error(error.stack || error); process.exitCode = 1})
}
