'use strict'
// DRAFT. Builtin-only evidence reader for the four fresh packaged Action jobs.
// Zero pins deliberately reject before the Action, install or test dispatch.
const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const {spawnSync} = require('node:child_process')
const ACTION_B = '97caed0c9b9e998b5b1765c8fde21b4336d368f2'
const SOURCE_Q = '85c96d6c62c9252f3aa464a851f514348353d010'
const PROJECT_C11 = '5eaa7e5160736054d53f8b71e2c522835eb427e0'
const NATIVE_C11 = '09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd'
const CONTRACT = 'quod-managed-v0-protectors-frozen-slots-investigation'
const INTRINSICS = 'quod-managed-frozen-array-promise-slots-v1-probe'
const MAX_JSON = 32 * 1024 * 1024, MAX_FILE = 128 * 1024 * 1024, MAX_TOTAL = 1024 * 1024 * 1024
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const digest = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value)
const pin = value => typeof value === 'string' && /^[a-f0-9]{40}$/.test(value) && !/^0+$/.test(value)
const sort = rows => rows.sort((a, b) => JSON.stringify(a) < JSON.stringify(b) ? -1 : JSON.stringify(a) > JSON.stringify(b) ? 1 : 0)
const stable = item => JSON.stringify((function visit(value) {
  if (value === null || typeof value !== 'object') return value
  return Array.isArray(value) ? value.map(visit) : Object.fromEntries(Object.keys(value).sort().map(key => [key, visit(value[key])]))
})(item))
const write = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', {flag: 'wx'})
const read = (file, bound = MAX_JSON) => {
  const stat = fs.lstatSync(file)
  assert(stat.isFile() && !stat.isSymbolicLink() && stat.size > 0 && stat.size <= bound, 'bounded regular evidence required: ' + path.basename(file))
  const bytes = fs.readFileSync(file); assert.equal(bytes.length, stat.size)
  return {bytes, data: JSON.parse(bytes), sha256: sha(bytes)}
}
function fileBytes(file, bound = MAX_FILE) {
  const stat = fs.lstatSync(file)
  assert(stat.isFile() && !stat.isSymbolicLink() && stat.size > 0 && stat.size <= bound, 'bounded regular bytes required')
  const bytes = fs.readFileSync(file); assert.equal(bytes.length, stat.size); return bytes
}
function relative(file) {
  assert(typeof file === 'string' && file && !path.isAbsolute(file) && !file.includes('\\') &&
    file.split('/').every(part => part && part !== '.' && part !== '..'), 'relative selected file required')
  return file
}
function namespace(role) {return `packaged-${process.env.GITHUB_RUN_ID}-${role === 'v0' ? 'v0' : role === 'control' ? 'matrix-control' : 'matrix'}-`}
function validatePins(role) {
  assert(['seed', 'control', 'consumer', 'v0'].includes(role), 'known proof role required')
  assert(pin(ACTION_B) && pin(SOURCE_Q), 'reviewed built Action B and runtime source Q pins are pending')
  assert.equal(process.env.PACKAGED_ACTION_SHA, ACTION_B)
  assert.equal(process.env.PACKAGED_SOURCE_SHA, SOURCE_Q)
  assert(pin(process.env.GITHUB_SHA) && /^\d+$/.test(process.env.GITHUB_RUN_ID || ''))
  assert.equal(process.version, 'v24.18.0')
}
function preflight(role, out) {
  validatePins(role)
  const project = fs.realpathSync(process.env.GITHUB_WORKSPACE)
  assert.equal(fs.readFileSync(path.join(project, '.nvmrc'), 'utf8').trim(), '24.18.0')
  const head = spawnSync('git', ['rev-parse', 'HEAD'], {cwd: project, encoding: 'utf8'})
  assert.equal(head.status, 0); assert.equal(head.stdout.trim(), PROJECT_C11)
  fs.mkdirSync(out, {recursive: true}); assert.equal(fs.readdirSync(out).length, 0)
  fs.copyFileSync(__filename, path.join(out, 'proof-probe.cjs'), fs.constants.COPYFILE_EXCL)
  write(path.join(out, 'preflight.json'), {schema: 'quod-matrix-packaged-proof-preflight-v1', role,
    actionPin: ACTION_B, sourcePin: SOURCE_Q, workflowPin: process.env.GITHUB_SHA,
    projectPin: PROJECT_C11, project, namespace: namespace(role), nodeVersion: process.version,
    commands: ['npm ci', 'npm run build', 'npm run test'], fullJobHit: false})
}
function runtime(role, out) {
  validatePins(role)
  const installed = path.join(path.dirname(process.env.RUNNER_TEMP), '_actions/pango07/quod-action', ACTION_B)
  const dist = path.join(installed, 'dist'), manifest = read(path.join(dist, 'managed/manifest.json'))
  assert.equal(manifest.data.schema, 'quod-managed-action-runtime-v1')
  assert.deepEqual(manifest.data.source, {repository: 'pango07/quod', commit: SOURCE_Q})
  const archive = fileBytes(path.join(dist, 'managed/runtime.tar.gz'))
  assert.equal(sha(archive), manifest.data.sha256)
  const base = path.join(process.env.RUNNER_TEMP, 'quod'), config = read(path.join(base, 'config.json')).data
  assert.equal(fs.realpathSync(config.root), fs.realpathSync(process.env.GITHUB_WORKSPACE))
  assert.equal(config.managed.mode, role === 'v0' ? 'v0' : role === 'control' ? 'matrix-control' : 'matrix-research')
  assert.equal(config.managed.namespace, namespace(role))
  assert.equal(config.managed.write, role === 'seed' || role === 'v0')
  assert.equal(config.managed.epoch, 1700000000000); assert.equal(config.managed.seed, 42)
  assert.deepEqual(config.managed.env, {NODE_ENV: 'test'})
  assert.deepEqual(config.managed.source, manifest.data.source); assert.equal(config.managed.archiveSha256, sha(archive))
  assert.equal(config.managed.runtime, path.join(base, 'managed-runtime'))
  const directories = ['main', 'post', 'managed-bridge', 'matrix-bridge']
  const members = ['50.index.js', '835.index.js', 'index.js']
  for (const directory of directories) assert.deepEqual(fs.readdirSync(path.join(dist, directory)).sort(), members,
    'installed ncc directory differs from the reviewed built Action')
  const names = directories.flatMap(directory => members.map(member => directory + '/' + member))
  const files = names.map(file => {const bytes = fileBytes(path.join(dist, file)); return {file, bytes: bytes.length, sha256: sha(bytes)}})
  fs.writeFileSync(path.join(out, 'runtime.tar.gz'), archive, {flag: 'wx'})
  assert.equal(sha(fileBytes(path.join(out, 'runtime.tar.gz'))), manifest.data.sha256)
  fs.writeFileSync(path.join(out, 'action-manifest.json'), manifest.bytes, {flag: 'wx'})
  write(path.join(out, 'runtime-provenance.json'), {schema: 'quod-matrix-packaged-runtime-v1', actionPin: ACTION_B,
    sourcePin: SOURCE_Q, archiveSha256: sha(archive), archiveBytes: archive.length, files,
    sessionLifecycleScope: 'finite trusted install/build; matrix session consumed before Vitest dispatch, not a general same-UID isolation claim'})
}
function logEvidence(text, marker) {
  const found = [], release = []
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/^[0-9TZ:.-]+ /, '').replace(/\u001b\[[0-9;]*m/g, '')
    if (line.startsWith(marker + ' ')) found.push(JSON.parse(line.slice(marker.length + 1)))
    if (/^(?:# |ℹ )(?:tests|pass|fail|cancelled|skipped|todo) \d+$/.test(line)) release.push(line.replace(/^(?:# |ℹ )/, ''))
  }
  assert.equal(found.length, 1, 'one actual bridge invocation required')
  assert.deepEqual(release.sort(), ['cancelled 0', 'fail 0', 'pass 20', 'skipped 0', 'tests 20', 'todo 0'])
  return found[0]
}
function closed(data, preflight = false) {
  assert.equal(data.ok, true)
  const boundary = data.boundary, completion = boundary.completion
  assert.equal(boundary.contract, CONTRACT); assert.equal(boundary.intrinsicPolicy, INTRINSICS)
  assert.equal(completion.sealed, true); assert.equal(completion.intrinsicLockdown, true)
  for (const field of ['unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors']) assert.deepEqual(boundary[field], [])
  if (preflight) assert.deepEqual(boundary.unsupported, ['global fetch', 'global crypto', 'global WeakRef', 'global FinalizationRegistry',
    'Event.timeStamp', 'global WebAssembly', 'AbortSignal.timeout', 'eval', 'Function constructor', 'AsyncFunction constructor',
    'GeneratorFunction constructor', 'AsyncGeneratorFunction constructor', 'global EventTarget.addEventListener',
    'global EventTarget.removeEventListener', 'global EventTarget.dispatchEvent'])
  else assert.deepEqual(boundary.unsupported, [])
  assert.equal(boundary.pendingTimers, 0)
  for (const field of ['pendingTimerCallbacks', 'pendingEventCallbacks', 'pendingMicrotasks']) assert.equal(completion[field], 0)
  if (!preflight) {
    assert(data.files.every(file => ['pass', 'skip', 'todo'].includes(file.state) && file.errors.length === 0))
    assert(data.rows.every(row => ['pass', 'skip', 'todo'].includes(row.state)))
  }
}
function logical(data) {
  assert.equal(data.schema, 'quod-managed-matrix-input-logical-results-v1'); assert.equal(data.exitCode, 0)
  assert.equal(data.files.length, 120); assert.equal(data.instances.length, 120); assert.equal(data.rows.length, 4131)
  const identities = rows => sort(rows.map(row => [row.instanceId, row.project, row.file, row.pool]))
  assert.deepEqual(identities(data.files), identities(data.instances))
  const groups = new Map(), fileStates = []
  for (const file of data.files) {
    assert(digest(file.instanceId) && typeof file.project === 'string' && file.project && file.pool === 'forks')
    relative(file.file); assert(['pass', 'skip', 'todo'].includes(file.state))
    const key = stable([file.project, file.file, file.pool]); assert(!groups.has(key)); groups.set(key, [])
    fileStates.push([file.project, file.file, file.pool, file.name, file.state])
  }
  assert.equal(new Set(data.files.map(file => file.instanceId)).size, 120)
  const rows = data.rows.map(row => {
    const group = groups.get(stable([row.project, row.file, row.pool]))
    assert(group && typeof row.name === 'string' && ['pass', 'skip', 'todo'].includes(row.state))
    group.push([row.name, row.state]); return [row.project, row.file, row.pool, row.name, row.state]
  })
  const strong = sha(JSON.stringify(sort(rows))); assert.equal(strong, data.sha256)
  const projection = sha(JSON.stringify(sort([...groups].map(([key, rows]) => [JSON.parse(key)[1], sort(rows)]))))
  return {strong, projection, fileStatesSha256: sha(JSON.stringify(sort(fileStates)))}
}
const namedStates = rows => sort(rows.map(row => [row.name, row.state]))
function matrix(role, out, log, provenance) {
  const base = path.join(process.env.RUNNER_TEMP, 'quod'), reports = path.join(base, 'matrix-reports')
  const names = fs.readdirSync(reports).filter(name => name.endsWith('.summary.json')); assert.equal(names.length, 1)
  const summary = read(path.join(reports, names[0])), event = summary.data
  assert.deepEqual(event, logEvidence(log, 'QUOD_MANAGED_MATRIX_INPUT_RESULT'))
  assert.equal(event.schema, 'quod-managed-matrix-action-result-v1')
  assert.equal(event.mode, role === 'control' ? 'matrix-control' : 'matrix-research')
  assert.equal(event.exitCode, 0); assert.equal(event.fullJobHit, false); assert.equal(event.stickyManagedFailure, false)
  assert.equal(event.selectedScope.kind, 'complete-original-default-selection')
  assert.equal(event.selectedInstances, 120); assert.equal(event.candidateInstances, 108); assert.equal(event.logicalStates, 4131)
  assert.equal(event.cacheTransport.scopeInput.source.commit, SOURCE_Q)
  assert.equal(event.cacheTransport.scopeInput.runtimeArchiveSha256, provenance.archiveSha256)
  assert.equal(event.cacheTransport.scopeInput.bridgeSha256, provenance.files.find(file => file.file === 'matrix-bridge/index.js').sha256)
  assert.equal(event.cacheTransport.namespace, namespace(role))
  assert.equal(sha(stable(event.cacheTransport.scopeInput)), event.cacheTransport.scope)
  assert.equal(event.authLifecycle.sessionConsumedBeforeVitestDispatch, true)
  assert.equal(event.authLifecycle.authAbsentOutsideSdk, true)
  assert(!fs.existsSync(path.join(base, 'managed-cache-auth.json')))
  const logicalFile = path.join(reports, names[0].replace(/\.summary\.json$/, '.logical.json'))
  assert.equal(event.logicalReport.file, logicalFile)
  const document = read(logicalFile), fingerprints = logical(document.data)
  assert.equal(document.sha256, event.logicalReport.sha256); assert.equal(document.bytes.length, event.logicalReport.bytes)
  assert.equal(event.logicalReport.instances, 120); assert.equal(event.logicalReport.states, 4131)
  assert.equal(event.logicalSha256, fingerprints.strong)
  const invocation = event.evidenceDirectory
  assert.equal(path.dirname(invocation), base); assert(/^matrix-run-[A-Za-z0-9]+$/.test(path.basename(invocation)))
  const dispatch = read(path.join(invocation, 'result.json')).data
  for (const field of ['exitCode', 'selectedInstances', 'candidateInstances', 'replayedInstances', 'managedExecutions',
    'skippedCompilations', 'nativeSpecsExecuted', 'recordedReceipts', 'logicalSha256', 'logicalStates']) assert.equal(dispatch[field], event[field])
  assert.equal(dispatch.passed, true); assert.equal(dispatch.observedNativeFailure, false)
  assert.equal(dispatch.instances.length, 120)
  const native = read(path.join(invocation, 'native/native-selection.json')).data
  assert(native.matched && native.completed && native.exitCode === 0)
  assert.deepEqual(native.unhandledErrors, []); assert.equal(native.failedModules, 0)
  assert.equal(native.originalSelected.length, 120)
  const identity = rows => sort(rows.map(row => [row.instanceId, row.project, row.file, row.pool]))
  assert.deepEqual(identity(native.originalSelected), identity(dispatch.instances))
  assert.deepEqual(identity(native.selected), identity(native.executed))
  const nativeReport = read(path.join(invocation, 'native/native-report.json'))
  assert.equal(nativeReport.sha256, native.reportSha256); assert.equal(nativeReport.data.success, true)
  const stateMap = {passed: 'pass', failed: 'fail', skipped: 'skip', todo: 'todo', pending: 'pending'}
  assert.deepEqual(sort(native.executed.flatMap(file => file.rows.map(row => [file.absolute, row.name, row.state]))),
    sort(nativeReport.data.testResults.flatMap(file => file.assertionResults.map(row => [file.name, row.fullName, stateMap[row.status]]))))
  const candidates = dispatch.instances.filter(row => row.candidate), ordinary = dispatch.instances.filter(row => !row.candidate)
  assert.equal(candidates.length, 108); assert.equal(ordinary.length, 12)
  assert.equal(dispatch.compilationPolicy, 'closed-adjacent-full-fences-before-any-candidate-worker-v1-diagnostic')
  assert(!Object.hasOwn(dispatch, 'admissionDeclined'), 'packaged role must not hide an admission fallback')
  const batchFile = path.join(invocation, 'batch-compilation.json')
  if (role === 'seed' || role === 'control') {
    const batch = dispatch.compilationBatch
    assert.equal(batch.attempted, true); assert.equal(batch.requests, 108); assert.equal(batch.completed, true)
    assert(!Object.hasOwn(batch, 'error'), 'successful cold role retained a batch error')
    assert.equal(batch.diagnostics.requests, 108); assert.equal(batch.diagnostics.fullFences, 218)
    const expectedTrace = ['before-constructor', 'after-constructor/before-compile:0',
      ...Array.from({length: 108}, (_, index) => ['after-compile:' + index + '/before-capture:' + index,
        index === 107 ? 'after-capture:' + index + '/before-publication'
          : 'after-capture:' + index + '/before-compile:' + (index + 1)]).flat()]
    assert.deepEqual(batch.diagnostics.trace, expectedTrace, 'cold packaged role must use the exact 218 full batch fences')
    assert.deepEqual(read(batchFile).data, batch, 'actual batch artifact differs from dispatch evidence')
  } else {
    assert.deepEqual(dispatch.compilationBatch, {attempted: false, requests: 0, completed: false, seconds: 0})
    assert(!fs.existsSync(batchFile), 'consumer unexpectedly materialized a candidate compilation batch')
    assert.deepEqual(dispatch.phases.map(phase => phase.name), ['preflight'], 'consumer started a candidate worker')
    for (const row of candidates) {
      assert.equal(row.evidenceDirectory, 'instances/' + row.instanceId)
      const directory = path.join(invocation, row.evidenceDirectory)
      assert(!fs.existsSync(path.join(directory, 'test-build.json')), 'consumer retained a candidate build')
      assert(!fs.existsSync(path.join(directory, 'compile-certificate.json')), 'consumer retained a newly captured certificate')
      assert(!row.workerEvidenceDirectory, 'consumer retained a candidate worker path')
    }
  }
  const preflight = read(path.join(invocation, 'preflight/response.json')).data
  closed(preflight, true); assert.equal(preflight.checks.length, 31); assert(preflight.checks.every(check => check.pass))
  const transport = read(event.cacheTransport.file).data
  assert.equal(event.cacheTransport.file, path.join(invocation, 'cache-transport.json'))
  assert.equal(transport.schema, 'quod-managed-matrix-action-cache-v1')
  const restores = transport.events.filter(row => row.operation === 'collection-restore')
  const saves = transport.events.filter(row => row.operation === 'collection-save')
  const entries = transport.events.filter(row => row.operation === 'collection-entry')
  assert(transport.events.every(row => ['collection-restore', 'collection-save', 'collection-entry'].includes(row.operation)))
  const prefix = `quod-managed-collection-v1-${namespace(role)}${event.cacheTransport.scope}-`
  let collectionSha256 = null, cacheId = null
  if (role === 'consumer') {
    assert(digest(process.env.SEED_LOGICAL_SHA) && digest(process.env.CONTROL_LOGICAL_SHA) && digest(process.env.SEED_COLLECTION_SHA))
    assert(Number.isSafeInteger(Number(process.env.SEED_CACHE_ID)) && Number(process.env.SEED_CACHE_ID) > 0)
    assert.equal(event.cacheTransport.scope, process.env.SEED_SCOPE_SHA)
    assert.equal(event.cacheTransport.scope, process.env.CONTROL_SCOPE_SHA)
    assert.equal(fingerprints.strong, process.env.SEED_LOGICAL_SHA); assert.equal(fingerprints.strong, process.env.CONTROL_LOGICAL_SHA)
    assert.equal(fingerprints.fileStatesSha256, process.env.SEED_FILE_STATES_SHA)
    assert.equal(fingerprints.fileStatesSha256, process.env.CONTROL_FILE_STATES_SHA)
    assert.equal(event.replayedInstances, 108); assert.equal(event.skippedCompilations, 108)
    assert.equal(event.managedExecutions, 0); assert.equal(event.nativeSpecsExecuted, 12); assert.equal(event.recordedReceipts, 0)
    assert.equal(dispatch.inputCandidates, 108)
    assert(candidates.every(row => row.decision === 'replay' && row.compilationSkipped === true && row.compilationSeconds === 0 && !row.workerEvidenceDirectory && row.nativeExecuted === false))
    assert.deepEqual(identity(native.selected), identity(ordinary)); assert.equal(native.omittedAsHits.length, 108)
    assert.equal(restores.length, 1); assert.equal(saves.length, 0); assert.equal(entries.length, 108)
    assert.equal(restores[0].reason, 'hit'); assert.equal(restores[0].collectionSha256, process.env.SEED_COLLECTION_SHA)
    assert.equal(restores[0].matchedServiceKey, prefix + process.env.SEED_COLLECTION_SHA)
    assert.equal(new Set(entries.map(row => row.executionKey)).size, 108)
    assert(entries.every(row => row.matchedServiceKey === restores[0].matchedServiceKey && row.collectionSha256 === restores[0].collectionSha256))
    assert.equal(event.publication.saved, false)
    assert(!fs.readdirSync(invocation).some(name => name.startsWith('managed-')), 'replay consumer created a managed test worker')
    collectionSha256 = restores[0].collectionSha256
  } else {
    assert.equal(event.replayedInstances, 0); assert.equal(event.skippedCompilations, 0)
    assert.equal(event.managedExecutions, 108); assert.equal(event.nativeSpecsExecuted, 120)
    assert.equal(native.selected.length, 120); assert.equal(native.omittedAsHits.length, 0)
    assert.equal(nativeFingerprint(nativeReport.data, process.env.GITHUB_WORKSPACE), NATIVE_C11)
    assert(candidates.every(row => row.decision === 'managed' && row.nativeExecuted && row.comparison.equal && row.fileMatches && !row.disagreement))
    if (role === 'control') {
      assert.equal(event.recordedReceipts, 0); assert.deepEqual(transport.events, [])
    } else {
      assert.equal(event.recordedReceipts, 108); assert.equal(restores.length, 1); assert.equal(restores[0].reason, 'miss')
      assert.equal(saves.length, 1); assert.equal(entries.length, 0); assert.equal(saves[0].entries, 108)
      assert(Number.isSafeInteger(saves[0].cacheId) && saves[0].cacheId > 0)
      assert.equal(saves[0].key, prefix + saves[0].collectionSha256)
      assert.equal(event.publication.saved, true); assert.equal(event.publication.cacheId, saves[0].cacheId)
      collectionSha256 = saves[0].collectionSha256; cacheId = saves[0].cacheId
    }
  }
  const nativeById = new Map(native.executed.map(row => [row.instanceId, row]))
  for (const candidate of candidates) {
    const directory = path.join(invocation, 'instances', candidate.instanceId)
    const response = read(role === 'consumer' ? path.join(directory, 'replayed-response.json') : path.join(invocation, candidate.workerEvidenceDirectory, 'response.json')).data
    closed(response)
    const logicalRows = document.data.rows.filter(row => row.project === candidate.project && row.file === candidate.file && row.pool === candidate.pool)
    assert.deepEqual(namedStates(response.rows), namedStates(logicalRows))
    const logicalFile = document.data.files.find(row => row.instanceId === candidate.instanceId)
    assert.equal(response.files[0].name, logicalFile.name); assert.equal(response.files[0].state, logicalFile.state)
    if (role !== 'consumer') {
      const actual = nativeById.get(candidate.instanceId)
      assert.deepEqual(namedStates(response.rows), namedStates(actual.rows))
      assert.equal(response.files[0].name, actual.relativeModuleId); assert.equal(response.files[0].state, actual.state)
    }
  }
  for (const row of ordinary) {
    const actual = nativeById.get(row.instanceId); assert(actual)
    assert.deepEqual(namedStates(actual.rows), namedStates(document.data.rows.filter(test => test.project === row.project && test.file === row.file && test.pool === row.pool)))
    const file = document.data.files.find(file => file.instanceId === row.instanceId)
    assert.equal(actual.relativeModuleId, file.name); assert.equal(actual.state, file.state)
  }
  if (role !== 'control') {
    const kind = role === 'seed' ? 'published' : 'restored', descriptor = event.retainedCollections[kind]
    assert.equal(descriptor.file, path.join(invocation, kind + '-collection.json'))
    const pack = read(descriptor.file)
    assert.equal(pack.sha256, collectionSha256); assert.equal(descriptor.sha256, collectionSha256)
    assert.equal(pack.bytes.length, descriptor.bytes); assert.equal(descriptor.serviceKey, prefix + collectionSha256)
    assert.equal(pack.data.schema, 'quod-managed-collection-v1'); assert.equal(pack.data.scope, event.cacheTransport.scope)
    assert.deepEqual(Object.keys(pack.data), ['schema', 'scope', 'entries'])
    assert.equal(pack.data.entries.length, 108); assert.equal(Buffer.from(JSON.stringify(pack.data) + '\n').equals(pack.bytes), true)
    const byKey = new Map(); let previous = ''
    for (const entry of pack.data.entries) {
      assert.deepEqual(Object.keys(entry), ['executionKey', 'receiptSha256', 'receiptBase64'])
      assert(digest(entry.executionKey) && entry.executionKey > previous && digest(entry.receiptSha256))
      const bytes = Buffer.from(entry.receiptBase64, 'base64')
      assert(bytes.length > 0 && bytes.length <= 2 * 1024 * 1024 && bytes.toString('base64') === entry.receiptBase64)
      assert.equal(sha(bytes), entry.receiptSha256); previous = entry.executionKey; byKey.set(entry.executionKey, entry)
    }
    for (const candidate of candidates) {
      const entry = byKey.get(candidate.candidateKey); assert(entry)
      const bytes = fileBytes(path.join(invocation, 'instances', candidate.instanceId, role === 'seed' ? 'recorded-candidate.json' : 'candidate-envelope.json'))
      assert.equal(sha(bytes), entry.receiptSha256)
    }
  }
  fs.writeFileSync(path.join(out, 'bridge-summary.json'), summary.bytes, {flag: 'wx'})
  fs.writeFileSync(path.join(out, 'logical-results.json'), document.bytes, {flag: 'wx'})
  copyTree(invocation, path.join(out, 'invocation'))
  return {logicalSha256: fingerprints.strong, fileStatesSha256: fingerprints.fileStatesSha256, scopeSha256: event.cacheTransport.scope,
    projectionSha256: fingerprints.projection, collectionSha256, cacheId, fullJobHit: false,
    selectedInstances: 120, candidateInstances: 108, replayedInstances: event.replayedInstances,
    skippedCompilations: event.skippedCompilations, managedExecutions: event.managedExecutions,
    nativeSpecsExecuted: event.nativeSpecsExecuted, logicalStates: 4131, sdkRestores: restores.length, sdkSaves: saves.length}
}
function nativeFingerprint(report, root) {
  const rows = report.testResults.map(file => [path.relative(root, file.name), file.status,
    file.assertionResults.map(row => [row.fullName, row.status]).sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)))])
    .sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)))
  assert.equal(rows.length, 120); assert.equal(rows.reduce((count, row) => count + row[2].length, 0), 4131)
  return sha(JSON.stringify(rows))
}
function v0(out, log) {
  const event = logEvidence(log, 'QUOD_MANAGED_RESULT'), invocation = event.evidenceDirectory
  assert.equal(path.dirname(invocation), path.join(process.env.RUNNER_TEMP, 'quod'))
  assert(/^managed-run-[A-Za-z0-9]+$/.test(path.basename(invocation)))
  assert.equal(event.mode, 'v0'); assert.equal(event.hit, false); assert.equal(event.fallback, false)
  assert.equal(event.exitCode, 0); assert.equal(event.managedExecutions, 1)
  const result = read(path.join(invocation, 'result.json')).data
  assert.equal(result.hit, false); assert.equal(result.fallback, false); assert.equal(result.exitCode, 0)
  assert.equal(result.managedExecutions, 1); assert.equal(result.files.length, 1); assert.equal(result.files[0].decision, 'managed')
  assert.equal(result.ordinaryRepeatedCandidate, false); assert.equal(result.ordinaryRuns.length, 1)
  assert.equal(result.phases.filter(row => row.name === 'managed').length, 1)
  assert.deepEqual(result.args.slice(0, 1), ['run'])
  assert(result.args.slice(1).every(arg => /^--(?:reporter=|outputFile\.json=)/.test(arg)), 'old mode must use full root command, not filename filters')
  const data = read(path.join(invocation, 'managed/response.json')).data; closed(data)
  assert.equal(data.rows.length, 47); assert.equal(data.files.length, 1)
  const selection = read(path.join(invocation, 'ordinary-remainder-selection.json')).data
  assert(selection.matched && selection.completed); assert.equal(selection.exitCode, 0)
  assert.equal(selection.selected.length, 120); assert.equal(selection.excluded.length, 1)
  assert.equal(selection.remainder.length, 119); assert.equal(selection.executed.length, 119)
  assert.equal(selection.unhandledErrors, 0); assert.equal(selection.failedModules, 0)
  const keys = rows => sort(rows.map(row => [row.absolute, row.project]))
  assert.deepEqual(keys(selection.executed), keys(selection.remainder))
  assert(!selection.executed.some(row => row.absolute === selection.excluded[0].absolute && row.project === selection.excluded[0].project))
  const report = read(process.env.BENCH_VITEST_REPORT)
  assert.equal(nativeFingerprint(report.data, process.env.GITHUB_WORKSPACE), NATIVE_C11)
  fs.writeFileSync(path.join(out, 'native-report.json'), report.bytes, {flag: 'wx'})
  copyTree(invocation, path.join(out, 'invocation'))
  return {managedExecutions: 1, managedTests: 47, nativeSpecsExecuted: 119, selectedInstances: 120,
    logicalStates: 4131, ordinarySemanticSha256: NATIVE_C11, fullJobHit: false}
}
function copyTree(source, target) {
  let total = 0
  function copy(current, output) {
    const stat = fs.lstatSync(current)
    assert(!stat.isSymbolicLink(), 'symlink not allowed in retained invocation evidence')
    if (stat.isDirectory()) {
      fs.mkdirSync(output); for (const name of fs.readdirSync(current).sort()) copy(path.join(current, name), path.join(output, name))
    } else {
      assert(stat.isFile() && stat.size <= MAX_FILE); total += stat.size; assert(total <= MAX_TOTAL)
      fs.copyFileSync(current, output, fs.constants.COPYFILE_EXCL)
    }
  }
  copy(source, target)
}
function retain(role, out) {
  validatePins(role)
  const base = path.join(process.env.RUNNER_TEMP, 'quod')
  const prefix = role === 'v0' ? 'managed-run-' : 'matrix-run-'
  const names = fs.existsSync(base) ? fs.readdirSync(base).filter(name => name.startsWith(prefix)) : []
  assert(names.length <= 1, 'unexpected repeated invocation directories')
  if (names.length && !fs.existsSync(path.join(out, 'invocation'))) copyTree(path.join(base, names[0]), path.join(out, 'invocation'))
  const reports = path.join(base, 'matrix-reports')
  if (role !== 'v0' && fs.existsSync(reports)) copyTree(reports, path.join(out, 'retained-reports'))
}
function after(role, out) {
  validatePins(role)
  const provenance = read(path.join(out, 'runtime-provenance.json')).data
  const log = fileBytes(path.join(out, 'test.log')).toString('utf8')
  const data = role === 'v0' ? v0(out, log) : matrix(role, out, log, provenance)
  const result = {schema: 'quod-matrix-packaged-proof-v1', role, passed: true, actionPin: ACTION_B, sourcePin: SOURCE_Q,
    workflowPin: process.env.GITHUB_SHA, projectPin: PROJECT_C11, archiveSha256: provenance.archiveSha256,
    namespace: namespace(role), ...data,
    limits: ['Four fresh diagnostic jobs, not the original48 window or a controlled speedup claim.',
      'Matrix reports logical project/file/pool/name/state fidelity; native reporter timing metadata is not reconstructed.',
      'Earlier finite install/build steps remain trusted; no general same-UID credential-isolation claim.']}
  write(path.join(out, 'proof.json'), result)
  if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT,
    'logical_sha256=' + (result.logicalSha256 || '') + '\nfile_states_sha256=' + (result.fileStatesSha256 || '') +
    '\ncollection_sha256=' + (result.collectionSha256 || '') + '\ncache_id=' + (result.cacheId || '') +
    '\nscope_sha256=' + (result.scopeSha256 || '') + '\n')
  console.log('QUOD_MATRIX_PACKAGED_PROOF ' + JSON.stringify(result))
}
try {
  const [command, role, out] = process.argv.slice(2)
  assert(path.isAbsolute(out), 'absolute owned proof output required')
  if (command === 'preflight') preflight(role, out)
  else if (command === 'runtime') runtime(role, out)
  else if (command === 'after') after(role, out)
  else if (command === 'retain') retain(role, out)
  else throw Error('usage: proof-probe.cjs preflight|runtime|after|retain ROLE OUT')
} catch (error) {
  console.error(error.stack || error); process.exitCode = 1
}
