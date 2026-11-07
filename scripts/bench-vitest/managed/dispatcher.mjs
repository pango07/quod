// Explicit opt-in dispatcher for the reviewed convex-helpers profile. Cache
// transport is supplied by the caller; a digest inside cache bytes is no trust.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {fileURLToPath} from 'node:url'
import {spawn} from 'node:child_process'
import {createManagedCompiler, canonicalManagedJSON} from './compiler.mjs'
import {prepareManagedSuite} from './preparation.mjs'
import {createManagedExecutor} from './executor.mjs'
import {makeExecutionIdentity, validatePassingResponse, lookupReceipt, recordReceipt} from './receipt.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const defaultFile = 'packages/convex-helpers/server/stream.property.test.ts'
const sha = value => crypto.createHash('sha256').update(value).digest('hex')
const inside = (root, file) => file === root || file.startsWith(root + path.sep)
const errorData = error => ({name: String(error?.name || 'Error'), message: String(error?.message || error)})
const status = {pass: 'passed', fail: 'failed', skip: 'skipped', todo: 'todo'}
const state = {passed: 'pass', failed: 'fail', skipped: 'skip', pending: 'skip', todo: 'todo'}
const write = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n')

function reporterArguments(args, project) {
  if (!Array.isArray(args) || args.some(arg => typeof arg !== 'string') || args[0] !== 'run') return null
  const reporters = []
  let output
  for (let index = 1; index < args.length; index++) {
    const arg = args[index]
    if (arg === '--reporter' || arg === '--reporters') reporters.push(args[++index])
    else if (/^--reporters?=/.test(arg)) reporters.push(arg.slice(arg.indexOf('=') + 1))
    else if (arg === '--outputFile' || arg === '--outputFile.json') output = args[++index]
    else if (/^--outputFile(?:\.json)?=/.test(arg)) output = arg.slice(arg.indexOf('=') + 1)
    else return null
  }
  if (reporters.some(name => !['default', 'json'].includes(name)) || output !== undefined && (typeof output !== 'string' || !output)) return null
  if (!reporters.length) reporters.push('default')
  if (output && !reporters.includes('json')) return null
  return {reporters: [...new Set(reporters)], output: output ? path.resolve(project, output) : null}
}
function originalJSONOutput(args, project) {
  let output, json = false
  for (let index = 0; index < args.length; index++) {
    const arg = args[index]
    if (arg === '--reporter' || arg === '--reporters') json ||= args[++index] === 'json'
    else if (/^--reporters?=json$/.test(arg)) json = true
    else if (arg === '--outputFile' || arg === '--outputFile.json') output = args[++index]
    else if (/^--outputFile(?:\.json)?=/.test(arg)) output = arg.slice(arg.indexOf('=') + 1)
  }
  return json && typeof output === 'string' && output ? path.resolve(project, output) : null
}
function boundarySupported(data) {
  const b = data?.boundary, c = b?.completion
  return !data?.error && c?.sealed === true && c.intrinsicLockdown === true &&
    ['unsupported', 'unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors'].every(key => Array.isArray(b[key]) && b[key].length === 0) &&
    b.pendingTimers === 0 && ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'].every(key => c[key] === 0)
}
function normalizedRows(report) {
  return (report?.testResults || []).flatMap(file => (file.assertionResults || []).map(test =>
    ({file: file.name, name: test.fullName, state: state[test.status] || test.status})))
    .sort((a, b) => a.file.localeCompare(b.file) || a.name.localeCompare(b.name) || a.state.localeCompare(b.state))
}
function mergeReport(ordinary, data, absolute, replayed) {
  const file = {name: absolute, status: data.files[0].state === 'pass' ? 'passed' : 'failed', message: '',
    assertionResults: data.rows.map(row => ({ancestorTitles: [], fullName: row.name, title: row.name,
      status: status[row.state] || row.state, failureMessages: row.errors.map(error => error.stack || error.message),
      meta: {}, tags: [], benchmarks: []})), quodManaged: {replayed, virtualTime: data.boundary.virtualTime}}
  const report = ordinary ? structuredClone(ordinary) : {snapshot: {added: 0, failure: false, filesAdded: 0, filesRemoved: 0,
    filesRemovedList: [], filesUnmatched: 0, filesUpdated: 0, matched: 0, total: 0, unchecked: 0,
    uncheckedKeysByFile: [], unmatched: 0, updated: 0, didUpdate: false}, startTime: Date.now()}
  report.testResults = [...(report.testResults || []).filter(row => path.resolve(row.name) !== absolute), file]
  const tests = report.testResults.flatMap(row => row.assertionResults || [])
  report.numTotalTestSuites = report.testResults.length
  report.numPassedTestSuites = report.testResults.filter(row => row.status === 'passed').length
  report.numFailedTestSuites = report.testResults.filter(row => row.status === 'failed').length
  report.numPendingTestSuites = report.numTotalTestSuites - report.numPassedTestSuites - report.numFailedTestSuites
  report.numTotalTests = tests.length
  report.numPassedTests = tests.filter(row => row.status === 'passed').length
  report.numFailedTests = tests.filter(row => row.status === 'failed').length
  report.numTodoTests = tests.filter(row => row.status === 'todo').length
  report.numPendingTests = tests.length - report.numPassedTests - report.numFailedTests - report.numTodoTests
  report.success = ordinary?.success !== false && data.ok === true && report.numFailedTests === 0 && report.numFailedTestSuites === 0
  return report
}
async function runOrdinary({project, node, entry, args, name, out}) {
  const started = performance.now(), logPath = path.join(out, name + '.log'), fd = fs.openSync(logPath, 'w')
  const invocation = {node, entry, args, cwd: project}
  write(path.join(out, name + '-invocation.json'), invocation)
  let exit
  try {
    exit = await new Promise((resolve, reject) => {
      const child = spawn(node, [entry, ...args], {cwd: project, env: {...process.env, QUOD_SHIM_ACTIVE: '1'}, stdio: ['ignore', 'pipe', 'pipe']})
      child.stdout.on('data', bytes => { fs.writeSync(fd, bytes); process.stdout.write(bytes) })
      child.stderr.on('data', bytes => { fs.writeSync(fd, bytes); process.stderr.write(bytes) })
      child.once('error', reject)
      child.once('close', (code, signal) => resolve({exitCode: code === null ? 1 : code, signal}))
    })
  } finally { fs.closeSync(fd) }
  return {...exit, seconds: (performance.now() - started) / 1000, logPath}
}
function reportFrom(file) {
  if (!file || !fs.existsSync(file)) return null
  const report = JSON.parse(fs.readFileSync(file, 'utf8'))
  assert(Array.isArray(report.testResults), 'ordinary reporter omitted testResults')
  return report
}

