// Actual fresh-job Actions cache transport investigation. The only receipt
// authority crossing jobs is the SDK-returned immutable service key and bytes.
// Seed/control logical digests are workflow outputs, never receipt artifacts.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {fileURLToPath} from 'node:url'
import {spawnSync} from 'node:child_process'
import {canonicalManagedJSON} from './compiler.mjs'
import {dispatchManagedMatrix} from './matrix-dispatcher.mjs'

const ACTION_PIN = '02f65151206f3a12d6f17e3b3d1831b704f28a0b'
const AUTH_KEYS = ['ACTIONS_RUNTIME_TOKEN', 'ACTIONS_CACHE_URL', 'ACTIONS_RESULTS_URL', 'ACTIONS_CACHE_SERVICE_V2', 'ACTIONS_RUNTIME_URL']
const [role, projectArg, outArg, actionArg, sessionArg, seedLogical, controlLogical, seedCollection] = process.argv.slice(2)
const self = fileURLToPath(import.meta.url), source = path.resolve(path.dirname(self), '../../..')
const out = outArg ? path.resolve(outArg) : null, session = sessionArg ? path.resolve(sessionArg) : null
let project, action, temporary, privateSessionValidated = false
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const digest = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value)
const gitHead = cwd => {
  const child = spawnSync('git', ['rev-parse', 'HEAD'], {cwd, encoding: 'utf8'})
  assert.equal(child.status, 0, 'source commit identity unavailable')
  const sha = child.stdout.trim(); assert(/^[0-9a-f]{40}$/.test(sha)); return sha
}
const absent = () => AUTH_KEYS.every(key => !Object.hasOwn(process.env, key))
const result = {schema: 'quod-managed-matrix-sdk-v1-research', role, passed: false, productionEnabled: false,
  actionPin: ACTION_PIN, sdkCalls: {restore: 0, save: 0}, transport: [], authAbsentOutsideSdk: true,
  privateSessionRemoved: false, actualSdkAuthenticationVerified: false}
