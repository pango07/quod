// GitHub-only mutation matrix for the separate production-disabled replay probe.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {canonicalReceiptJSON, makeExecutionIdentity, validatePassingResponse, recordReceipt, lookupReceipt} from './receipt.mjs'

const [outArg] = process.argv.slice(2)
assert(outArg, 'usage: receipt-fixtures.mjs EMPTY_OUTPUT_DIRECTORY')
const out = path.resolve(outArg)
assert(!fs.existsSync(out)); fs.mkdirSync(out, {recursive: true})
const hash = value => crypto.createHash('sha256').update(value).digest('hex')
const copy = value => JSON.parse(JSON.stringify(value))
const checks = [], check = (name, pass) => { checks.push({name, pass: Boolean(pass)}); assert(pass, name) }
const throws = callback => { try { callback(); return false } catch { return true } }
function inputs() {
  const bundle = 'export default {fetch(){return 42}}\n'
  const config = {root: '/project', sequence: {seed: 42}}
  const identity = {schema: 'quod-managed-compile-v1-research', compilerSha256: hash('compiler'),
    bundleSha256: hash(bundle), configurationSha256: hash('env=declared'),
    globs: [{names: ['./one.mjs']}], sources: [{source: '/project/test.mjs', sourceSha256: hash('source')}]}
  return {built: {bundle, sha256: hash(bundle), identity, identitySha256: hash(canonicalReceiptJSON(identity))},
    prepared: {config, identity: {schema: 'quod-managed-preparation-v1-research',
      configurationSha256: hash(canonicalReceiptJSON(config)), preparerSha256: hash('preparer')}},
    runtimePolicy: {schema: 'quod-managed-runtime-policy-v1-research', workerdSha256: hash('runtime'),
      nativeFingerprintSha256: hash('native fingerprint'), compatibilityDate: '2026-07-01',
      compatibilityFlags: ['nodejs_als'], moduleName: 'worker.mjs', services: ['main', 'deny'], globalOutbound: 'deny',
      denyModuleSha256: hash('outbound deny service'), bindings: [], processEnv: {PATH: '/declared/bin', TZ: 'UTC', LANG: 'C.UTF-8'},
      request: {method: 'GET', path: '/run'}, supervisor: {sourceSha256: hash('supervisor'), nodeVersion: 'v24.18.0',
        nodeExecutableSha256: hash('node'), watchdogMs: 180000, startupAttempts: 80, healthTimeoutMs: 500,
        startupRetryMs: 100, shutdownGraceMs: 1000, responseBytesMax: 16777216,
        completionBounds: {nativeTurnFencesMax: 512, timerCallbackBudget: 4096,
          timerCallbackBudgetScope: 'callbacks after completion begins; reject when exceeded', quietNativeTurns: 2}}},
    supervisorPolicy: {receiptValidatorSha256: hash('validator'), experimentSha256: hash('experiment')},
    host: {ImageOS: 'ubuntu26', ImageVersion: 'research', platform: 'linux', arch: 'x64', nodeVersion: 'v24.18.0',
      kernel: {release: 'research-release', version: 'research-version'},
      libc: {family: 'glibc', runtimeVersion: 'research', compilerVersion: 'research'}}}
}
function rehash(built) {
  built.sha256 = hash(built.bundle); built.identity.bundleSha256 = built.sha256
  built.identitySha256 = hash(canonicalReceiptJSON(built.identity))
}
const response = {ok: true, rows: [{name: 'closed managed fixture', state: 'pass', errors: []}],
  files: [{name: 'fixture.test.mjs', state: 'pass', errors: []}], boundary: {
    inventory: ['Object', 'Date'], guarded: ['fetch'], unguardable: [], symbolInventory: [], symbolUnguardable: [],
    unsupported: [], timerErrors: [], unhandledErrors: [],
    pendingTimers: 0, pendingTimerDetails: [], virtualTime: 0,
    completion: {sealed: true, nativeTurnFences: 2, timerCallbacks: 0, pendingTimerCallbacks: 0,
      pendingMicrotasks: 0, pendingEventCallbacks: 0, intrinsicLockdown: true, protectedIntrinsicObjects: 12, reason: ''},
    logs: [['log', ['fixture effect', '42']]], replayEligible: false, contract: 'quod-managed-v0-investigation'}}