/** Caller callbacks must derive trustedReceiptSha256 from authenticated
 * transport metadata, never a manifest in the mutable cache payload. */
export async function dispatchManaged({project: projectArg, out: outArg, cacheDir: cacheArg, args = ['run'],
  vitestEntry, node = process.execPath, control = false, epoch = 1700000000000, seed = 42,
  env = {NODE_ENV: 'test'}, lookup, record, files, expectedCount, emitTelemetry = false} = {}) {
  const project = fs.realpathSync(path.resolve(projectArg)), out = path.resolve(outArg)
  fs.mkdirSync(out, {recursive: true})
  assert(fs.statSync(out).isDirectory() && fs.readdirSync(out).length === 0, 'dispatcher OUT must be empty')
  const cacheDir = path.resolve(cacheArg || path.join(out, 'cache'))
  fs.mkdirSync(cacheDir, {recursive: true})
  const require = createRequire(path.join(project, 'package.json'))
  const entry = fs.realpathSync(vitestEntry || path.join(path.dirname(require.resolve('vitest/package.json')), 'vitest.mjs'))
  node = fs.realpathSync(node)
  const started = performance.now(), parsed = reporterArguments(args, project)
  const result = {schema: 'quod-managed-dispatch-v1', mode: control ? 'control' : 'v0', optIn: true,
    project, args, files: [], phases: [], hit: false, managedExecutions: 0, fallback: false, exitCode: 1,
    receipts: [], rows: [], reportPath: null, ordinaryRuns: []}
  let managedFailure = false, ordinaryFailure = false
  const save = () => write(path.join(out, 'result.json'), result)
  const selected = files === undefined ? null : files.map(file => {
    assert(typeof file === 'string' && !path.isAbsolute(file) && !file.split(/[\\/]/).includes('..'), 'explicit files must be project relative')
    return file.split(path.sep).join('/')
  })
  const originalArgs = [...args, ...(selected || [])]
  result.candidate = selected?.[0] || defaultFile
  async function ordinaryAll(reason, knownFailure = false) {
    result.fallback = true; result.fallbackReason = String(reason)
    const ordinary = await runOrdinary({project, node, entry, args: originalArgs, name: 'ordinary-' + result.ordinaryRuns.length, out})
    result.ordinaryRuns.push(ordinary)
    ordinaryFailure ||= ordinary.exitCode !== 0
    result.ordinary = ordinary
    managedFailure ||= knownFailure
    result.exitCode = ordinary.exitCode || (managedFailure || ordinaryFailure ? 1 : 0)
    const reportPath = parsed?.output || originalJSONOutput(args, project)
    const report = reportFrom(reportPath)
    if (report) { result.reportPath = reportPath; result.rows = normalizedRows(report) }
    const target = selected?.[0] || defaultFile
    result.files.push({file: target, decision: 'ordinary', reason: String(reason), ordinaryExit: ordinary.exitCode})
  }
  save()
  try {
    if (!parsed) { await ordinaryAll('unsupported Vitest arguments'); return result }
    if (selected && selected.length !== 1) { await ordinaryAll('one explicit managed file is supported'); return result }
    assert(Number.isSafeInteger(epoch) && epoch >= 0 && Number.isSafeInteger(seed) && seed >= 0 && seed <= 0xffffffff,
      'declared finite epoch and uint32 seed required')
    assert(env && typeof env === 'object' && !Array.isArray(env) && Object.values(env).every(value => typeof value === 'string'), 'declared environment must contain string values')
    canonicalManagedJSON(env)
    const target = selected?.[0] || defaultFile, count = selected ? expectedCount : 47
    assert(Number.isSafeInteger(count) && count > 0, 'explicit managed workload count required')
    const suite = await prepareManagedSuite({project, files: selected || undefined, seed})
    write(path.join(out, 'preparation.json'), suite.evidence)
    result.preparationSeconds = suite.evidence.seconds
    const actual = suite.selected.filter(spec => spec.file === target)
    const candidate = suite.prepared.find(spec => spec.file === target)
    if (actual.length !== 1 || !candidate) {
      await ordinaryAll(suite.declined.find(spec => spec.file === target)?.reason || 'managed candidate not selected exactly once')
      return result
    }
    // The finite review permits only the project's installed pinned Vitest entry.
    assert.equal(entry, fs.realpathSync(path.join(path.dirname(require.resolve('vitest/package.json')), 'vitest.mjs')))
    assert.equal(node, fs.realpathSync(process.execPath), 'different ordinary Node executable unsupported')
    const executor = createManagedExecutor({out, watchdogMs: 300000, onPhase(row) {
      result.phases.push({name: row.name, seconds: row.seconds, responseSha256: row.responseSha256}); save()
    }})
    const compile = createManagedCompiler({project}), base = {epoch, seed, env: {...env}}
    const preflightBuild = await compile('preflight.mjs', base)
    write(path.join(out, 'preflight-build.json'), {...preflightBuild, bundle: undefined})
    const preflight = await executor.execute('preflight', preflightBuild.bundle)
    assert(preflight.ok && preflight.checks.every(check => check.pass) && preflight.boundary.completion.sealed &&
      !preflight.boundary.unguardable.length, 'managed capability preflight declined')
    const options = {...base, vitest: candidate.config, file: '/project/' + target}
    const built = await compile('runner.mjs', options, candidate.absolute)
    write(path.join(out, 'test-build.json'), {...built, bundle: undefined, options, preparation: candidate.identity})
    const expected = {count, file: path.posix.relative(candidate.config.root, options.file)}
    const runtimePolicy = {...executor.runtimePolicy, nativeFingerprintSha256: sha(canonicalManagedJSON({
      preflightIdentity: preflightBuild.identitySha256, inventory: preflight.boundary.inventory, symbols: preflight.boundary.symbolInventory}))}
    const execution = makeExecutionIdentity({built, prepared: candidate, runtimePolicy,
      supervisorPolicy: {receiptValidatorSha256: sha(fs.readFileSync(path.join(here, 'receipt.mjs'))),
        experimentSha256: sha(fs.readFileSync(fileURLToPath(import.meta.url))), expected}, host: executor.host})
    write(path.join(out, 'execution-identity.json'), execution)
    result.executionKey = execution.key
    result.runtimeClassSha256 = sha(canonicalManagedJSON({runtimePolicy, host: executor.host}))
    result.nativeFingerprintSha256 = runtimePolicy.nativeFingerprintSha256
    result.compilationSeconds = built.seconds
    let found = {hit: false, reason: control ? 'control mode' : 'no authenticated receipt supplied'}
    if (!control && lookup) {
      try {
        const source = await lookup({execution, expected})
        if (source) found = lookupReceipt({...source, execution, expected})
      } catch (error) { found = {hit: false, reason: 'cache lookup: ' + error.message} }
    }
    result.hit = found.hit; result.missReason = found.reason || null
    let data
    if (found.hit) {
      data = found.data
      fs.writeFileSync(path.join(out, 'selected-response.json'), found.responseBytes)
    } else {
      result.managedExecutions++
      data = await executor.execute('managed', built.bundle)
      write(path.join(out, 'selected-response.json'), data)
    }
    const supported = boundarySupported(data)
    if (!supported || data.ok !== true) {
      const knownFailure = supported && data.ok === false
      managedFailure ||= knownFailure
      await ordinaryAll(knownFailure ? 'managed test failure' : 'unsupported or incomplete managed execution', knownFailure)
      return result
    }
    validatePassingResponse(data, expected)
    const effect = {ok: data.ok, rows: data.rows, files: data.files, logs: data.boundary.logs, virtualTime: data.boundary.virtualTime}
    result.effectsSha256 = sha(canonicalManagedJSON(effect))
    result.effects = effect
    const decision = {file: target, decision: found.hit ? 'replay' : 'managed', executionKey: execution.key}
    result.files.push(decision)
    // A basename exclusion is safe only when actual resolved selection proves
    // it removes exactly this one file/project instance. Include the absolute
    // path too; native Vite globbers differ in absolute ignore handling.
    const basename = path.basename(target)
    assert(/^[A-Za-z0-9_.-]+$/.test(basename) && suite.selected.filter(spec => path.basename(spec.file) === basename).length === 1,
      'candidate basename is not unique in actual selection')
    const remainder = suite.selected.filter(spec => spec.absolute !== candidate.absolute)
    let ordinaryReport = null
    if (remainder.length) {
      const ordinaryOutput = path.join(out, 'ordinary-report.json')
      const reporters = [...new Set([...parsed.reporters, 'json'])]
      const ordinaryArgs = ['run', ...(selected || []), ...reporters.flatMap(name => ['--reporter', name]),
        '--outputFile=' + ordinaryOutput, '--exclude=' + candidate.absolute, '--exclude=**/' + basename]
      result.ordinary = await runOrdinary({project, node, entry, args: ordinaryArgs, name: 'ordinary-remainder', out})
      result.ordinaryRuns.push(result.ordinary)
      ordinaryFailure ||= result.ordinary.exitCode !== 0
      ordinaryReport = reportFrom(ordinaryOutput)
      assert(ordinaryReport, 'ordinary remainder did not produce its complete report')
      result.ordinaryRepeatedCandidate = ordinaryReport.testResults.some(file => path.resolve(file.name) === candidate.absolute)
      assert(!result.ordinaryRepeatedCandidate, 'ordinary remainder repeated the managed candidate')
      if (result.ordinary.exitCode === 0) assert.deepEqual(ordinaryReport.testResults.map(file => path.resolve(file.name)).sort(),
        remainder.map(spec => path.resolve(spec.absolute)).sort(), 'ordinary remainder selection differs from actual preparation')
    }
    const report = mergeReport(ordinaryReport, data, candidate.absolute, found.hit)
    assert.equal(report.testResults.length, suite.selected.length, 'merged report omitted or repeated actual selected file/project instances')
    result.reportPath = parsed.output || path.join(out, 'vitest-report.json')
    fs.mkdirSync(path.dirname(result.reportPath), {recursive: true})
    write(result.reportPath, report)
    result.rows = normalizedRows(report)
    result.exitCode = result.ordinary?.exitCode || (managedFailure || ordinaryFailure || !report.success ? 1 : 0)
    for (const [method, values] of data.boundary.logs) (['error', 'warn'].includes(method) ? process.stderr : process.stdout).write(values.join(' ') + '\n')
    if (!found.hit) {
      const receiptFile = path.join(out, 'receipt.json')
      const receipt = recordReceipt({file: receiptFile, execution, expected,
        responseBytes: fs.readFileSync(path.join(out, 'managed', 'response.json'))})
      const item = {...receipt, file: receiptFile, expected}
      decision.receipt = item; result.receipts.push(item)
      if (!control && record) {
        try { await record({execution, expected, file: receiptFile, receiptSha256: receipt.receiptSha256}) }
        catch (error) { result.cacheRecordError = errorData(error) }
      }
    }
    return result
  } catch (error) {
    result.managedError = errorData(error)
    await ordinaryAll(error.message)
    return result
  } finally {
    result.commandSeconds = (performance.now() - started) / 1000
    save()
    if (emitTelemetry) console.log('QUOD_MANAGED_RESULT ' + JSON.stringify({mode: result.mode, candidate: selected?.[0] || defaultFile,
      hit: result.hit, managedExecutions: result.managedExecutions, fallback: result.fallback, exitCode: result.exitCode,
      effectsSha256: result.effectsSha256 || null, runtimeClassSha256: result.runtimeClassSha256 || null,
      executionKey: result.executionKey || null, commandSeconds: result.commandSeconds}))
  }
}

