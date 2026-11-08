// Opt-in finite Linux supervisor only. No native test/worker overlap is enabled.
// Exact workerd release sources: cloudflare/workerd@359d773d14642106efbceeeaf8c85709440ec626
// cli/socket_fd/unix.rs validates+duplicates; server/run.rs writes exactly the
// listen event below. Without --watch the original childFD3 can be dropped;
// ownership is verified by the retained socket inode, never by a fixed childFD.
import fs from 'node:fs'
import path from 'node:path'
import net from 'node:net'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {spawn} from 'node:child_process'
import {fileURLToPath} from 'node:url'

export const INHERITED_LISTENER_MODE = 'inherited-linux-fd3-control4-v1-research'
const workerdVersion = '1.20261107.1'
const reviewedWorkerdSHA = 'a4d954a9ec17a367cf7d5509a2282785e84dacca6fc13b69c532ab25d157ef48'
const limits = Object.freeze({leaseTimeoutMs: 8000, ownershipTimeoutMs: 8000,
  controlBytesMax: 4096, controlEventsMax: 1, procEntriesMax: 256,
  nativeLogBytesMax: 16 * 1024 * 1024})
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const errors = error => ({name: String(error?.name || 'Error'), message: String(error?.message || error),
  code: error?.code === undefined ? null : String(error.code)})
const statIdentity = stat => ({dev: String(stat.dev), ino: String(stat.ino), size: String(stat.size)})
const sameFile = (left, right) => ['dev', 'ino', 'size'].every(key => left[key] === right[key])
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))

export function inheritedListenerPolicy({packageVersion, workerdSha256}) {
  assert.equal(process.platform, 'linux', 'inherited listener requires the finite Linux profile')
  assert.equal(process.arch, 'x64', 'inherited listener requires the reviewed Linux x64 binary')
  assert.equal(process.version, 'v24.18.0', 'inherited listener requires reviewed Node24.18.0')
  assert.equal(process.execArgv.length, 0, 'inherited listener requires no Node CLI flags')
  assert(!process.env.NODE_OPTIONS, 'inherited listener requires no Node preload/options')
  assert.equal(packageVersion, workerdVersion, 'inherited listener workerd package differs')
  assert.equal(workerdSha256, reviewedWorkerdSHA, 'inherited listener workerd bytes differ')
  return Object.freeze({protocol: INHERITED_LISTENER_MODE,
    helperSourceSha256: sha(fs.readFileSync(fileURLToPath(import.meta.url))),
    platform: 'linux', arch: 'x64', nodeVersion: 'v24.18.0', workerdVersion,
    workerdSha256: reviewedWorkerdSHA,
    sourceCommit: '359d773d14642106efbceeeaf8c85709440ec626',
    address: '127.0.0.1', socket: 'http', inheritedFd: 3, controlFd: 4,
    ownership: 'leased socket dev/inode in bounded child procfs scan plus executable/liveness and exact private control event',
    handoff: 'spawn then parent server.close synchronously before any await or request',
    expectedShutdown: 'exit code0 or SIGTERM only; every other status fails',
    cleanup: 'TERM then KILL; grace overruns fail sticky; always await actual child and stream closure', ...limits})
}

/** Pure bounded parser for the one exact release-written control line. */
export function parseInheritedListenControl(bytes, port) {
  assert(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length <= limits.controlBytesMax,
    'bounded inherited listener control bytes required')
  assert(Number.isInteger(port) && port > 0 && port <= 65535, 'valid leased port required')
  const expected = {event: 'listen', socket: 'http', port}
  // Exact serialization also rejects duplicate keys, extra fields/events,
  // alternate whitespace, malformed UTF-8, truncation and a spoofed port.
  assert(bytes.equals(Buffer.from(JSON.stringify(expected) + '\n')), 'inherited listener control differs')
  return Object.freeze(expected)
}

