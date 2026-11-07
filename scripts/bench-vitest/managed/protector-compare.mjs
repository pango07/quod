// Trusted JSON-only analysis after both actual 47-test workers finish in CI.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {execFileSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'

const [projectArg, candidateArg, legacyArg, outArg, legacySourceArg] = process.argv.slice(2)
assert(projectArg && candidateArg && legacyArg && outArg && legacySourceArg,
  'usage: protector-compare.mjs PROJECT CANDIDATE_OUTPUT LEGACY_OUTPUT EMPTY_OUTPUT LEGACY_SOURCE')
const candidate = fs.realpathSync(candidateArg), legacy = fs.realpathSync(legacyArg)
const project = fs.realpathSync(projectArg), legacySource = fs.realpathSync(legacySourceArg), out = path.resolve(outArg)
assert(!fs.existsSync(out), 'comparison output must be new')
fs.mkdirSync(out, {recursive: true})
const here = path.dirname(fileURLToPath(import.meta.url)), source = path.resolve(here, '../../..')
const legacyCommit = '2a4d6f2f7547dd00e720a32d5b451229784ba8fb'
const candidateContract = 'quod-managed-v0-protectors-frozen-slots-investigation'
const candidatePolicy = 'quod-managed-frozen-array-promise-slots-v1-probe'
const legacyContract = 'quod-managed-v0-investigation'
const test = 'packages/convex-helpers/server/stream.property.test.ts'
const testSourceSha256 = '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75'
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => {
  assert(fs.statSync(file).isFile() && !fs.lstatSync(file).isSymbolicLink() && fs.statSync(file).size <= 16 * 1024 * 1024)
  return JSON.parse(fs.readFileSync(file, 'utf8'))
}
const head = directory => execFileSync('git', ['-C', directory, 'rev-parse', 'HEAD'], {encoding: 'utf8'}).trim()
const result = {schema: 'quod-managed-protector-fixed-order-diagnostic-v1', passed: false,
  order: ['candidate-full-hardening', 'legacy-control'],
  interpretation: 'same-runner fixed-order diagnostic; no causal or general performance claim',
  candidateSource: head(source), legacySource: head(legacySource), checks: []}
const save = () => fs.writeFileSync(path.join(out, 'result.json'), JSON.stringify(result, null, 2) + '\n')
const check = (name, pass) => { result.checks.push({name, pass: Boolean(pass)}); save(); assert(pass, name) }
const effects = data => ({ok: data.ok, rows: data.rows, files: data.files,
  logs: data.boundary.logs, virtualTime: data.boundary.virtualTime})
const complete = data => data.ok === true && data.rows.length === 47 &&
  new Set(data.rows.map(row => row.name)).size === 47 &&
  data.rows.every(row => row.state === 'pass' && row.errors.length === 0) &&
  data.files.length === 1 && data.files[0].name === 'server/stream.property.test.ts' &&
  data.files[0].state === 'pass' && data.files[0].errors.length === 0 &&
  ['unsupported', 'unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors'].every(key =>
    Array.isArray(data.boundary[key]) && data.boundary[key].length === 0) &&
  data.boundary.pendingTimers === 0 && data.boundary.completion.sealed === true &&
  data.boundary.completion.intrinsicLockdown === true && data.boundary.completion.reason === '' &&
  ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'].every(key => data.boundary.completion[key] === 0)
