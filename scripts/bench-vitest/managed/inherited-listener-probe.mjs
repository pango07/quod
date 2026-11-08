// Future remote-only transport diagnostic. No cached response is admitted and
// raw failure bundles below make no managed-boundary or test-result claim.
import fs from 'node:fs'
import path from 'node:path'
import net from 'node:net'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {createRequire} from 'node:module'
import {createCertifiedManagedCompiler} from './resolved-compile-certificate.mjs'
import {createManagedExecutor} from './executor.mjs'
import {INHERITED_LISTENER_MODE, parseInheritedListenControl} from './inherited-listener-executor.mjs'
import {MANAGED_BOUNDARY_CONTRACT, MANAGED_INTRINSIC_POLICY} from './receipt.mjs'

assert.equal(process.env.GITHUB_ACTIONS, 'true', 'remote-only inherited listener diagnostic')
const [projectArg, outArg] = process.argv.slice(2)
assert(projectArg && outArg, 'usage: inherited-listener-probe.mjs PROJECT EMPTY_OUT')
const project = fs.realpathSync(projectArg), out = path.resolve(outArg)
assert(!fs.existsSync(out)); fs.mkdirSync(out)
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n', {flag: 'wx'})
const result = {schema: 'quod-managed-inherited-listener-probe-v1-research', passed: false,
  actualSdkAuthenticationClaimed: false, dispatcherConcurrencyEnabled: false, checks: [], cases: []}
const check = (name, pass) => {result.checks.push({name, pass: Boolean(pass)}); assert(pass, name)}
const errorData = error => ({name: String(error?.name || 'Error'), message: String(error?.message || error)})
const effect = data => ({checks: data.checks, random: data.random, virtualTime: data.boundary.virtualTime,
  logs: data.boundary.logs, unsupported: data.boundary.unsupported})
const closed = data => data.ok === true && data.checks.length === 31 && new Set(data.checks.map(row => row.name)).size === 31 &&
  data.checks.every(row => row.pass === true) && data.boundary.contract === MANAGED_BOUNDARY_CONTRACT &&
  data.boundary.intrinsicPolicy === MANAGED_INTRINSIC_POLICY && data.boundary.completion.intrinsicLockdown === true &&
  data.boundary.completion.reason === '' && data.boundary.completion.sealed === true && data.boundary.pendingTimers === 0 &&
  ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'].every(key => data.boundary.completion[key] === 0) &&
  ['unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors'].every(key => data.boundary[key].length === 0)
const rawWorker = body => 'export default {async fetch(request){if(new URL(request.url).pathname==="/health")return new Response("ok");' + body + '}};\n'
async function bindAndClose(port) {
  const server = net.createServer()
  try {
    await new Promise((resolve, reject) => {
      server.once('error', reject)
      server.listen({host: '127.0.0.1', port, exclusive: true, reusePort: false}, resolve)
    })
    assert.equal(server.address().port, port)
  } finally {await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()))}
}
function settled(name, succeeded) {
  const directory = path.join(out, name), execution = read(path.join(directory, 'execution.json'))
  const listener = read(path.join(directory, 'listener.json'))
  assert.equal(execution.completed, succeeded)
  assert.equal(listener.cleanupCompleted, true); assert.equal(listener.parentClosed, true)
  assert.equal(listener.parentAccepted, 0); assert.equal(execution.listenerCleanupCompleted, true)
  assert.equal(sha(fs.readFileSync(path.join(directory, 'control.bin'))), listener.controlSha256)
  if (succeeded) {
    assert(listener.ownershipVerified === true && listener.runRequests === 1 && listener.healthRequests > 0)
    assert(listener.parentClosedMonoMs <= listener.ownershipMonoMs && listener.ownershipMonoMs <= listener.firstHealthMonoMs &&
      listener.firstHealthMonoMs <= listener.runMonoMs && listener.runMonoMs < listener.shutdownMonoMs &&
      listener.spawnedMonoMs < listener.closedMonoMs)
    assert(listener.childOwnership.descriptors.every(fd => fd.ino === listener.lease.ino && fd.dev === listener.lease.dev))
    assert.deepEqual(listener.controlEvent, {event: 'listen', socket: 'http', port: listener.port})
    assert.equal(listener.controlTruncated, false)
  }
  result.cases.push({name, execution, listener})
  return listener
}

