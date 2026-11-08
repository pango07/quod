// Host-side research supervisor. A timeout or startup/transport failure throws;
// it never manufactures a successful worker receipt or grants replay admission.
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import net from 'node:net'
import {createRequire} from 'node:module'
import {fileURLToPath} from 'node:url'
import {spawn} from 'node:child_process'
import {MANAGED_BOUNDARY_CONTRACT, MANAGED_INTRINSIC_POLICY} from './receipt.mjs'
import {INHERITED_LISTENER_MODE, inheritedListenerPolicy, executeInheritedManaged} from './inherited-listener-executor.mjs'

const require = createRequire(import.meta.url)
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const denyModule = "export default {fetch(){console.error('QUOD_DENIED_OUTBOUND');throw new Error('outbound denied')}};\n"
const compatibilityDate = '2026-07-01', compatibilityFlags = ['nodejs_als']
const startupAttempts = 80, healthTimeoutMs = 500, startupRetryMs = 100
const shutdownGraceMs = 1000, responseBytesMax = 16 * 1024 * 1024
const completionBounds = {nativeTurnFencesMax: 512, timerCallbackBudget: 4096,
  timerCallbackBudgetScope: 'callbacks after completion begins; reject when exceeded', quietNativeTurns: 2}

function frozenData(value) {
  if (value && typeof value === 'object') {
    for (const child of Object.values(value)) frozenData(child)
    Object.freeze(value)
  }
  return value
}
function errorData(error) {
  return {name: String(error?.name || 'Error'), message: String(error?.message || error),
    code: error?.code === undefined ? null : String(error.code)}
}
function hostIdentity() {
  const header = process.report?.getReport().header || {}
  return frozenData({
    ImageOS: process.env.ImageOS || null,
    ImageVersion: process.env.ImageVersion || null,
    platform: process.platform,
    arch: process.arch,
    kernel: {release: os.release(), version: os.version()},
    libc: {family: header.glibcVersionRuntime ? 'glibc' : process.platform === 'darwin' ? 'libSystem' : 'unknown',
      runtimeVersion: header.glibcVersionRuntime || null, compilerVersion: header.glibcVersionCompiler || null},
    nodeVersion: process.version,
  })
}
async function freePort() {
  const server = net.createServer()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const port = server.address().port
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()))
  return port
}
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds))
async function responseBytes(response) {
  const chunks = []
  let length = 0
  for await (const chunk of response.body || []) {
    length += chunk.byteLength
    if (length > responseBytesMax) throw new Error('workerd response exceeds the supervisor byte limit')
    chunks.push(Buffer.from(chunk))
  }
  return Buffer.concat(chunks, length)
}

/**
 * out must be an existing explicit directory. onPhase receives the original
 * probe row only after the response, child termination and watchdog checks pass.
 * Both returned identity objects contain data only and are deeply immutable.
 */
