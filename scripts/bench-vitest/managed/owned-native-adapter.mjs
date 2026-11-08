// Diagnostic opt-in only. The strict wrapper spawns the unchanged native child;
// this module imports builtins only and never evaluates project code in parent.
// Owned Linux process groups close the reviewed child's ordinary descendants.
// This is a trusted-host, finite Vitest profile, not a sandbox against a hostile
// child deliberately escaping setsid/ancestry between sampled procfs scans.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {spawn} from 'node:child_process'
import {fileURLToPath} from 'node:url'

const self = fileURLToPath(import.meta.url)
const nativeChild = path.join(path.dirname(self), 'matrix-native.mjs')
const reviewedChildSha256 = 'ed1b6a2e6fac1ad5abd350c8dbb8528377f6b8e44d67f4b0bffa89de7011cdfe'
export const OWNED_NATIVE_MODE = 'linux-process-group-native-v1-research'
const limits = Object.freeze({watchdogMsMax: 300000, termGraceMs: 1000, killGraceMs: 1000,
  procPollMs: 100, procEntriesMax: 8192, procStatBytesMax: 65536, trackedProcessesMax: 512,
  logBytesMax: 16 * 1024 * 1024, requestBytesMax: 4 * 1024 * 1024, evidenceBytesMax: 32 * 1024 * 1024})
const authKeys = ['ACTIONS_RUNTIME_TOKEN', 'ACTIONS_CACHE_URL', 'ACTIONS_RESULTS_URL',
  'ACTIONS_CACHE_SERVICE_V2', 'ACTIONS_RUNTIME_URL']
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const write = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', {flag: 'wx'})
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const errorData = error => ({name: String(error?.name || 'Error').slice(0, 128),
  message: String(error?.message || error).slice(0, 2048),
  code: error?.code === undefined ? null : String(error.code).slice(0, 128)})
const fileIdentity = stat => ({dev: String(stat.dev), ino: String(stat.ino), size: String(stat.size)})
const sameIdentity = (left, right) => ['dev', 'ino', 'size'].every(key => left[key] === right[key])
const gone = error => error.code === 'ENOENT' || error.code === 'ESRCH'

function regularBytes(file, max) {
  const before = fs.lstatSync(file, {bigint: true})
  assert(before.isFile() && before.size <= BigInt(max), 'bounded regular native evidence required')
  const fd = fs.openSync(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW)
  try {
    const opened = fs.fstatSync(fd, {bigint: true})
    assert(sameIdentity(fileIdentity(before), fileIdentity(opened)), 'native evidence changed before open')
    const bytes = fs.readFileSync(fd), after = fs.fstatSync(fd, {bigint: true})
    assert(bytes.length <= max && sameIdentity(fileIdentity(opened), fileIdentity(after)) &&
      opened.mtimeNs === after.mtimeNs && opened.ctimeNs === after.ctimeNs && BigInt(bytes.length) === after.size,
    'native evidence changed while reading')
    return bytes
  } finally {fs.closeSync(fd)}
}

function procRow(pid) {
  let text
  try {text = fs.readFileSync(`/proc/${pid}/stat`, 'utf8')}
  catch (error) {if (gone(error)) return null; throw error}
  assert(Buffer.byteLength(text) <= limits.procStatBytesMax, 'native proc stat exceeds bound')
  const end = text.lastIndexOf(') '), prefix = text.slice(0, text.indexOf(' '))
  assert(end > 0 && prefix === String(pid), 'native proc stat identity differs')
  const fields = text.slice(end + 2).trim().split(/\s+/)
  assert(fields.length >= 20 && /^[A-Za-z]$/.test(fields[0]), 'native proc stat fields differ')
  for (const index of [1, 2, 3, 19]) assert(/^(0|[1-9][0-9]*)$/.test(fields[index]), 'invalid native proc identity field')
  const row = {pid: Number(pid), state: fields[0], ppid: Number(fields[1]), pgid: Number(fields[2]),
    session: Number(fields[3]), startTicks: fields[19]}
  assert([row.pid, row.ppid, row.pgid, row.session].every(Number.isSafeInteger), 'native proc number exceeds bound')
  return row
}

function procRows() {
  const entries = fs.readdirSync('/proc').filter(name => /^[1-9][0-9]*$/.test(name))
  assert(entries.length <= limits.procEntriesMax, 'native proc process bound exceeded')
  const rows = []
  for (const entry of entries) {
    const pid = Number(entry)
    assert(Number.isSafeInteger(pid), 'native proc PID exceeds bound')
    const row = procRow(pid)
    if (row) rows.push(row)
  }
  return rows
}