const expected = {count: 1, file: 'fixture.test.mjs'}, initial = inputs()
const execution = makeExecutionIdentity(initial), file = path.join(out, 'seed-receipt.json')
const recorded = recordReceipt({file, execution, responseBytes: JSON.stringify(response), expected})
const lookup = (overrides = {}) => lookupReceipt({file, execution, trustedReceiptSha256: recorded.receiptSha256, expected, ...overrides})
const hit = lookup()
check('unchanged validated receipt hits', hit.hit)
check('exact response bytes are preserved', hit.responseBytes.equals(Buffer.from(JSON.stringify(response))))
check('complete primitive console effects are preserved', JSON.stringify(hit.data.boundary.logs) === JSON.stringify(response.boundary.logs))
check('missing trusted seed digest misses', !lookup({trustedReceiptSha256: undefined}).hit)
check('wrong trusted seed digest misses', !lookup({trustedReceiptSha256: hash('other')}).hit)
check('missing explicit receipt misses', !lookup({file: path.join(out, 'missing.json')}).hit)
check('a receipt cannot overwrite existing seed', throws(() => recordReceipt({file, execution, responseBytes: JSON.stringify(response), expected})))

for (const [name, mutate] of [
  ['bundle', input => { input.built.bundle += '// relevant source\n'; rehash(input.built) }],
  ['configuration', input => { input.prepared.config.sequence.seed = 43;
    input.prepared.identity.configurationSha256 = hash(canonicalReceiptJSON(input.prepared.config)) }],
  ['declared environment', input => { input.built.identity.configurationSha256 = hash('env=changed'); rehash(input.built) }],
  ['module-name set', input => { input.built.identity.globs[0].names = ['./renamed.mjs']; rehash(input.built) }],
  ['runtime binary', input => { input.runtimePolicy.workerdSha256 = hash('changed runtime') }],
  ['native fingerprint', input => { input.runtimePolicy.nativeFingerprintSha256 = hash('changed native surface') }],
  ['runtime flags', input => { input.runtimePolicy.compatibilityFlags.push('changed-policy') }],
  ['supervisor', input => { input.runtimePolicy.supervisor.sourceSha256 = hash('changed supervisor') }],
  ['validator', input => { input.supervisorPolicy.receiptValidatorSha256 = hash('changed validator') }],
  ['host', input => { input.host.ImageVersion = 'different-image' }],
]) {
  const changed = inputs(); mutate(changed)
  const altered = makeExecutionIdentity(changed)
  check(name + ' changes execution key', altered.key !== execution.key)
  check(name + ' misses old receipt', !lookup({execution: altered}).hit)
}
fs.writeFileSync(path.join(out, 'unrelated.txt'), 'not supplied as a closed execution input\n')
check('unrelated input absent from execution identity hits', lookup().hit)
check('changed expected count misses', !lookup({expected: {count: 2, file: expected.file}}).hit)
check('changed expected file misses', !lookup({expected: {count: 1, file: 'other.test.mjs'}}).hit)

for (const [name, mutate] of [
  ['test failure', data => { data.ok = false; data.rows[0].state = 'fail' }],
  ['passing flag with failed test', data => { data.rows[0].state = 'fail' }],
  ['file failure', data => { data.files[0].state = 'fail' }],
  ['test error', data => { data.rows[0].errors.push({message: 'error'}) }],
  ['file error', data => { data.files[0].errors.push({message: 'error'}) }],
  ['error response', data => { data.error = {message: 'error'} }],
  ['missing test row', data => { data.rows = [] }],
  ['unknown test state', data => { data.rows[0].state = 'unknown' }],
  ['unsupported operation', data => { data.boundary.unsupported.push('global fetch') }],
  ['unguardable native alias', data => { data.boundary.unguardable.push('native alias') }],
  ['unguardable native symbol', data => { data.boundary.symbolUnguardable.push('native symbol') }],
  ['unsealed completion', data => { data.boundary.completion.sealed = false }],
  ['missing intrinsic lockdown', data => { data.boundary.completion.intrinsicLockdown = false }],
  ['pending timer', data => { data.boundary.pendingTimers = 1 }],
  ['pending timer callback', data => { data.boundary.completion.pendingTimerCallbacks = 1 }],
  ['pending microtask', data => { data.boundary.completion.pendingMicrotasks = 1 }],
  ['pending event callback', data => { data.boundary.completion.pendingEventCallbacks = 1 }],
  ['async timer failure', data => { data.boundary.timerErrors.push('rejected') }],
  ['unhandled rejection', data => { data.boundary.unhandledErrors.push('rejected') }],
  ['obsolete worker response', data => { delete data.boundary.completion }],
  ['non-primitive console effect', data => { data.boundary.logs[0][1].push({toString: 'not primitive'}) }],
  ['unexpected receipt field', data => { data.forged = true }],
]) {
  const changed = copy(response); mutate(changed)
  check(name + ' rejects validation', throws(() => validatePassingResponse(changed, expected)))
  check(name + ' cannot be recorded', throws(() => recordReceipt({file: path.join(out, name.replaceAll(' ', '-') + '.json'),
    execution, responseBytes: JSON.stringify(changed), expected})))
}

