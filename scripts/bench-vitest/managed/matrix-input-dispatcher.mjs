// Separate research API: logical project/file/full-name/state fidelity only.
// It does not replace the Action bridge or reconstruct native timing/metadata.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {fileURLToPath} from 'node:url'
import {MATRIX_CANDIDATE_POLICY, selectMatrixCandidates} from './matrix-candidates.mjs'
import {resolveMatrixCandidateProfile, selectMatrixProfileCandidates,
  PROPERTY47_CANDIDATE_PROFILE, PROPERTY47_CANDIDATE} from './matrix-candidate-profiles.mjs'
import {createManagedExecutor} from './executor.mjs'
import {MANAGED_BOUNDARY_CONTRACT, MANAGED_INTRINSIC_POLICY,
  canonicalReceiptJSON as canonicalManagedJSON} from './receipt.mjs'
import {makeMatrixExecutionIdentity, validateMatrixNativeControl, makeMatrixConformance,
  makeMatrixExecutionIdentityFromProof, createMatrixReceipt, lookupMatrixReceipt} from './matrix-receipt.mjs'
import {compileAndCaptureCertifiedBatch, validateResolvedCompileCertificates,
  createCertifiedManagedCompiler} from './resolved-compile-certificate.mjs'
import {makeMatrixCandidateKey, encodeMatrixCandidateEnvelope, decodeMatrixCandidateEnvelope} from './matrix-candidate-envelope.mjs'