// This generic surface is for tiny remote ownership negatives only. It grants
// no managed receipt policy; only the finite wrapper below may feed conformance.
export async function runOwnedNativeProcess({executable: executableArg, args, cwd: cwdArg, env,
  logFile: logFileArg, watchdogMs = limits.watchdogMsMax, termGraceMs = limits.termGraceMs,
  killGraceMs = limits.killGraceMs, logBytesMax = limits.logBytesMax}) {
  assert.equal(process.platform, 'linux', 'owned native processes require Linux procfs/process groups')
  assert(typeof executableArg === 'string' && typeof cwdArg === 'string' && typeof logFileArg === 'string')
  const executable = fs.realpathSync(executableArg), cwd = fs.realpathSync(cwdArg), logFile = path.resolve(logFileArg)
  assert(Array.isArray(args) && args.length <= 64 && args.every(arg => typeof arg === 'string' &&
    !arg.includes('\0') && Buffer.byteLength(arg) <= 1024 * 1024), 'bounded native argv required')
  assert(args.reduce((sum, arg) => sum + Buffer.byteLength(arg) + 1, 0) <= limits.requestBytesMax,
    'native aggregate argv bound exceeded')
  assert(env && Object.getPrototypeOf(env) === Object.prototype && Object.entries(env).every(([key, value]) =>
    key && !key.includes('=') && !key.includes('\0') && typeof value === 'string' && !value.includes('\0')), 'plain string native environment required')
  assert(Object.entries(env).reduce((sum, [key, value]) => sum + Buffer.byteLength(key) + Buffer.byteLength(value) + 2, 0)
    <= 1024 * 1024, 'native environment byte bound exceeded')
  for (const [name, value, max] of [['watchdog', watchdogMs, limits.watchdogMsMax], ['TERM grace', termGraceMs, 10000],
    ['KILL grace', killGraceMs, 10000], ['log bytes', logBytesMax, limits.logBytesMax]])
    assert(Number.isSafeInteger(value) && value > 0 && value <= max, `native ${name} bound differs`)
  const executableIdentity = fileIdentity(fs.statSync(executable, {bigint: true}))
  const fd = fs.openSync(logFile, 'wx'), started = performance.now()
  const evidence = {schema: 'quod-owned-native-process-v1-research', protocol: OWNED_NATIVE_MODE,
    pid: null, pgid: null, ownershipVerified: false, cleanupCompleted: false, spawnAttempts: 0,
    spawnAttemptMonoMs: null, spawnedMonoMs: null, spawnEventMonoMs: null, rootExitMonoMs: null, closedMonoMs: null,
    streamsClosedMonoMs: null, groupSettledMonoMs: null, joinedMonoMs: null,
    executable, executableIdentity, argvSha256: sha(Buffer.from([executable, ...args].join('\0') + '\0')), spawnError: null,
    watchdogMs, termGraceMs, killGraceMs, logBytesMax, root: null, failures: [], signals: [], procScans: 0,
    tracked: [], finalLive: [], finalZombies: [], logBytes: 0, retainedLogBytes: 0,
    streams: {stdout: {bytes: 0, closed: false}, stderr: {bytes: 0, closed: false}},
    settlement: 'root and both pipes closed; no live owned-group or observed-descendant process; zombies reported explicitly'}
  let child, rootClosed = false, exit = {code: null, signal: null}, timedOut = false
  let stopRequested = false, termAt = null, killAt = null, killOverrun = false, lastScan = null
  let resolveRoot
  const rootClosure = new Promise(resolve => {resolveRoot = resolve})
  const streams = [], tracked = new Map(), failureLabels = new Set()
  const fail = (label, error) => {
    if (!failureLabels.has(label)) {
      failureLabels.add(label)
      if (evidence.failures.length < 32) evidence.failures.push({kind: label, ...errorData(error)})
    }
    stopRequested = true
  }
  const aliveRow = row => row && row.state !== 'Z' && row.state !== 'X' && row.state !== 'x'
  const identityKey = row => `${row.pid}:${row.startTicks}`
  function scan() {
    const rows = procRows(); evidence.procScans++
    const byPID = new Map(rows.map(row => [row.pid, row]))
    let added = true
    while (added) {
      added = false
      for (const row of rows) {
        const parent = byPID.get(row.ppid)
        if (row.pgid === evidence.pgid || tracked.has(identityKey(row)) ||
          parent && tracked.has(identityKey(parent))) {
          const key = identityKey(row)
          if (!tracked.has(key)) {
            assert(tracked.size < limits.trackedProcessesMax, 'native observed descendant bound exceeded')
            tracked.set(key, {...row, firstObservedMonoMs: performance.now()}); added = true
          }
          const observed = tracked.get(key)
          observed.lastObservedMonoMs = performance.now(); observed.lastState = row.state
          if (row.pid !== evidence.pid && (row.pgid !== evidence.pgid || row.session !== evidence.pgid))
            fail('descendant-escaped-group', new Error('observed native descendant changed its owned group/session'))
        }
      }
    }
    const owned = rows.filter(row => row.pgid === evidence.pgid || tracked.has(identityKey(row)))
    lastScan = {live: owned.filter(aliveRow), zombies: owned.filter(row => !aliveRow(row))}
    return lastScan
  }
  function signalOwned(signal) {
    if (!evidence.pid) return
    const stamp = {signal, monoMs: performance.now(), group: evidence.pgid, delivered: false, extraPids: []}
    try {process.kill(-evidence.pgid, signal); stamp.delivered = true}
    catch (error) {if (!gone(error)) fail('group-signal', error)}
    // Only signal observed escaped descendants after fresh PID/start-time
    // equality; never signal a reused PID from an old proc sample.
    for (const row of lastScan?.live || []) {
      if (row.pgid === evidence.pgid) continue
      try {
        const current = procRow(row.pid)
        if (current && current.startTicks === row.startTicks && aliveRow(current)) {
          process.kill(row.pid, signal); stamp.extraPids.push(row.pid)
        }
      } catch (error) {if (!gone(error)) fail('descendant-signal', error)}
    }
    if (evidence.signals.length < 32) evidence.signals.push(stamp)
  }
  function observe(stream, name) {
    assert(stream && typeof stream.on === 'function', 'owned native pipe unavailable')
    const row = evidence.streams[name]
    streams.push(new Promise(resolve => stream.once('close', () => {
      row.closed = true; row.closedMonoMs = performance.now(); resolve()
    })))
    stream.on('error', error => fail(`${name}-pipe`, error))
    stream.on('data', bytes => {
      row.bytes += bytes.length; evidence.logBytes += bytes.length
      const remaining = Math.max(0, logBytesMax - evidence.retainedLogBytes)
      const retained = bytes.subarray(0, remaining)
      try {
        for (let offset = 0; offset < retained.length;) {
          const length = fs.writeSync(fd, retained, offset, retained.length - offset)
          assert(length > 0, 'native log write made no progress'); offset += length
          evidence.retainedLogBytes += length
        }
        // The opt-in path retains combined logs without live parent stdout
        // forwarding. An asynchronous parent-console error must not escape
        // child/group cleanup; final summaries and native.log remain available.
      } catch (error) {fail('log-write', error)}
      if (evidence.logBytes > logBytesMax) fail('log-bound', new Error('owned native log byte bound exceeded'))
    })
  }
  const watchdog = setTimeout(() => {
    timedOut = true; fail('watchdog', new Error('owned native watchdog exhausted'))
    if (evidence.pid && termAt === null) {termAt = performance.now(); signalOwned('SIGTERM')}
  }, watchdogMs)
  try {
    // Linux detached setsid establishes PGID=SID=PID before exec. No shell or
    // user callbacks intervene in spawn/first identity observation.
    evidence.spawnAttempts++; evidence.spawnAttemptMonoMs = performance.now()
    child = spawn(executable, [...args], {cwd, env: {...env}, detached: true,
      stdio: ['ignore', 'pipe', 'pipe']})
    evidence.spawnedMonoMs = performance.now(); evidence.pid = child.pid || null; evidence.pgid = child.pid || null
    child.once('spawn', () => {evidence.spawnEventMonoMs = performance.now()})
    child.once('error', error => {evidence.spawnError = errorData(error); fail('spawn', error)})
    child.once('exit', (code, signal) => {
      evidence.rootExitMonoMs = performance.now(); exit = {code, signal}
    })
    child.once('close', (code, signal) => {
      rootClosed = true; evidence.closedMonoMs = performance.now(); exit = {code, signal}; resolveRoot()
    })
    observe(child.stdout, 'stdout'); observe(child.stderr, 'stderr')
    if (evidence.pid) {
      const root = procRow(evidence.pid)
      assert(root && root.pgid === evidence.pid && root.session === evidence.pid,
        'native child did not establish its exclusive process group/session')
      const actualExecutable = fileIdentity(fs.statSync(`/proc/${evidence.pid}/exe`, {bigint: true}))
      assert(sameIdentity(actualExecutable, executableIdentity), 'native child executable identity differs')
      const cmdline = fs.readFileSync(`/proc/${evidence.pid}/cmdline`)
      assert.equal(sha(cmdline), evidence.argvSha256, 'native child actual argv differs')
      evidence.observedArgvSha256 = sha(cmdline)
      assert.equal(fs.realpathSync(`/proc/${evidence.pid}/cwd`), cwd, 'native child actual cwd differs')
      evidence.observedCwd = cwd
      evidence.root = root; tracked.set(identityKey(root), {...root, firstObservedMonoMs: performance.now()})
      evidence.ownershipVerified = true
    }
    while (true) {
      try {scan()} catch (error) {fail('proc-scan', error); lastScan = null}
      if (rootClosed && lastScan && lastScan.live.length === 0) break
      if (evidence.rootExitMonoMs !== null && lastScan?.live.some(row => row.pid !== evidence.pid))
        fail('descendant-after-root', new Error('native descendant remained live after root exit'))
      const now = performance.now()
      if (stopRequested && termAt === null) {termAt = now; signalOwned('SIGTERM')}
      if (termAt !== null && killAt === null && now - termAt >= termGraceMs) {
        fail('term-grace', new Error('owned native exceeded TERM cleanup grace'))
        killAt = now; signalOwned('SIGKILL')
      }
      if (killAt !== null && now - killAt >= killGraceMs) {
        if (!killOverrun) {killOverrun = true; fail('kill-grace', new Error('owned native exceeded KILL cleanup grace'))}
        // A bounded grace is a failure, never permission to return with a live
        // owned process. Continue joining; the outer job watchdog is ultimate
        // protection against an unkillable kernel task/procfs failure.
        signalOwned('SIGKILL')
      }
      if (rootClosed) await pause(limits.procPollMs)
      else await Promise.race([pause(limits.procPollMs), rootClosure])
    }
  } catch (error) {
    fail('supervision', error)
  } finally {
    // Errors in ownership observation must follow the same actual settlement
    // path. In particular an 'error' event never rejects before close/pipes.
    if (child) {
      while (true) {
        try {scan()} catch (error) {fail('proc-scan', error); lastScan = null}
        if (rootClosed && lastScan && lastScan.live.length === 0) break
        if (termAt === null) {termAt = performance.now(); signalOwned('SIGTERM')}
        if (killAt === null && performance.now() - termAt >= termGraceMs) {
          fail('term-grace', new Error('owned native exceeded TERM cleanup grace'))
          killAt = performance.now(); signalOwned('SIGKILL')
        }
        if (killAt !== null && performance.now() - killAt >= killGraceMs) {
          fail('kill-grace', new Error('owned native exceeded KILL cleanup grace')); signalOwned('SIGKILL')
        }
        await pause(limits.procPollMs)
      }
      await rootClosure
      await Promise.all(streams)
      evidence.streamsClosedMonoMs = Math.max(...Object.values(evidence.streams).map(row => row.closedMonoMs))
      evidence.groupSettledMonoMs = performance.now()
    }
    clearTimeout(watchdog)
    try {fs.closeSync(fd)} catch (error) {fail('log-close', error)}
    evidence.tracked = [...tracked.values()]; evidence.finalLive = lastScan?.live || []
    evidence.finalZombies = lastScan?.zombies || []
    evidence.cleanupCompleted = !!child && rootClosed && Object.values(evidence.streams).every(row => row.closed) &&
      !!lastScan && lastScan.live.length === 0
    evidence.exit = exit
    evidence.joinedMonoMs = performance.now()
    if (!evidence.cleanupCompleted) fail('cleanup', new Error('owned native cleanup did not complete'))
  }
  if (!evidence.ownershipVerified) fail('ownership', new Error('native root identity was not verified'))
  if (exit.code !== 0 || exit.signal !== null) fail('native-exit', new Error('native process did not exit normally with code0'))
  return {exitCode: evidence.failures.length ? Number.isInteger(exit.code) && exit.code > 0 ? exit.code : 1 : 0,
    signal: exit.signal, timedOut,
    seconds: (performance.now() - started) / 1000, logFile, ownership: evidence}
}