try {
  const line = Buffer.from('{"event":"listen","socket":"http","port":4567}\n')
  check('exact primary-release control event is accepted', parseInheritedListenControl(line, 4567).port === 4567)
  const invalid = [
    ['wrong port', line, 4568],
    ['duplicate key', Buffer.from('{"event":"listen","event":"listen","socket":"http","port":4567}\n'), 4567],
    ['extra field', Buffer.from('{"event":"listen","socket":"http","port":4567,"ready":true}\n'), 4567],
    ['extra event', Buffer.concat([line, line]), 4567],
    ['truncated line', line.subarray(0, line.length - 1), 4567],
    ['wrong socket', Buffer.from('{"event":"listen","socket":"other","port":4567}\n'), 4567],
    ['malformed UTF8', Buffer.from([255, 10]), 4567],
    ['oversized bytes', Buffer.alloc(4097), 4567],
    ['empty bytes', Buffer.alloc(0), 4567],
    ['nonprimitive port', line, '4567'],
  ]
  for (const [name, bytes, port] of invalid) {
    let denied = false
    try {parseInheritedListenControl(bytes, port)} catch {denied = true}
    check('exact control parser denies ' + name, denied)
  }
  process.env.TEST = 'true'; process.env.VITEST = 'true'; process.env.NODE_ENV ??= 'test'
  process.chdir(project)
  const compile = await createCertifiedManagedCompiler({project})
  const built = await compile('preflight.mjs', {epoch: 1700000000000, seed: 42, env: {NODE_ENV: 'test'}})
  write(path.join(out, 'preflight-build.json'), {...built, bundle: undefined})
  const ordinary = createManagedExecutor({out, watchdogMs: 30000})
  const before = await ordinary.execute('default-preflight', built.bundle)
  check('default path retains31 actual sealed preflight checks and no listener protocol field', closed(before) &&
    !Object.hasOwn(ordinary.runtimePolicy.supervisor, 'listenerProtocol'))
  let callbacks = 0
  const executor = createManagedExecutor({out, listenerMode: INHERITED_LISTENER_MODE, watchdogMs: 30000,
    onPhase(phase) {
      const directory = path.join(out, phase.name)
      assert.equal(read(path.join(directory, 'execution.json')).completed, true)
      assert.equal(read(path.join(directory, 'listener.json')).cleanupCompleted, true)
      callbacks++
    }})
  write(path.join(out, 'listener-policy.json'), executor.runtimePolicy)
  const success = await executor.execute('inherited-preflight', built.bundle)
  const listener = settled('inherited-preflight', true)
  check('actual inherited socket preserves default preflight effects and31 sealed checks', closed(success) &&
    JSON.stringify(effect(success)) === JSON.stringify(effect(before)))
  await bindAndClose(listener.port)
  check('successful inherited listener port is released before callback/result authority', callbacks === 1)
  const nonces = ['QUOD_LISTENER_NONCE_ONE', 'QUOD_LISTENER_NONCE_TWO']
  const attempts = await Promise.allSettled(['independent-one', 'independent-two'].map((name, index) => executor.execute(name,
    rawWorker('return new Response(JSON.stringify({nonce:' + JSON.stringify(nonces[index]) + '}));'))))
  // A failed peer never abandons the other listener/child cleanup operation.
  for (const attempt of attempts) if (attempt.status === 'rejected') throw attempt.reason
  const parallel = attempts.map(attempt => attempt.value)
  const pair = ['independent-one', 'independent-two'].map(name => settled(name, true))
  check('two actual independent inherited listeners return their own distinct nonce payloads', parallel.every((data, index) =>
    JSON.stringify(data) === JSON.stringify({nonce: nonces[index]})))
  check('simultaneous child lifetimes have distinct socket ports and inodes', pair[0].port !== pair[1].port &&
    pair[0].lease.ino !== pair[1].lease.ino && Math.max(...pair.map(row => row.spawnedMonoMs)) < Math.min(...pair.map(row => row.closedMonoMs)))
  for (const row of pair) await bindAndClose(row.port)
  check('both independent listeners are closed before all phase callbacks return', callbacks === 3)
  for (const [name, bundle, watchdogMs] of [
    ['malformed-module', 'export default {', 10000],
    ['request-failure', rawWorker('return new Response("deliberate failure",{status:503});'), 10000],
    ['request-watchdog', rawWorker('await new Promise(resolve=>setTimeout(resolve,60000));return new Response("late");'), 5000],
  ]) {
    let failure
    const failing = createManagedExecutor({out, listenerMode: INHERITED_LISTENER_MODE, watchdogMs,
      onPhase() {throw new Error('failed transport must never invoke phase callback')}})
    try {await failing.execute(name, bundle)} catch (error) {failure = errorData(error)}
    assert(failure)
    const evidence = settled(name, false), execution = read(path.join(out, name, 'execution.json'))
    await bindAndClose(evidence.port)
    check(name + ': actual failure awaits lease child and stream cleanup without a successful phase',
      name === 'malformed-module' ? evidence.healthRequests === 0 && evidence.runRequests === 0
        : evidence.ownershipVerified === true && evidence.runRequests === 1 &&
          (name === 'request-watchdog' ? execution.timedOut === true : failure.message.includes('HTTP 503')))
  }
  let callbackFailure
  const callback = createManagedExecutor({out, listenerMode: INHERITED_LISTENER_MODE, watchdogMs: 30000,
    onPhase() {throw new Error('deliberate post-cleanup callback failure')}})
  try {await callback.execute('callback-failure', built.bundle)} catch (error) {callbackFailure = error}
  const callbackEvidence = settled('callback-failure', false)
  await bindAndClose(callbackEvidence.port)
  check('post-cleanup callback failure cannot leave completed execution authority',
    callbackFailure?.message === 'deliberate post-cleanup callback failure' && read(path.join(out, 'callback-failure/execution.json')).phaseCallbackFailed === true)
  const require = createRequire(import.meta.url)
  const binary = fs.realpathSync(path.join(path.dirname(require.resolve('workerd/package.json')), 'bin/workerd'))
  const fd = fs.openSync('/dev/null', 'r')
  let wrong
  try {wrong = spawnSync(binary, ['serve', path.join(out, 'not-a-config.capnp'), '--socket-fd', 'http=3'],
    {stdio: ['ignore', 'pipe', 'pipe', fd], timeout: 5000, killSignal: 'SIGKILL', maxBuffer: 16384,
      env: {PATH: process.env.PATH, TZ: 'UTC', LANG: 'C.UTF-8'}})}
  finally {fs.closeSync(fd)}
  fs.writeFileSync(path.join(out, 'wrong-fd-stdout.log'), wrong.stdout || Buffer.alloc(0), {flag: 'wx'})
  fs.writeFileSync(path.join(out, 'wrong-fd-stderr.log'), wrong.stderr || Buffer.alloc(0), {flag: 'wx'})
  check('exact packaged workerd rejects actual inherited regular-file descriptor before config loading',
    !wrong.error && wrong.status === 1 && wrong.signal === null && wrong.stderr.toString('utf8').includes('not a socket'))
  assert.equal(result.checks.length, 22)
  assert.deepEqual(result.cases.map(row => row.name), ['inherited-preflight', 'independent-one', 'independent-two',
    'malformed-module', 'request-failure', 'request-watchdog', 'callback-failure'])
  result.passed = true
} catch (error) {result.error = errorData(error); process.exitCode = 1}
finally {
  result.checkCount = result.checks.length; result.expectedCheckCount = 22
  write(path.join(out, 'result.json'), result)
}