function socketInodes(pid) {
  const directory = `/proc/${pid}/fd`, entries = fs.readdirSync(directory)
  assert(entries.length <= limits.procEntriesMax, 'inherited listener proc descriptor bound exceeded')
  assert(entries.every(name => /^(0|[1-9][0-9]*)$/.test(name) && Number.isSafeInteger(Number(name))),
    'inherited listener proc descriptor name differs')
  const sockets = []
  for (const fd of entries) {
    let link
    try {link = fs.readlinkSync(path.join(directory, fd))}
    catch (error) {if (error.code === 'ENOENT') continue; throw error}
    const match = /^socket:\[([1-9][0-9]*)\]$/.exec(link)
    if (match) {
      let stat
      try {stat = fs.statSync(path.join(directory, fd), {bigint: true})}
      catch (error) {if (error.code === 'ENOENT') continue; throw error}
      assert(stat.isSocket() && String(stat.ino) === match[1], 'inherited listener proc socket changed during scan')
      sockets.push({fd: Number(fd), dev: String(stat.dev), ino: String(stat.ino)})
    }
  }
  return {entries: entries.length, sockets}
}

/** Separate implementation so the unopted executor path remains unchanged. */
export async function executeInheritedManaged({name, bundle, out, workerd, runtimePolicy, host, onPhase,
  denyModule, compatibilityDate, compatibilityFlags, startupAttempts, healthTimeoutMs,
  startupRetryMs, shutdownGraceMs, responseBytesMax, watchdogMs}) {
  assert(typeof name === 'string' && /^[a-z][a-z0-9-]*$/.test(name), 'invalid managed phase name')
  assert(typeof bundle === 'string' || Buffer.isBuffer(bundle), 'managed bundle must be text or bytes')
  assert.equal(sha(fs.readFileSync(workerd)), runtimePolicy.workerdSha256, 'workerd binary changed after policy creation')
  assert.equal(runtimePolicy.supervisor.listenerProtocol.protocol, INHERITED_LISTENER_MODE)
  const executableIdentity = statIdentity(fs.statSync(workerd, {bigint: true}))
  const dir = path.join(out, name); fs.mkdirSync(dir)
  const write = (file, data) => fs.writeFileSync(path.join(dir, file), JSON.stringify(data, null, 2) + '\n')
  fs.writeFileSync(path.join(dir, 'worker.mjs'), bundle)
  fs.writeFileSync(path.join(dir, 'deny.mjs'), denyModule)
  const configFile = path.join(dir, 'config.capnp')
  const config = `using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
  services = [
    (name = "main", worker = (modules = [(name = "worker.mjs", esModule = embed "worker.mjs")],
      compatibilityDate = "${compatibilityDate}", compatibilityFlags = [${compatibilityFlags.map(value => JSON.stringify(value)).join(', ')}], globalOutbound = "deny")),
    (name = "deny", worker = (modules = [(name = "deny.mjs", esModule = embed "deny.mjs")],
      compatibilityDate = "${compatibilityDate}", globalOutbound = "deny"))
  ],
  sockets = [(name = "http", http = (), service = "main")]
);\n`
  fs.writeFileSync(configFile, config)
  const args = ['serve', configFile, '--socket-fd', 'http=3', '--control-fd', '4']
  const evidence = {schema: 'quod-managed-inherited-listener-v1-research', protocol: INHERITED_LISTENER_MODE,
    acquired: false, parentClosed: false, parentAccepted: 0, controlBytes: 0, controlEvent: null,
    ownershipVerified: false, cleanupCompleted: false, healthRequests: 0, runRequests: 0}
  const invocation = {schema: 'quod-managed-invocation-v1-research', name, executable: workerd, args,
    cwd: process.cwd(), env: runtimePolicy.processEnv, configSha256: sha(config), bundleSha256: sha(bundle),
    request: {method: 'GET', url: null}, runtimePolicy, host,
    stdio: ['ignore', 'bounded-log-pipe', 'bounded-log-pipe', 'leased-listener', 'private-control-pipe']}
  write('invocation.json', invocation)
  const log = fs.openSync(path.join(dir, 'runtime.log'), 'wx'), started = performance.now()
  const controller = new AbortController()
  let child, lease, heldHandle, leaseClosing = false, leaseClosed = false, closed = false, exit, spawnError
  let timedOut = false, failure, shutdownRequested = false, unexpectedExit = false, data, row, receivedResponseSha256
  let controlBuffer = Buffer.alloc(0), logBytes = 0
  let resolveChild, resolveLease
  const childClosed = new Promise(resolve => {resolveChild = resolve})
  const leaseClosedPromise = new Promise(resolve => {resolveLease = resolve})
  const streamClosures = [], parentSocketClosures = [], parentSockets = new Set()
  const fail = error => {failure ||= error; controller.abort(error)}
  const alive = () => child && !closed && !spawnError && child.exitCode === null && child.signalCode === null
  function closeLease() {
    if (!lease || leaseClosing) return
    leaseClosing = true
    // Pinned Node24 Server.close also schedules close for an unbound server:
    // _emitCloseIfDrained runs even after listen errors/timeouts, and the
    // listeningId change prevents a pending lookup from binding afterwards.
    try {lease.close(error => {if (error && error.code !== 'ERR_SERVER_NOT_RUNNING') fail(error)})}
    catch (error) {fail(error)}
  }
  function observeStream(stream, receive) {
    assert(stream && typeof stream.on === 'function', 'inherited listener child stream unavailable')
    streamClosures.push(new Promise(resolve => stream.once('close', resolve)))
    stream.on('error', fail)
    stream.on('data', bytes => {try {receive(bytes)} catch (error) {fail(error)}})
  }
  const watchdog = setTimeout(() => {
    timedOut = true; fail(new Error('managed workerd watchdog exhausted'))
    if (alive()) {try {child.kill('SIGKILL')} catch (error) {fail(error)}}
    closeLease()
  }, watchdogMs)
  try {
    lease = net.createServer(socket => {
      evidence.parentAccepted++; parentSockets.add(socket)
      parentSocketClosures.push(new Promise(resolve => socket.once('close', () => {parentSockets.delete(socket); resolve()})))
      socket.on('error', fail); socket.destroy()
      fail(new Error('parent accepted an inherited listener connection'))
    })
    lease.once('close', () => {leaseClosed = true; evidence.parentClosed = true; resolveLease()})
    lease.on('error', fail)
    let leaseTimer, listenError
    try {
      await new Promise((resolve, reject) => {
        leaseTimer = setTimeout(() => reject(new Error('inherited listener lease timed out')), limits.leaseTimeoutMs)
        listenError = reject; lease.once('error', listenError)
        lease.listen({host: '127.0.0.1', port: 0, exclusive: true, reusePort: false}, resolve)
      })
    } finally {
      clearTimeout(leaseTimer)
      if (listenError) lease.removeListener('error', listenError)
    }
    assert(!failure && !timedOut && evidence.parentAccepted === 0, 'inherited listener lease was not exclusive')
    const address = lease.address()
    assert(address && address.address === '127.0.0.1' && address.family === 'IPv4' && Number.isInteger(address.port) && address.port > 0)
    heldHandle = lease._handle
    const fd = heldHandle?.fd
    assert(Number.isSafeInteger(fd) && fd > 2, 'reviewed Node listener descriptor unavailable')
    const stat = fs.fstatSync(fd, {bigint: true})
    assert(stat.isSocket() && stat.ino > 0n, 'inherited lease descriptor is not an actual socket')
    evidence.acquired = true; evidence.port = address.port
    evidence.lease = {fd, dev: String(stat.dev), ino: String(stat.ino)}
    // CRITICAL: no await, HTTP call, callback, or unrelated spawn between the
    // child spawn and parent close. Pinned Node/Linux dup/exec completes inside
    // spawn; server.close synchronously removes poll events and closes our FD.
    try {
      child = spawn(workerd, args, {stdio: ['ignore', 'pipe', 'pipe', fd, 'pipe'], env: runtimePolicy.processEnv})
      evidence.spawnedMonoMs = performance.now()
    } finally {closeLease()}
    child.once('error', error => {spawnError = error; fail(error)})
    child.once('exit', () => {unexpectedExit ||= !shutdownRequested && !timedOut})
    child.once('close', (code, signal) => {
      closed = true; unexpectedExit ||= !shutdownRequested && !timedOut
      evidence.closedMonoMs = performance.now()
      exit = {code, signal, spawnError: spawnError ? errors(spawnError) : null}; resolveChild()
    })
    const receiveLog = bytes => {
      const remaining = limits.nativeLogBytesMax - logBytes
      const retained = bytes.subarray(0, Math.max(0, remaining))
      for (let offset = 0; offset < retained.length;) {
        const written = fs.writeSync(log, retained, offset, retained.length - offset)
        assert(written > 0, 'workerd native log write made no progress'); offset += written
      }
      logBytes += bytes.length
      assert(logBytes <= limits.nativeLogBytesMax, 'workerd native log byte bound exceeded')
    }
    observeStream(child.stdout, receiveLog); observeStream(child.stderr, receiveLog)
    observeStream(child.stdio[4], bytes => {
      evidence.controlBytes += bytes.length
      controlBuffer = Buffer.concat([controlBuffer, bytes.subarray(0, limits.controlBytesMax - controlBuffer.length)])
      assert(evidence.controlBytes <= limits.controlBytesMax, 'inherited listener control byte bound exceeded')
      assert(evidence.controlEvent === null, 'extra inherited listener control data')
      if (controlBuffer.includes(10)) evidence.controlEvent = parseInheritedListenControl(controlBuffer, address.port)
    })
    child.stdio[4].on('end', () => {
      if (!shutdownRequested && !closed) fail(new Error(evidence.controlEvent
        ? 'inherited listener control ended before shutdown' : 'truncated inherited listener control'))
    })
    invocation.pid = child.pid || null; invocation.request.url = `http://127.0.0.1:${address.port}/run`
    write('invocation.json', invocation)
    await leaseClosedPromise
    evidence.parentClosedMonoMs = performance.now()
    assert(heldHandle.fd === -1 && !lease.listening && lease.address() === null && evidence.parentAccepted === 0,
      'parent inherited listener was not closed before ownership verification')
    const parentScan = socketInodes('self')
    assert(!parentScan.sockets.some(socket => socket.ino === evidence.lease.ino && socket.dev === evidence.lease.dev),
      'parent retained an inherited listening socket')
    evidence.parentProcEntries = parentScan.entries
    const ownershipStarted = performance.now()
    while (alive() && !failure && !timedOut && performance.now() - ownershipStarted <= limits.ownershipTimeoutMs) {
      try {
        const executable = fs.statSync(`/proc/${child.pid}/exe`, {bigint: true})
        assert(executable.isFile() && sameFile(statIdentity(executable), executableIdentity), 'inherited listener child executable differs')
        const scan = socketInodes(child.pid)
        const owned = scan.sockets.filter(socket => socket.ino === evidence.lease.ino && socket.dev === evidence.lease.dev)
        if (owned.length && evidence.controlEvent) {
          evidence.childOwnership = {pid: child.pid, executable: statIdentity(executable), procEntries: scan.entries, descriptors: owned}
          evidence.ownershipVerified = true; evidence.ownershipMonoMs = performance.now(); break
        }
      } catch (error) {
        if (error.code !== 'ENOENT' && error.code !== 'ESRCH') throw error
      }
      await pause(startupRetryMs)
    }
    if (failure) throw failure
    assert(alive() && !timedOut && evidence.ownershipVerified, 'workerd inherited listener ownership was not verified')
    write('listener.json', evidence)
    let ready = false
    for (let index = 0; index < startupAttempts && alive() && !failure && !timedOut; index++) {
      try {
        const signal = AbortSignal.any([controller.signal, AbortSignal.timeout(healthTimeoutMs)])
        evidence.firstHealthMonoMs ??= performance.now()
        evidence.healthRequests++
        const response = await fetch(`http://127.0.0.1:${address.port}/health`, {method: 'GET', signal})
        ready = response.ok; await response.body?.cancel()
      } catch {}
      if (ready) break
      await pause(startupRetryMs)
    }
    if (failure) throw failure
    assert(ready && alive() && !timedOut && evidence.parentAccepted === 0, 'inherited workerd did not become ready')
    evidence.runMonoMs = performance.now(); evidence.runRequests++
    const response = await fetch(invocation.request.url, {method: 'GET', signal: controller.signal})
    const chunks = []; let length = 0
    for await (const bytes of response.body || []) {
      length += bytes.byteLength; assert(length <= responseBytesMax, 'workerd response exceeds the supervisor byte limit')
      chunks.push(Buffer.from(bytes))
    }
    const bytes = Buffer.concat(chunks, length); receivedResponseSha256 = sha(bytes)
    fs.writeFileSync(path.join(dir, 'response.json'), bytes)
    write('response-meta.json', {status: response.status, statusText: response.statusText,
      headers: Object.fromEntries(response.headers), bytes: bytes.length, sha256: receivedResponseSha256})
    assert(response.ok, `workerd returned HTTP ${response.status}`)
    data = JSON.parse(new TextDecoder('utf-8', {fatal: true}).decode(bytes))
    assert(!failure && !timedOut && alive(), 'inherited workerd failed before response validation')
    row = {name, seconds: (performance.now() - started) / 1000, responseSha256: receivedResponseSha256, data}
  } catch (error) {failure ||= error}
  finally {
    shutdownRequested = true; evidence.shutdownMonoMs = performance.now(); closeLease()
    for (const socket of parentSockets) socket.destroy()
    if (child && !closed) {
      try {child.kill('SIGTERM')} catch (error) {failure ||= error}
      const termStarted = performance.now()
      while (!closed && performance.now() - termStarted < shutdownGraceMs) await pause(10)
      if (!closed) {
        failure ||= new Error('inherited workerd exceeded TERM cleanup grace')
        try {child.kill('SIGKILL')} catch (error) {failure ||= error}
        const killStarted = performance.now()
        while (!closed && performance.now() - killStarted < shutdownGraceMs) await pause(10)
        if (!closed) failure ||= new Error('inherited workerd exceeded KILL cleanup grace')
        // Never return/issue a phase result while a spawned child remains live.
        await childClosed
      }
    }
    if (lease) await leaseClosedPromise
    await Promise.all(parentSocketClosures)
    await Promise.all(streamClosures)
    clearTimeout(watchdog)
    fs.closeSync(log)
    evidence.cleanupCompleted = (!child || closed) && (!lease || leaseClosed) && parentSockets.size === 0
    evidence.logBytes = logBytes; evidence.controlSha256 = sha(controlBuffer)
    evidence.controlTruncated = evidence.controlBytes > controlBuffer.length
    if (!evidence.cleanupCompleted) failure ||= new Error('inherited listener cleanup is incomplete')
    fs.writeFileSync(path.join(dir, 'control.bin'), controlBuffer)
    write('listener.json', evidence)
  }
  if (timedOut) failure = new Error('managed workerd watchdog exhausted; execution is incomplete')
  else if (spawnError) failure ||= spawnError
  else if (unexpectedExit) failure ||= new Error('workerd exited before supervisor shutdown; execution is incomplete')
  else if (child && !(exit?.code === 0 && exit.signal === null || exit?.code === null && exit.signal === 'SIGTERM')) {
    failure ||= new Error('inherited workerd shutdown status differs')
  }
  const execution = {schema: 'quod-managed-execution-v1-research', name, completed: !failure, timedOut,
    seconds: (performance.now() - started) / 1000, exit: exit || null,
    error: failure ? errors(failure) : null, responseSha256: receivedResponseSha256 || null,
    listenerProtocol: INHERITED_LISTENER_MODE, listenerOwnershipVerified: evidence.ownershipVerified,
    listenerCleanupCompleted: evidence.cleanupCompleted}
  write('execution.json', execution)
  if (failure) throw failure
  try {if (onPhase) await onPhase(row)}
  catch (error) {write('execution.json', {...execution, completed: false, error: errors(error), phaseCallbackFailed: true}); throw error}
  return data
}