save()
try {
  check('legacy source is the frozen measured implementation', result.legacySource === legacyCommit)
  check('project retains the exact unchanged 47-test source', sha(fs.readFileSync(path.join(project, test))) === testSourceSha256)
  const current = read(path.join(candidate, 'result.json')), old = read(path.join(legacy, 'result.json'))
  const currentData = read(path.join(candidate, 'managed/response.json')), oldData = read(path.join(legacy, 'managed/response.json'))
  check('candidate full hardening and unchanged ordinary comparison completed', current.managedSupported === true &&
    current.managedPassed === true && current.namesAndStatesAgree === true && current.ordinary.exit === 0 &&
    current.preflightPassed === true && current.nativeOutboundDenied === true &&
    current.fixtureChecksPassed === true && current.moduleFixtureChecksPassed === true && current.boundaryFixtureChecksPassed === true &&
    current.protectorChecksPassed === true && current.protectorCheckCount === 51)
  check('legacy executes an actual uncached control worker', old.role === 'control' && old.passed === true &&
    old.actualManagedExecutions === 1 && old.replayed === false && old.phases.map(row => row.name).join(',') === 'preflight,managed')
  check('both actual workers pass all 47 original assertions', complete(currentData) && complete(oldData))
  check('policy differences are explicit', currentData.boundary.contract === candidateContract &&
    currentData.boundary.intrinsicPolicy === candidatePolicy && oldData.boundary.contract === legacyContract &&
    !Object.hasOwn(oldData.boundary, 'intrinsicPolicy'))
  const currentInvocation = read(path.join(candidate, 'managed/invocation.json'))
  const oldInvocation = read(path.join(legacy, 'managed/invocation.json'))
  check('same actual host class', JSON.stringify(currentInvocation.host) === JSON.stringify(oldInvocation.host))
  check('same actual Node and workerd binary bytes',
    currentInvocation.runtimePolicy.workerdSha256 === oldInvocation.runtimePolicy.workerdSha256 &&
    currentInvocation.runtimePolicy.supervisor.nodeExecutableSha256 === oldInvocation.runtimePolicy.supervisor.nodeExecutableSha256 &&
    currentInvocation.runtimePolicy.supervisor.nodeVersion === oldInvocation.runtimePolicy.supervisor.nodeVersion)
  const capabilities = policy => Object.fromEntries(['moduleName', 'services', 'compatibilityDate', 'compatibilityFlags',
    'globalOutbound', 'denyModuleSha256', 'bindings', 'processEnv', 'request'].map(key => [key, policy[key]]))
  check('same native capabilities environment and request policy',
    JSON.stringify(capabilities(currentInvocation.runtimePolicy)) === JSON.stringify(capabilities(oldInvocation.runtimePolicy)))
  const builds = [read(path.join(candidate, 'test-build.json')), read(path.join(legacy, 'test-build.json'))]
  check('both builds retain the same source and declared seed/epoch/environment', builds.every(build =>
    build.identity.sources.some(row => row.source === '/project/' + test && row.sourceSha256 === testSourceSha256) &&
    build.options.epoch === 1700000000000 && build.options.seed === 42 &&
    JSON.stringify(build.options.env) === '{"NODE_ENV":"test"}' && build.options.vitest.sequence.seed === 42))
  assert.deepEqual(effects(currentData), effects(oldData), '47 names/states/files/console/virtual-time effects differ')
  check('complete logical test effects agree', true)
  result.effectsSha256 = sha(JSON.stringify(effects(currentData)))
  const phase = value => value.phases.find(row => row.name === 'managed')
  result.seconds = {candidateWorker: phase(current).seconds, legacyWorker: phase(old).seconds,
    candidateOrdinary: current.ordinary.seconds}
  check('both retained worker timings are finite positive durations',
    [result.seconds.candidateWorker, result.seconds.legacyWorker].every(value => Number.isFinite(value) && value > 0))
  result.workerRatio = result.seconds.candidateWorker / result.seconds.legacyWorker
  result.rawEvidence = {}
  for (const [label, directory] of [['candidate', candidate], ['legacy', legacy]]) {
    result.rawEvidence[label] = Object.fromEntries(['result.json', 'test-build.json', 'managed/invocation.json',
      'managed/execution.json', 'managed/worker.mjs', 'managed/config.capnp', 'managed/response.json'].map(file =>
      [file, sha(fs.readFileSync(path.join(directory, file)))]))
  }
  result.sourceFiles = {}
  for (const [label, directory] of [['candidate', source], ['legacy', legacySource]]) {
    const sourceOut = path.join(out, 'source-' + label)
    fs.mkdirSync(sourceOut)
    result.sourceFiles[label] = {}
    for (const name of ['prelude.mjs', 'compiler.mjs', 'runner.mjs', 'executor.mjs', 'receipt.mjs', 'preparation.mjs', 'package-lock.json']) {
      const bytes = fs.readFileSync(path.join(directory, 'scripts/bench-vitest/managed', name))
      fs.writeFileSync(path.join(sourceOut, name), bytes)
      result.sourceFiles[label][name] = sha(bytes)
    }
  }
  result.passed = true
} catch (error) {
  result.error = {name: error.name, message: error.message, stack: error.stack}
  process.exitCode = 1
} finally {
  save()
  console.log(JSON.stringify(result))
}