async function main() {
  const [project, out, cacheDir, ...options] = process.argv.slice(2)
  assert(project && out && cacheDir, 'usage: dispatcher.mjs PROJECT OUT CACHE_DIR [--control] [--node NODE --vitest ENTRY] -- ARGS')
  let control = false, node = process.execPath, vitestEntry, index = 0
  for (; index < options.length && options[index] !== '--'; index++) {
    if (options[index] === '--control') control = true
    else if (options[index] === '--node') node = options[++index]
    else if (options[index] === '--vitest') vitestEntry = options[++index]
    else throw new Error('unsupported dispatcher option: ' + options[index])
  }
  const args = options[index] === '--' ? options.slice(index + 1) : ['run']
  const trustPath = process.env.QUOD_MANAGED_TRUST_FILE
  let trusted = []
  if (trustPath) {
    assert(!inside(path.resolve(cacheDir), path.resolve(trustPath)), 'trusted metadata must be outside mutable cache')
    trusted = JSON.parse(fs.readFileSync(trustPath, 'utf8'))
    assert(Array.isArray(trusted), 'trusted receipt metadata must be an explicit array')
  }
  const result = await dispatchManaged({project, out, cacheDir, args, node, vitestEntry, control,
    epoch: Number(process.env.QUOD_MANAGED_EPOCH || 1700000000000), seed: Number(process.env.QUOD_MANAGED_SEED || 42),
    env: process.env.QUOD_MANAGED_ENV ? JSON.parse(process.env.QUOD_MANAGED_ENV) : {NODE_ENV: 'test'},
    files: process.env.QUOD_MANAGED_FILES ? JSON.parse(process.env.QUOD_MANAGED_FILES) : undefined,
    expectedCount: Number(process.env.QUOD_MANAGED_EXPECTED_COUNT || 1), emitTelemetry: true,
    lookup: async ({execution}) => {
      const item = trusted.find(item => item.key === execution.key)
      return item ? {file: path.resolve(item.file), trustedReceiptSha256: item.receiptSha256} : null
    },
    record: async ({execution, file, receiptSha256}) => {
      const destination = path.join(cacheDir, execution.key + '-' + receiptSha256 + '.json')
      if (!fs.existsSync(destination)) fs.copyFileSync(file, destination, fs.constants.COPYFILE_EXCL)
    }})
  process.exitCode = result.exitCode
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.stack || error); process.exitCode = 1 })
}