const self = fileURLToPath(import.meta.url), here = path.dirname(self)
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n')
const errorData = error => ({name: String(error?.name || 'Error'), message: String(error?.message || error), stack: String(error?.stack || '')})
const moduleKey = row => canonicalManagedJSON([row.project, row.file, row.pool])
const BATCH_POLICY = 'closed-adjacent-full-fences-before-any-candidate-worker-v1-diagnostic'

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
  env = {NODE_ENV: 'test'}, control = false, suiteFiles, candidateProfile = MATRIX_CANDIDATE_POLICY,
  coldOverlap = false,
  lookup, record, publish} = {}) {
  assert(projectArg && outArg, 'explicit research project/output required')
  assert(Number.isSafeInteger(epoch) && epoch >= 0 && Number.isSafeInteger(seed) && seed >= 0 && seed <= 0xffffffff)
  assert.equal(typeof control, 'boolean')
  assert.equal(typeof coldOverlap, 'boolean', 'cold overlap must be a primitive boolean')
  if (coldOverlap) assert([PROPERTY47_CANDIDATE_PROFILE, MATRIX_CANDIDATE_POLICY].includes(candidateProfile) && suiteFiles === undefined,
    'cold overlap requires a closed candidate profile and complete original selection')
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
    refreshedCandidates: 0, candidatePolicy: MATRIX_CANDIDATE_POLICY, compilationPolicy: BATCH_POLICY,
    compilationBatch: {attempted: false, requests: 0, completed: false, seconds: 0}}
  const save = () => write(path.join(out, 'result.json'), result)
  const contexts = new Map(), dataById = new Map()
  let suite, stickyManagedFailure = false, native, nativePolicy, admissionFailure, observedNativeFailure = false
  let runMatrixNativeBatch, compareManagedMatrixStates, logicalDocument
  save()
  let nativeOnce, overlapPolicy, managedPool, makeManagedPool
  const overlapEvidence = () => {
    if (!nativeOnce) return
    result.coldOverlap.nativeOnce = nativeOnce.snapshot()
    write(path.join(out, 'native-once.json'), result.coldOverlap.nativeOnce)
  }
  const reserveNative = requestJSON => {
    const settlement = nativeOnce.start(requestJSON)
    const file = path.join(out, 'native-once-request.json')
    if (!fs.existsSync(file)) fs.writeFileSync(file, requestJSON, {flag: 'wx'})
    else assert.equal(fs.readFileSync(file, 'utf8'), requestJSON, 'reserved native request changed')
    return settlement
  }
  const poolEvidence = () => {
    if (!managedPool) return
    result.coldOverlap.managedPool = managedPool.snapshot()
    write(path.join(out, 'managed-pool.json'), result.coldOverlap.managedPool)
  }
  const ownedNative = async request => {
    // Match the ordinary adapter's optional-field omission without admitting
    // non-JSON values into the synchronously frozen once request.
    const frozen = Object.fromEntries(Object.entries(request).filter(([, value]) => value !== undefined))
    reserveNative(canonicalManagedJSON(frozen))
    const settlement = await nativeOnce.join()
    overlapEvidence()
    if (settlement.status === 'rejected') throw settlement.reason
    return settlement.value
  }
  // Once reserved, the native child is joined even when result/evidence writes
  // throw out of either admission or final-report handling. No retry is made.
  try {
  try {
    // This module's static imports use only builtins. Initialize and pin the
    // tool epoch before preparation/planner/native helpers load the compiler.
    // Match preparation's existing public CLI bootstrap before the snapshot.
    process.env.TEST = 'true'
    process.env.VITEST = 'true'
    process.env.NODE_ENV ??= 'test'
    const compile = await createCertifiedManagedCompiler({project})
    const profile = resolveMatrixCandidateProfile(candidateProfile)
    if (coldOverlap) {
      const {createNativeOnce} = await import('./native-once.mjs')
      const {runOwnedMatrixNativeBatch, ownedMatrixNativePolicy} = await import('./owned-native-adapter.mjs')
      ;({createManagedTaskPool: makeManagedPool} = await import('./managed-task-pool.mjs'))
      nativeOnce = createNativeOnce({run: requestJSON => runOwnedMatrixNativeBatch(JSON.parse(requestJSON))})
      result.coldOverlap = {enabled: true, policy: null, startedBeforeCandidate: false}
      overlapPolicy = {schema: 'quod-owned-cold-overlap-policy-v1-research',
        scheduling: 'all-cold only after every fence, execution key and refresh decision; default108 fixed4 or property47 fixed1; warm/mixed/refresh serial; every started owner joined on every exit',
        schedulingSourceSha256: sha(fs.readFileSync(self)),
        ownedAdapterSha256: sha(fs.readFileSync(path.join(here, 'owned-native-adapter.mjs'))),
        nativeChildSha256: sha(fs.readFileSync(path.join(here, 'matrix-native.mjs'))),
        nativeOnceSha256: sha(fs.readFileSync(path.join(here, 'native-once.mjs'))),
        managedPoolSha256: sha(fs.readFileSync(path.join(here, 'managed-task-pool.mjs'))),
        coldConcurrency: profile.label === MATRIX_CANDIDATE_POLICY ? 4 : 1,
        listenerHelperSha256: sha(fs.readFileSync(path.join(here, 'inherited-listener-executor.mjs'))),
        listenerMode: 'inherited-linux-fd3-control4-v1-research', nativeWatchdogMs: 300000,
        nativeProcessPolicy: ownedMatrixNativePolicy()}
      result.coldOverlap.policy = overlapPolicy
    }
    const {prepareManagedMatrix} = await import('./preparation.mjs')
    const planner = await import('./matrix-planner.mjs')
    const {createManagedMatrixPlan} = planner
    compareManagedMatrixStates = planner.compareManagedMatrixStates
    ;({runMatrixNativeBatch} = await import('./matrix-native.mjs'))
    suite = await prepareManagedMatrix({project, seed, files: suiteFiles})
    write(path.join(out, 'preparation.json'), suite.evidence)
    const plan = createManagedMatrixPlan(suite)
    write(path.join(out, 'matrix-plan.json'), plan)
    const candidates = new Set((profile.label === MATRIX_CANDIDATE_POLICY ? selectMatrixCandidates(suite.selected)
      : selectMatrixProfileCandidates(suite.selected, profile.label)).map(row => row.instanceId))
    if (coldOverlap) assert(suite.selected.length === 120 &&
      candidates.size === (profile.label === MATRIX_CANDIDATE_POLICY ? 108 : 1) && candidates.has(PROPERTY47_CANDIDATE.instanceId),
      'cold overlap requires all original120 and the exact closed candidate roster')
    if (profile.label !== MATRIX_CANDIDATE_POLICY) {
      result.candidatePolicy = profile.label
      result.candidateProfile = profile
    }
    result.selectedInstances = suite.selected.length; result.candidateInstances = candidates.size
    result.initialOrdinaryInstances = suite.selected.length - candidates.size; result.planKey = plan.key
    result.instances = suite.selected.map(row => ({instanceId: row.instanceId, project: row.project, file: row.file,
      pool: row.pool, candidate: candidates.has(row.instanceId), decision: 'ordinary', nativeExecuted: false,
      reason: candidates.has(row.instanceId) ? 'candidate not yet admitted' : 'outside reviewed research candidate selection'}))
    const phaseRows = new Map()
    const onPhase = phase => {
      if (coldOverlap && phaseRows.has(phase.name)) {
        // The executor has already closed its child and delivered actual data.
        // Latch known failure before any result mutation or fallible save; a
        // later callback/write failure must not let ordinary success erase it.
        const data = phase.data
        if (complete(data) && Array.isArray(data.boundary.unsupported) && data.boundary.unsupported.length === 0) {
          stickyManagedFailure ||= data.ok !== true
          if (supported(data)) {
            stickyManagedFailure ||= data.rows.some(test => test.state === 'fail')
            stickyManagedFailure ||= data.files.some(file => file?.state === 'fail' || Array.isArray(file?.errors) && file.errors.length > 0)
          }
        }
      }
      result.phases.push({name: phase.name, seconds: phase.seconds, responseSha256: phase.responseSha256})
      const row = phaseRows.get(phase.name)
      if (row) {row.workerSeconds = phase.seconds; row.responseSha256 = phase.responseSha256}
      save()
    }
    // Factory policy discovery hashes native executables. Reuse that immutable
    // current-host class; every execute() still starts a fresh independent
    // workerd process and verifies its actual binary bytes again.
    const preflightExecutor = createManagedExecutor({out, watchdogMs: 30000, onPhase})
    let propertyExecutor, candidateExecutor
    const preflightBuild = await compile('preflight.mjs', base)
    write(path.join(out, 'preflight-build.json'), {...preflightBuild, bundle: undefined, options: base})
    const preflight = await preflightExecutor.execute('preflight', preflightBuild.bundle)
    assert(preflight.ok === true && preflight.checks.length === 31 && new Set(preflight.checks.map(row => row.name)).size === 31 &&
      preflight.checks.every(row => row.pass === true) && complete(preflight), 'private current-host boundary preflight failed')
    const nativeFingerprintSha256 = sha(canonicalManagedJSON({preflightIdentity: preflightBuild.identitySha256,
      inventory: preflight.boundary.inventory, symbols: preflight.boundary.symbolInventory}))
    const adapterSha256 = sha(fs.readFileSync(path.join(here, coldOverlap ? 'owned-native-adapter.mjs' : 'matrix-native.mjs')))
    nativePolicy = {adapterSha256, nodeExecutableSha256: preflightExecutor.runtimePolicy.supervisor.nodeExecutableSha256,
      nodeVersion: process.version, vitestVersion: '5.0.0', host: preflightExecutor.host}
    const supervisorPolicy = {receiptValidatorSha256: sha(fs.readFileSync(path.join(here, 'matrix-receipt.mjs'))),
      experimentSha256: sha(fs.readFileSync(self)), conformanceAdapterSha256: adapterSha256,
      candidateSelectorSha256: sha(fs.readFileSync(path.join(here, 'matrix-candidates.mjs'))),
      inputValidatorSha256: sha(fs.readFileSync(path.join(here, 'resolved-compile-certificate.mjs'))),
      candidateEnvelopeSha256: sha(fs.readFileSync(path.join(here, 'matrix-candidate-envelope.mjs'))),
      compilationPolicy: BATCH_POLICY,
      ...(coldOverlap ? {overlapPolicy} : {}),
      ...(profile.label === MATRIX_CANDIDATE_POLICY ? {} : {candidateProfile: profile,
        candidateProfileSelectorSha256: sha(fs.readFileSync(path.join(here, 'matrix-candidate-profiles.mjs')))})}
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
        ? (propertyExecutor ||= createManagedExecutor({out, watchdogMs: 180000, onPhase,
          ...(coldOverlap ? {listenerMode: overlapPolicy.listenerMode} : {})})) : coldOverlap
          ? (candidateExecutor ||= createManagedExecutor({out, watchdogMs: 30000, onPhase, listenerMode: overlapPolicy.listenerMode}))
          : preflightExecutor
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
    // Finish all opaque-proof and complete receipt admission before reserving
    // the compiler. Hits remain private/provisional until every cold member has
    // crossed its final full fence. No callback runs inside the closed batch.
    const cold = []
    for (const row of result.instances) {
      if (!row.candidate) continue
      const context = contexts.get(row.instanceId)
      if (!context) {save(); continue}
      const {candidate, options, policy} = context
      if (context.proof) {
        try {
          const execution = await makeMatrixExecutionIdentityFromProof({proof: context.proof, options, prepared: candidate, ...policy})
          const checked = lookupMatrixReceipt({...context.cached, execution})
          if (checked.hit) {
            context.replay = {execution, checked, compilationSkipped: true}
            row.stage = 'receipt-admitted-pending-cold-batch'; continue
          }
          row.cacheMissReason = checked.reason
        } catch (error) {row.cacheMissReason = error.message}
      }
      row.stage = 'queued-for-cold-batch'; cold.push(context)
    }
    if (cold.length) {
      const batchStarted = performance.now()
      result.compilationBatch = {attempted: true, requests: cold.length, completed: false, seconds: 0}
      save()
      let batch
      try {
        batch = await compileAndCaptureCertifiedBatch({project, adapterRoot: here,
          requestsJSON: canonicalManagedJSON(cold.map(context => ({entry: 'runner.mjs',
            options: context.options, testFile: context.candidate.absolute})))})
      } catch (error) {
        result.compilationBatch.seconds = (performance.now() - batchStarted) / 1000
        result.compilationBatch.error = errorData(error)
        result.compilationBatch.fenceTrace = Array.isArray(error.batchFenceTrace) ? error.batchFenceTrace : []
        write(path.join(out, 'batch-compilation.json'), result.compilationBatch)
        throw error
      }
      result.compilationBatch = {...result.compilationBatch, completed: true,
        seconds: (performance.now() - batchStarted) / 1000, diagnostics: batch.diagnostics}
      assert.equal(batch.results.length, cold.length, 'closed compiler batch omitted a requested miss')
      assert.equal(batch.diagnostics.fullFences, 2 * cold.length + 2, 'closed compiler batch full fence inventory differs')
      write(path.join(out, 'batch-compilation.json'), result.compilationBatch)
      // The API has now finished its final full guard and privately issued all
      // actual builds. Use its exact certificate bytes without strict recapture.
      // Derive every execution before any candidate worker or replay publication.
      for (const [index, context] of cold.entries()) {
        const {row, candidate, options, directory, policy} = context
        const {built, certificate} = batch.results[index]
        built.options = options; built.preparation = candidate.identity
        context.built = built; context.certificate = certificate
        assert(Buffer.isBuffer(certificate.bytes) && sha(certificate.bytes) === certificate.sha256,
          'closed batch returned invalid certificate bytes')
        write(path.join(directory, 'test-build.json'), {...built, bundle: undefined})
        fs.writeFileSync(path.join(directory, 'compile-certificate.json'), certificate.bytes, {flag: 'wx'})
        row.compilationSeconds = built.seconds; row.bundleSha256 = built.sha256
        const execution = makeMatrixExecutionIdentity({built, prepared: candidate, ...policy})
        context.execution = execution
        write(path.join(directory, 'execution-identity.json'), execution)
        row.executionKey = execution.key
        row.stage = 'compiled-pending-serial-execution'
        if (context.cached) {
          const checked = lookupMatrixReceipt({...context.cached, execution})
          if (checked.hit) {
            context.replay = {execution, checked, compilationSkipped: false}
            context.refresh = {receiptBytes: context.cached.receiptBytes}
          } else row.cacheMissReason = checked.reason
        }
      }
      save()
    }
    const allCold = coldOverlap && cold.length === candidates.size && contexts.size === candidates.size &&
      ![...contexts.values()].some(context => context.replay)
    if (allCold) {
      assert(result.compilationBatch.completed === true && result.compilationBatch.requests === candidates.size &&
        result.compilationBatch.diagnostics.fullFences === 2 * candidates.size + 2, 'native overlap preceded the final guarded cold batch')
      for (const context of contexts.values()) assert(context.built && context.execution &&
        context.row.executionKey === context.execution.key && !context.refresh,
        'native overlap preceded complete current execution and refresh decisions')
      result.coldOverlap.startedBeforeCandidate = true
      result.coldOverlap.coldReadyMonoMs = performance.now()
      reserveNative(canonicalManagedJSON({project, out: path.join(out, 'native'), selected: suite.selected,
        instanceIds: suite.selected.map(row => row.instanceId)}))
      overlapEvidence(); save()
    }
    if (allCold && profile.label === MATRIX_CANDIDATE_POLICY) {
      managedPool = makeManagedPool({maxConcurrency: 4})
      const tasks = result.instances.filter(row => row.candidate).map(row => ({id: row.instanceId, async run() {
        const context = contexts.get(row.instanceId), phaseName = 'managed-' + row.instanceId
        assert(context?.built && context.execution && !context.replay, 'pool requires every final cold execution before start')
        row.stage = 'execution'; row.workerEvidenceDirectory = phaseName; result.managedExecutions++; save()
        phaseRows.set(phaseName, row)
        let data
        try {data = await context.executor.execute(phaseName, context.built.bundle)}
        catch (error) {row.error = errorData(error); row.reason = error.message; throw error}
        row.workerSettledMonoMs = performance.now()
        dataById.set(row.instanceId, data); row.supported = supported(data)
        row.unsupported = data.boundary?.unsupported || []
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
        row.stage = 'complete'; save()
      }}))
      managedPool.start(tasks)
      const settlement = await managedPool.join()
      poolEvidence()
      if (settlement.status === 'rejected') throw settlement.reason
    } else {
    // Only after all cold builds are closed can any admitted replay omit its
    // native counterpart. Workers remain fresh, serial and in original order.
    for (const row of result.instances) {
      if (!row.candidate) continue
      const context = contexts.get(row.instanceId)
      if (!context) {save(); continue}
      const {executor, directory, phaseDirectory, built} = context
      try {
        if (context.replay) {
          const {execution, checked, compilationSkipped} = context.replay
          context.execution = execution
          write(path.join(directory, 'execution-identity.json'), execution)
          row.executionKey = execution.key; row.bundleSha256 = execution.identity.compile.bundleSha256
          if (compilationSkipped) {
            row.compilationSeconds = 0; row.compilationSkipped = true; result.skippedCompilations++
          }
          row.decision = 'replay'; row.reason = null; row.stage = 'complete'; row.receiptSha256 = checked.receiptSha256
          row.logicalFileState = checked.data.files[0].state; row.logicalFileName = checked.data.files[0].name
          fs.writeFileSync(path.join(directory, 'replayed-response.json'), checked.responseBytes)
          if (coldOverlap) row.replayMaterializedMonoMs = performance.now()
          dataById.set(row.instanceId, checked.data); result.replayedInstances++; save(); continue
        }
        assert(built && context.execution, 'cold candidate has no final guarded build')
        const phaseName = 'managed-' + row.instanceId
        row.stage = 'execution'; row.workerEvidenceDirectory = phaseName; result.managedExecutions++; save()
        phaseRows.set(phaseName, row)
        const data = await executor.execute(phaseName, built.bundle)
        if (coldOverlap) row.workerSettledMonoMs = performance.now()
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
    }
  } catch (error) {
    if (managedPool) {await managedPool.join(); poolEvidence()}
    // Discard provisional replay decisions. An opted-in already reserved
    // native batch is joined by the fallback instead of being started again.
    admissionFailure = errorData(error); result.admissionDeclined = admissionFailure
    for (const row of result.instances) {
      row.decision = 'ordinary'; row.reason = 'research admission declined'
      delete row.compilationSkipped
    }
    for (const context of contexts.values()) {delete context.replay; delete context.refresh}
    dataById.clear(); result.replayedInstances = 0; result.skippedCompilations = 0; save()
  }
  try {
    if (!runMatrixNativeBatch) ({runMatrixNativeBatch} = await import('./matrix-native.mjs'))
    const nonhits = suite?.selected.filter(selected => !result.instances.some(row => row.instanceId === selected.instanceId && row.decision === 'replay'))
    native = await (coldOverlap && nativeOnce ? ownedNative : runMatrixNativeBatch)({project, out: path.join(out, 'native'), selected: suite?.selected,
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
  if (publish && !control && !admissionFailure) {
    try {result.publication = await publish({exitCode: result.exitCode})}
    catch (error) {result.publicationError = errorData(error); result.exitCode = 1}
  }
  if (admissionFailure) result.publication = {saved: false, reason: 'research admission declined before complete candidate execution'}
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
  } catch (error) {
    if (coldOverlap) {result.error = errorData(error); result.exitCode = 1; result.passed = false}
    throw error
  } finally {
    if (managedPool || nativeOnce) {
      // Join both independent owners before any fallible evidence I/O. An
      // unexpected rejected join cannot skip settlement of the other owner.
      const joins = await Promise.allSettled([managedPool?.join(), nativeOnce?.join()])
      assert(joins.every(join => join.status === 'fulfilled'), 'owned cleanup join unexpectedly rejected')
      const settlement = joins[1].value
      if (settlement) {
        const actual = settlement.status === 'fulfilled' ? settlement.value : null
        observedNativeFailure ||= !actual || actual.exitCode !== 0 || actual.timedOut === true || (!actual.skipped &&
          (actual.data?.completed !== true || actual.data.matched !== true || actual.data.exitCode !== 0 ||
            actual.data.unhandledErrors.length > 0 || actual.data.failedModules > 0))
        if (observedNativeFailure) {result.exitCode = 1; result.passed = false}
      }
      result.stickyManagedFailure = stickyManagedFailure; result.observedNativeFailure = observedNativeFailure
      result.seconds = (performance.now() - started) / 1000
      poolEvidence()
      overlapEvidence()
      save()
    }
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === self) {
  const [project, out, flag] = process.argv.slice(2)
  assert(flag === undefined || flag === '--control', 'only the declared --control research flag is supported')
  dispatchManagedInputMatrix({project, out, control: flag === '--control'}).then(result => {
    console.log('QUOD_MANAGED_MATRIX_INPUT_RESULT ' + JSON.stringify(result)); process.exitCode = result.exitCode
  }).catch(error => {console.error(error.stack || error); process.exitCode = 1})
}