const save = () => {if (out && fs.existsSync(out)) fs.writeFileSync(path.join(out, 'sdk-result.json'), JSON.stringify(result, null, 2) + '\n')}
let auth
try {
  assert(['seed', 'control', 'consumer'].includes(role) && projectArg && outArg && actionArg && sessionArg,
    'usage: matrix-sdk-probe.mjs ROLE PROJECT EMPTY_OUTPUT ACTION_CHECKOUT PRIVATE_SESSION [SEED_LOGICAL CONTROL_LOGICAL SEED_COLLECTION]')
  temporary = fs.realpathSync(process.env.RUNNER_TEMP)
  assert(session !== out && !session.startsWith(out + path.sep), 'private session must be outside artifacts')
  assert(session.startsWith(temporary + path.sep), 'private session must remain in this job')
  privateSessionValidated = true
  fs.mkdirSync(out, {recursive: true}); assert.equal(fs.readdirSync(out).length, 0)
  project = fs.realpathSync(path.resolve(projectArg)); action = fs.realpathSync(path.resolve(actionArg))
  assert.equal(gitHead(action), ACTION_PIN, 'reviewed Action source pin required')
  result.sourcePin = gitHead(source)
  const sessionStat = fs.lstatSync(session)
  assert(sessionStat.isFile() && !sessionStat.isSymbolicLink() && sessionStat.size > 0 && sessionStat.size < 65536 &&
    (sessionStat.mode & 0o777) === 0o600, 'private regular0600 cache session required')
  const requireAction = createRequire(path.join(action, 'package.json'))
  const {loadCacheAuth, withCacheAuth} = requireAction('./src/managed-auth.js')
  auth = loadCacheAuth(session)
  assert(typeof auth.ACTIONS_RUNTIME_TOKEN === 'string' && auth.ACTIONS_RUNTIME_TOKEN, 'private session omitted cache authentication')
  // Native tests share this UID. A mode0600 file is not isolated from them:
  // retain the credential only in this supervisor's memory before any spawn.
  fs.unlinkSync(session)
  assert(!fs.existsSync(session), 'private cache session survived loading')
  result.privateSessionRemovedBeforeExecution = true
  for (const key of AUTH_KEYS) delete process.env[key]
  assert(absent(), 'cache authentication must not reach managed/native children')
  const cache = requireAction('@actions/cache')
  const {createManagedCollection, prefix} = requireAction('./src/managed-collection.js')
  const scopeInput = {schema: 'quod-managed-matrix-sdk-scope-v1', sourcePin: result.sourcePin, actionPin: ACTION_PIN,
    collectionSourceSha256: hash(fs.readFileSync(path.join(action, 'src/managed-collection.js'))),
    authSourceSha256: hash(fs.readFileSync(path.join(action, 'src/managed-auth.js'))), nodeVersion: process.version,
    cacheSdkVersion: requireAction('@actions/cache/package.json').version}
  result.scopeSha256 = hash(canonicalManagedJSON(scopeInput)); result.scope = scopeInput
  assert(/^\d+$/.test(process.env.GITHUB_RUN_ID || ''), 'actual workflow run namespace required')
  const namespace = 'matrix-gate-' + process.env.GITHUB_RUN_ID + '-'
  result.namespace = namespace
  const directory = path.join(temporary, 'quod-matrix-collection')
  fs.rmSync(directory, {recursive: true, force: true})
  const sdk = {
    async restoreCache(...args) {result.sdkCalls.restore++; assert(process.env.ACTIONS_RUNTIME_TOKEN === auth.ACTIONS_RUNTIME_TOKEN); return cache.restoreCache(...args)},
    async saveCache(...args) {result.sdkCalls.save++; assert(process.env.ACTIONS_RUNTIME_TOKEN === auth.ACTIONS_RUNTIME_TOKEN); return cache.saveCache(...args)},
  }
  const collection = createManagedCollection({sdk, directory, scope: result.scopeSha256, namespace,
    control: role === 'control', write: role === 'seed',
    async withCacheAuth(operation) {
      assert(absent(), 'cache authentication leaked before an SDK operation')
      assert(!fs.existsSync(session), 'private cache session was recreated')
      try {return await withCacheAuth(auth, operation)}
      finally {result.authAbsentOutsideSdk &&= absent(); assert(absent(), 'cache authentication survived an SDK operation')}
    }, onTransport(event) {result.transport.push(event); save()}})
  result.servicePrefix = prefix(result.scopeSha256, namespace); save()
  assert(!fs.existsSync(session), 'private cache session must be removed before execution')
  const dispatch = await dispatchManagedMatrix({project, out: path.join(out, 'dispatch'), control: role === 'control',
    lookup: collection.lookupBytes, record: role === 'seed' ? collection.record : undefined, publish: collection.publish})
  assert(absent(), 'cache authentication reached the execution interval')
  result.dispatch = dispatch
  assert.equal(dispatch.exitCode, 0, 'whole original logical suite did not pass')
  assert.equal(dispatch.selectedScope.kind, 'complete-original-default-selection')
  assert.equal(dispatch.selectedInstances, 120); assert.equal(dispatch.candidateInstances, 108)
  assert.equal(dispatch.ordinaryInstances, 12); assert.equal(dispatch.logicalStates, 4131)
  assert.equal(dispatch.fullJobHit, false)
  assert.equal(dispatch.stickyManagedFailure, false)
  if (role === 'seed') {
    assert.deepEqual(result.sdkCalls, {restore: 1, save: 1})
    assert.equal(dispatch.managedExecutions, 108); assert.equal(dispatch.replayedInstances, 0)
    assert.equal(dispatch.nativeSpecsExecuted, 120); assert.equal(dispatch.recordedReceipts, 108)
    assert(dispatch.publication?.saved === true && Number.isSafeInteger(dispatch.publication.cacheId) && dispatch.publication.cacheId > 0)
    const restored = result.transport.filter(event => event.operation === 'collection-restore')
    assert.equal(restored.length, 1); assert.equal(restored[0].reason, 'miss')
    const saved = result.transport.filter(event => event.operation === 'collection-save')
    assert.equal(saved.length, 1); assert.equal(saved[0].entries, 108)
    assert.equal(saved[0].collectionSha256, dispatch.publication.collectionSha256)
    result.collectionSha256 = saved[0].collectionSha256; result.cacheId = saved[0].cacheId
    result.actualSdkAuthenticationVerified = true
  } else if (role === 'control') {
    assert.deepEqual(result.sdkCalls, {restore: 0, save: 0})
    assert.equal(dispatch.managedExecutions, 108); assert.equal(dispatch.replayedInstances, 0)
    assert.equal(dispatch.nativeSpecsExecuted, 120); assert.equal(dispatch.recordedReceipts, 0)
  } else {
    assert(digest(seedLogical) && digest(controlLogical) && digest(seedCollection), 'independent seed/control workflow outputs required')
    assert.equal(seedLogical, controlLogical, 'independent uncached controls disagree')
    assert.equal(dispatch.logicalSha256, seedLogical, 'fresh consumer logical results differ from both controls')
    assert.deepEqual(result.sdkCalls, {restore: 1, save: 0})
    assert.equal(dispatch.replayedInstances, 108); assert.equal(dispatch.managedExecutions, 0)
    assert.equal(dispatch.nativeSpecsExecuted, 12); assert.equal(dispatch.recordedReceipts, 0)
    const restored = result.transport.filter(event => event.operation === 'collection-restore')
    assert.equal(restored.length, 1); assert.equal(restored[0].reason, 'hit')
    assert.equal(restored[0].entries, 108); assert.equal(restored[0].collectionSha256, seedCollection)
    assert.equal(restored[0].matchedServiceKey, result.servicePrefix + seedCollection)
    const entries = result.transport.filter(event => event.operation === 'collection-entry')
    assert.equal(entries.length, 108)
    assert.equal(new Set(entries.map(event => event.executionKey)).size, 108)
    assert(entries.every(event => event.matchedServiceKey === restored[0].matchedServiceKey && event.collectionSha256 === seedCollection && digest(event.receiptSha256)))
    assert.equal(dispatch.publication?.saved, false)
    result.collectionSha256 = seedCollection; result.actualSdkAuthenticationVerified = true
  }
  if (role !== 'control') {
    // Data-only audit evidence. These bytes never feed lookup or grant receipt
    // authority; that comes exclusively from the acknowledged SDK service key.
    const collectionFile = path.join(directory, 'collection.json')
    const collectionStat = fs.lstatSync(collectionFile)
    assert(collectionStat.isFile() && !collectionStat.isSymbolicLink() && collectionStat.size > 0 &&
      collectionStat.size <= 32 * 1024 * 1024, 'invalid retained authenticated collection')
    const collectionBytes = fs.readFileSync(collectionFile)
    assert.equal(collectionBytes.length, collectionStat.size)
    assert.equal(hash(collectionBytes), result.collectionSha256)
    const authority = result.transport.find(event => event.operation === (role === 'seed' ? 'collection-save' : 'collection-restore'))
    const serviceKey = role === 'seed' ? authority.key : authority.matchedServiceKey
    assert.equal(serviceKey, result.servicePrefix + result.collectionSha256)
    const retained = path.join(out, 'authenticated-collection.json')
    fs.writeFileSync(retained, collectionBytes, {flag: 'wx'})
    assert.equal(hash(fs.readFileSync(retained)), result.collectionSha256)
    result.retainedCollection = {file: 'authenticated-collection.json', sizeBytes: collectionBytes.length,
      sha256: result.collectionSha256, matchedServiceKey: serviceKey, source: 'SDK-operation-verified-bytes; audit-only'}
  }
  result.logicalSha256 = dispatch.logicalSha256
  result.passed = true
  if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT,
    'logical_sha256=' + result.logicalSha256 + '\ncollection_sha256=' + (result.collectionSha256 || '') +
    '\ncache_id=' + (result.cacheId || '') + '\nscope_sha256=' + result.scopeSha256 + '\n')
} catch (error) {
  result.error = {name: error.name, message: error.message, stack: error.stack}
  process.exitCode = 1
} finally {
  for (const key of AUTH_KEYS) delete process.env[key]
  if (privateSessionValidated) fs.rmSync(session, {force: true})
  result.privateSessionRemoved = !session || !fs.existsSync(session); result.authAbsentOutsideSdk &&= absent()
  save()
  console.log(JSON.stringify({schema: result.schema, role, passed: result.passed, sdkCalls: result.sdkCalls,
    actualSdkAuthenticationVerified: result.actualSdkAuthenticationVerified, logicalSha256: result.logicalSha256,
    privateSessionRemoved: result.privateSessionRemoved}))
}