export function createManagedExecutor({out: outArg, onPhase, watchdogMs = 180000, listenerMode}) {
  assert(typeof outArg === 'string' && outArg.length, 'an explicit output directory is required')
  const out = fs.realpathSync(path.resolve(outArg))
  assert(fs.statSync(out).isDirectory(), 'executor output directory must already exist')
  assert(onPhase === undefined || typeof onPhase === 'function', 'onPhase must be a function')
  assert(Number.isSafeInteger(watchdogMs) && watchdogMs > 0 && watchdogMs <= 2147483647,
    'watchdogMs must fit a positive native timer')
  assert(typeof process.env.PATH === 'string', 'an explicit child PATH is required')
  assert(listenerMode === undefined || listenerMode === INHERITED_LISTENER_MODE, 'unknown managed listener mode')
  const workerd = fs.realpathSync(path.join(path.dirname(require.resolve('workerd/package.json')), 'bin/workerd'))
  const processEnv = {PATH: process.env.PATH, TZ: 'UTC', LANG: 'C.UTF-8'}
  const host = hostIdentity()
  const runtimePolicy = frozenData({
    schema: 'quod-managed-runtime-policy-v1-research',
    boundaryContract: MANAGED_BOUNDARY_CONTRACT,
    intrinsicPolicy: MANAGED_INTRINSIC_POLICY,
    workerdSha256: sha(fs.readFileSync(workerd)),
    moduleName: 'worker.mjs',
    services: ['main', 'deny'],
    compatibilityDate,
    compatibilityFlags: [...compatibilityFlags],
    globalOutbound: 'deny',
    denyModuleSha256: sha(denyModule),
    bindings: [],
    processEnv,
    request: {method: 'GET', path: '/run'},
    supervisor: {
      sourceSha256: sha(fs.readFileSync(fileURLToPath(import.meta.url))),
      nodeVersion: process.version,
      nodeExecutableSha256: sha(fs.readFileSync(process.execPath)),
      watchdogMs,
      startupAttempts,
      healthTimeoutMs,
      startupRetryMs,
      shutdownGraceMs,
      responseBytesMax,
      completionBounds: {...completionBounds},
      ...(listenerMode === undefined ? {} : {listenerProtocol: inheritedListenerPolicy({
        packageVersion: require('workerd/package.json').version, workerdSha256: sha(fs.readFileSync(workerd))})}),
    },
  })

  async function execute(name, bundle) {
    if (listenerMode === INHERITED_LISTENER_MODE) return executeInheritedManaged({name, bundle, out, workerd,
      runtimePolicy, host, onPhase, denyModule, compatibilityDate, compatibilityFlags, startupAttempts,
      healthTimeoutMs, startupRetryMs, shutdownGraceMs, responseBytesMax, watchdogMs})
    assert(typeof name === 'string' && /^[a-z][a-z0-9-]*$/.test(name), 'invalid managed phase name')
    assert(typeof bundle === 'string' || Buffer.isBuffer(bundle), 'managed bundle must be text or bytes')
    assert.equal(sha(fs.readFileSync(workerd)), runtimePolicy.workerdSha256, 'workerd binary changed after policy creation')
    const dir = path.join(out, name)
    fs.mkdirSync(dir)
    const write = (file, value) => fs.writeFileSync(path.join(dir, file), JSON.stringify(value, null, 2) + '\n')
    fs.writeFileSync(path.join(dir, 'worker.mjs'), bundle)
    fs.writeFileSync(path.join(dir, 'deny.mjs'), denyModule)
    const port = await freePort()
    const configFile = path.join(dir, 'config.capnp')
    const config = `using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
  services = [
    (name = "main", worker = (modules = [(name = "worker.mjs", esModule = embed "worker.mjs")],
      compatibilityDate = "${compatibilityDate}", compatibilityFlags = [${compatibilityFlags.map(value => JSON.stringify(value)).join(', ')}], globalOutbound = "deny")),
    (name = "deny", worker = (modules = [(name = "deny.mjs", esModule = embed "deny.mjs")],
      compatibilityDate = "${compatibilityDate}", globalOutbound = "deny"))
  ],
  sockets = [(name = "http", address = "127.0.0.1:${port}", http = (), service = "main")]
);\n`
    fs.writeFileSync(configFile, config)
    const args = ['serve', configFile], url = `http://127.0.0.1:${port}/run`
    const invocation = {schema: 'quod-managed-invocation-v1-research', name, executable: workerd, args,
      cwd: process.cwd(), env: processEnv, configSha256: sha(config), bundleSha256: sha(bundle),
      request: {method: 'GET', url}, runtimePolicy, host}
    write('invocation.json', invocation)

    const log = fs.openSync(path.join(dir, 'runtime.log'), 'w'), started = performance.now()
    const controller = new AbortController()
    let child, closed = false, exit, spawnError, timedOut = false, failure, data, row, receivedResponseSha256
    let shutdownRequested = false, unexpectedExit = false
    let resolveClosed
    const childClosed = new Promise(resolve => { resolveClosed = resolve })
    const watchdog = setTimeout(() => {
      timedOut = true
      controller.abort(new Error('managed workerd watchdog exhausted'))
      if (child && !closed) child.kill('SIGKILL')
    }, watchdogMs)
    try {
      child = spawn(workerd, args, {stdio: ['ignore', log, log], env: processEnv})
      invocation.pid = child.pid || null
      write('invocation.json', invocation)
      child.once('error', error => { spawnError = error })
      child.once('exit', () => { unexpectedExit ||= !shutdownRequested && !timedOut })
      child.once('close', (code, signal) => {
        closed = true
        unexpectedExit ||= !shutdownRequested && !timedOut
        exit = {code, signal, spawnError: spawnError ? errorData(spawnError) : null}
        resolveClosed(exit)
      })
      let ready = false
      for (let index = 0; index < startupAttempts && !closed && !spawnError && !timedOut; index++) {
        try {
          const signal = AbortSignal.any([controller.signal, AbortSignal.timeout(healthTimeoutMs)])
          const response = await fetch(`http://127.0.0.1:${port}/health`, {method: 'GET', signal})
          ready = response.ok
          await response.body?.cancel()
        } catch {}
        if (ready) break
        await Promise.race([delay(startupRetryMs), childClosed])
      }
      if (spawnError) throw spawnError
      assert(!timedOut, 'managed workerd watchdog exhausted during startup')
      assert(ready && !closed, 'workerd did not become ready; inspect runtime.log')
      const response = await fetch(url, {method: 'GET', signal: controller.signal})
      const bytes = await responseBytes(response)
      receivedResponseSha256 = sha(bytes)
      fs.writeFileSync(path.join(dir, 'response.json'), bytes)
      write('response-meta.json', {status: response.status, statusText: response.statusText,
        headers: Object.fromEntries(response.headers), bytes: bytes.length, sha256: sha(bytes)})
      assert(response.ok, `workerd returned HTTP ${response.status}`)
      data = JSON.parse(new TextDecoder('utf-8', {fatal: true}).decode(bytes))
      assert(!timedOut, 'managed workerd watchdog exhausted before response validation')
      row = {name, seconds: (performance.now() - started) / 1000, responseSha256: sha(bytes), data}
    } catch (error) {
      failure = error
    } finally {
      if (child && !closed) {
        shutdownRequested = true
        try { child.kill('SIGTERM') } catch (error) { failure ||= error }
        await Promise.race([childClosed, delay(shutdownGraceMs)])
        if (!closed) {
          try { child.kill('SIGKILL') } catch (error) { failure ||= error }
          await Promise.race([childClosed, delay(shutdownGraceMs)])
        }
        if (!closed && !failure) failure = new Error('workerd did not terminate within supervisor cleanup bounds')
      }
      clearTimeout(watchdog)
      fs.closeSync(log)
    }
    if (timedOut) failure = new Error('managed workerd watchdog exhausted; execution is incomplete')
    else if (spawnError) failure ||= spawnError
    else if (unexpectedExit) failure ||= new Error('workerd exited before supervisor shutdown; execution is incomplete')
    const execution = {schema: 'quod-managed-execution-v1-research', name,
      completed: !failure, timedOut, seconds: (performance.now() - started) / 1000,
      exit: exit || null, error: failure ? errorData(failure) : null,
      responseSha256: receivedResponseSha256 || null}
    write('execution.json', execution)
    if (failure) throw failure
    try {
      if (onPhase) await onPhase(row)
    } catch (error) {
      write('execution.json', {...execution, completed: false, error: errorData(error), phaseCallbackFailed: true})
      throw error
    }
    return data
  }

  return Object.freeze({execute, runtimePolicy, host})
}