const original = fs.readFileSync(file), parsed = JSON.parse(original)
for (const [name, mutate, useNewTrustedSHA] of [
  ['obsolete receipt schema', data => { data.schema = 'quod-managed-receipt-v0-research' }, true],
  ['extra receipt field', data => { data.forged = true }, true],
  ['coherent response tampering', data => {
    const changed = copy(response); changed.rows[0].name = 'forged test'
    const bytes = Buffer.from(JSON.stringify(changed))
    data.response.bytes = bytes.toString('base64'); data.response.sha256 = hash(bytes)
    data.resultDigest = hash('quod-managed-receipt-v1-research\n' + execution.key + '\n' + bytes.toString('base64'))
  }, false],
  ['wrong response checksum', data => { data.response.sha256 = hash('corrupt') }, true],
  ['wrong result domain', data => { data.resultDigest = hash('wrong domain') }, true],
]) {
  const changed = copy(parsed); mutate(changed)
  const target = path.join(out, name.replaceAll(' ', '-') + '.json'), bytes = Buffer.from(canonicalReceiptJSON(changed))
  fs.writeFileSync(target, bytes)
  check(name + ' misses', !lookup({file: target, trustedReceiptSha256: useNewTrustedSHA ? hash(bytes) : recorded.receiptSha256}).hit)
}
const truncated = path.join(out, 'truncated.json'); fs.writeFileSync(truncated, original.subarray(0, original.length - 1))
check('truncated receipt misses', !lookup({file: truncated, trustedReceiptSha256: hash(fs.readFileSync(truncated))}).hit)
const duplicate = path.join(out, 'duplicate-key.json')
fs.writeFileSync(duplicate, original.toString().replace('{', '{"schema":"duplicate",'))
check('duplicate receipt fields miss', !lookup({file: duplicate, trustedReceiptSha256: hash(fs.readFileSync(duplicate))}).hit)
check('duplicate worker response fields cannot be recorded', throws(() => recordReceipt({file: path.join(out, 'duplicate-response.json'),
  execution, responseBytes: JSON.stringify(response).replace('{', '{"ok":false,'), expected})))
let getterInvoked = false
const accessor = copy(response); Object.defineProperty(accessor, 'forged', {enumerable: true, get() { getterInvoked = true; return true }})
check('own accessor rejects without invocation', throws(() => validatePassingResponse(accessor, expected)) && !getterInvoked)
const staleBuilt = inputs(); staleBuilt.built.bundle += 'changed'
check('bundle digest mismatch rejects identity', throws(() => makeExecutionIdentity(staleBuilt)))
const staleConfig = inputs(); staleConfig.prepared.config.sequence.seed = 99
check('configuration digest mismatch rejects identity', throws(() => makeExecutionIdentity(staleConfig)))
for (const [group, key] of [['runtimePolicy', 'workerdSha256'], ['runtimePolicy', 'nativeFingerprintSha256'], ['runtimePolicy', 'supervisor'],
  ['runtimePolicy', 'globalOutbound'], ['supervisorPolicy', 'receiptValidatorSha256'], ['host', 'kernel']]) {
  const omitted = inputs(); delete omitted[group][key]
  check(group + ' omitted ' + key + ' rejects identity', throws(() => makeExecutionIdentity(omitted)))
}

const result = {schema: 'quod-managed-receipt-fixtures-v1', cacheEnabled: false,
  researchOnly: true, checks, seed: recorded}
fs.writeFileSync(path.join(out, 'result.json'), JSON.stringify(result, null, 2) + '\n')
console.log(JSON.stringify(result))