export function ownedMatrixNativePolicy() {
  assert.equal(process.platform, 'linux'); assert.equal(process.arch, 'x64')
  assert.equal(process.version, 'v24.18.0', 'owned native requires reviewed Node24.18.0')
  assert.equal(process.execArgv.length, 0, 'owned native requires no Node CLI flags')
  assert(!process.env.NODE_OPTIONS && !process.env.NODE_PATH, 'owned native requires no Node preload/module overrides')
  assert(authKeys.every(key => !Object.hasOwn(process.env, key)), 'cache authentication must be absent before native start')
  const childSha256 = sha(regularBytes(nativeChild, 1024 * 1024))
  assert.equal(childSha256, reviewedChildSha256, 'owned native child source differs from reviewed finite profile')
  return {schema: 'quod-owned-matrix-native-policy-v1-research', protocol: OWNED_NATIVE_MODE,
    adapterSha256: sha(regularBytes(self, 1024 * 1024)), childSha256,
    nodeExecutableSha256: sha(fs.readFileSync(process.execPath)), nodeVersion: process.version,
    platform: process.platform, arch: process.arch, ...limits}
}

export async function runOwnedMatrixNativeBatch({project: projectArg, out: outArg, selected, instanceIds,
  suiteFiles, node = process.execPath, watchdogMs = limits.watchdogMsMax}) {
  const policy = ownedMatrixNativePolicy()
  const project = fs.realpathSync(path.resolve(projectArg)), out = path.resolve(outArg)
  assert.equal(fs.realpathSync(node), fs.realpathSync(process.execPath), 'reviewed same native Node executable required')
  assert(Number.isSafeInteger(watchdogMs) && watchdogMs > 0 && watchdogMs <= limits.watchdogMsMax)
  assert(selected === undefined || Array.isArray(selected))
  assert(suiteFiles === undefined || Array.isArray(suiteFiles) && suiteFiles.every(file => typeof file === 'string'))
  assert(instanceIds === undefined || Array.isArray(instanceIds) && instanceIds.every(id => /^[a-f0-9]{64}$/.test(id)))
  fs.mkdirSync(out, {recursive: true}); assert.equal(fs.readdirSync(out).length, 0, 'native batch output must be empty')
  if (instanceIds?.length === 0) return {skipped: true, exitCode: 0, seconds: 0, selected: [], executed: []}
  const requestFile = path.join(out, 'request.json'), reportFile = path.join(out, 'native-report.json')
  const evidenceFile = path.join(out, 'native-selection.json'), logFile = path.join(out, 'native.log')
  const request = {schema: 'quod-managed-matrix-native-batch-request-v1', project,
    selected: selected || null, instanceIds: instanceIds || null,
    filters: suiteFiles?.map(file => path.resolve(project, file)) || null, reportFile, evidenceFile}
  const requestBytes = Buffer.from(JSON.stringify(request, null, 2) + '\n')
  assert(requestBytes.length <= limits.requestBytesMax, 'native request byte bound exceeded')
  fs.writeFileSync(requestFile, requestBytes, {flag: 'wx'})
  const args = [nativeChild, '--child', requestFile], env = {...process.env, QUOD_SHIM_ACTIVE: '1'}
  write(path.join(out, 'invocation.json'), {schema: 'quod-owned-matrix-native-invocation-v1-research',
    executable: fs.realpathSync(node), args, cwd: project, requestSha256: sha(requestBytes), policy, watchdogMs,
    envBootstrap: {TEST: env.TEST ?? null, VITEST: env.VITEST ?? null, NODE_ENV: env.NODE_ENV ?? null},
    cacheAuthenticationAbsent: authKeys.every(key => !Object.hasOwn(env, key)), detached: true,
    stdio: ['ignore', 'bounded-log-pipe', 'bounded-log-pipe']})
  const processResult = await runOwnedNativeProcess({executable: node, args, cwd: project, env, logFile, watchdogMs})
  const result = {...processResult, reportFile, evidenceFile, data: null}
  try {
    const after = ownedMatrixNativePolicy()
    assert.deepEqual(after, policy, 'owned native sources/tools changed during execution')
    if (fs.existsSync(evidenceFile)) result.data = JSON.parse(new TextDecoder('utf-8', {fatal: true})
      .decode(regularBytes(evidenceFile, limits.evidenceBytesMax)))
  } catch (error) {
    result.exitCode = 1; result.ownership.failures.push({kind: 'post-native-evidence', ...errorData(error)})
  }
  write(path.join(out, 'ownership.json'), result.ownership)
  write(path.join(out, 'process.json'), {...result, data: undefined})
  return result
}
