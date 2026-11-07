// Run only in the GitHub gate. No reusable cache is written by this probe.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import net from 'node:net'
import {createRequire} from 'node:module'
import {fileURLToPath} from 'node:url'
import {spawn, spawnSync} from 'node:child_process'
import {createManagedCompiler} from './compiler.mjs'
import {prepareManagedProject} from './preparation.mjs'

const [projectArg, outArg] = process.argv.slice(2)
const project = path.resolve(projectArg), out = path.resolve(outArg)
assert(!fs.existsSync(out)); fs.mkdirSync(out, {recursive: true})
// Vitest resolves project patterns and CLI filters relative to cwd, just as
// its ordinary CLI does. Setting root alone does not reproduce that context.
process.chdir(project)
const here = path.dirname(fileURLToPath(import.meta.url)), require = createRequire(import.meta.url)
const workerd = path.join(path.dirname(require.resolve('workerd/package.json')), 'bin/workerd')
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const write = (name, value) => fs.writeFileSync(path.join(out, name), JSON.stringify(value, null, 2) + '\n')
const base = {epoch: 1700000000000, seed: 42, env: {NODE_ENV: 'test'}}
const results = {scope: 'Managed runtime boundary and unchanged-test feasibility; no cache', cacheEnabled: false,
  boundaryChecksVersion: 4, node: process.version, image: `${process.env.ImageOS}/${process.env.ImageVersion}`,
  workerdSha256: sha(fs.readFileSync(workerd)), phases: []}
write('result.json', results)

async function freePort() {
  const server = net.createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise(resolve => server.close(resolve))
  return port
}
async function execute(name, bundle) {
  const dir = path.join(out, name); fs.mkdirSync(dir)
  fs.writeFileSync(path.join(dir, 'worker.mjs'), bundle)
  fs.writeFileSync(path.join(dir, 'deny.mjs'), `export default {fetch(){console.error('QUOD_DENIED_OUTBOUND');throw new Error('outbound denied')}};\n`)
  const port = await freePort()
  fs.writeFileSync(path.join(dir, 'config.capnp'), `using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
  services = [
    (name = "main", worker = (modules = [(name = "worker.mjs", esModule = embed "worker.mjs")],
      compatibilityDate = "2026-07-01", compatibilityFlags = ["nodejs_als"], globalOutbound = "deny")),
    (name = "deny", worker = (modules = [(name = "deny.mjs", esModule = embed "deny.mjs")],
      compatibilityDate = "2026-07-01", globalOutbound = "deny"))
  ],
  sockets = [(name = "http", address = "127.0.0.1:${port}", http = (), service = "main")]
);\n`)
  const log = fs.openSync(path.join(dir, 'runtime.log'), 'w'), started = performance.now()
  const child = spawn(workerd, ['serve', path.join(dir, 'config.capnp')], {
    stdio: ['ignore', log, log], env: {PATH: process.env.PATH, TZ: 'UTC', LANG: 'C.UTF-8'},
  })
  let exit
  const exited = new Promise(resolve => child.once('exit', (code, signal) => { exit = {code, signal}; resolve(exit) }))
  try {
    let ready = false
    for (let i = 0; i < 80 && !exit; i++) {
      try { const r = await fetch(`http://127.0.0.1:${port}/health`, {signal: AbortSignal.timeout(500)}); ready = r.ok } catch {}
      if (ready) break
      await new Promise(resolve => setTimeout(resolve, 100))
    }
    assert(ready, 'workerd did not become ready; inspect runtime.log')
    const response = await fetch(`http://127.0.0.1:${port}/run`, {signal: AbortSignal.timeout(180000)})
    const text = await response.text(); fs.writeFileSync(path.join(dir, 'response.json'), text)
    assert(response.ok, `workerd returned HTTP ${response.status}`)
    const data = JSON.parse(text)
    const row = {name, seconds: (performance.now() - started) / 1000, responseSha256: sha(text), data}
    results.phases.push(row); write('result.json', results)
    console.log(JSON.stringify({phase: name, seconds: row.seconds, ok: data.ok, unsupported: data.boundary?.unsupported}))
    return data
  } finally {
    if (!exit) child.kill('SIGTERM')
    await exited; fs.closeSync(log)
  }
}
// Compatibility is deliberately stricter than a green test report. No field
// below grants replay admission; the shared Vitest realm is still under review.
const supported = data => !data.error && data.boundary?.completion?.sealed === true &&
  data.boundary.unsupported.length === 0 && data.boundary.unguardable.length === 0 &&
  data.boundary.timerErrors.length === 0 && data.boundary.unhandledErrors.length === 0 &&
  data.boundary.pendingTimers === 0 && data.boundary.completion.pendingTimerCallbacks === 0 &&
  data.boundary.completion.pendingMicrotasks === 0 && data.boundary.completion.pendingEventCallbacks === 0
const normalizedRows = rows => {
  const states = {pass: 'passed', fail: 'failed', skip: 'skipped', todo: 'todo'}
  return rows.map(r => [r.name, states[r.state] || r.state]).sort()
}
const compile = createManagedCompiler({project})

try {
  const compilerLog = fs.openSync(path.join(out, 'compiler-checks.log'), 'w')
  let compilerCheck
  try {
    compilerCheck = spawnSync(process.execPath, [path.join(here, 'compiler-fixtures/check.mjs'), project, path.join(out, 'compiler-checks')],
      {cwd: project, stdio: ['ignore', compilerLog, compilerLog], timeout: 60000})
  } finally { fs.closeSync(compilerLog) }
  assert.equal(compilerCheck.status, 0, 'closed compiler regression gate failed; inspect compiler-checks.log')
  results.compilerChecksPassed = true
  write('result.json', results)
  const fixture = await compile('preflight.mjs', base)
  write('preflight-build.json', {...fixture, bundle: undefined})
  const one = await execute('preflight-one', fixture.bundle)
  const two = await execute('preflight-two', fixture.bundle)
  assert(one.ok && two.ok, 'managed capability preflight failed')
  assert.deepEqual(one.random, two.random)
  assert.deepEqual(one.checks, two.checks)
  results.preflightPassed = true; write('result.json', results)

  const outbound = await compile('native-outbound.mjs', base)
  write('native-outbound-build.json', {...outbound, bundle: undefined})
  const native = await execute('native-outbound', outbound.bundle)
  assert(native.nativeOutboundError || native.nativeOutboundStatus >= 500)
  assert(fs.readFileSync(path.join(out, 'native-outbound/runtime.log'), 'utf8').includes('QUOD_DENIED_OUTBOUND'),
    'native fetch must reach the configured deny service')
  results.nativeOutboundDenied = true; write('result.json', results)

  const boundaryCases = JSON.parse(fs.readFileSync(path.join(here, 'boundary-fixture-cases.json')))
  results.boundaryFixtureEvidence = []
  for (const {mode, expected} of boundaryCases.cases) {
    const name = 'boundary-' + mode
    const options = {...base, env: {...base.env, [boundaryCases.environmentVariable]: mode}}
    const built = await compile(boundaryCases.entry, options)
    write(name + '-build.json', {...built, bundle: undefined, options})
    const data = await execute(name, built.bundle), boundary = data.boundary, completion = boundary?.completion
    assert(!data.error, name + ' fixture execution error')
    assert.equal(data.mode, mode)
    assert.equal(data.ok, expected.ok, name + ' result')
    assert(data.checks.length > 0 && data.checks.every(check => check.pass), name + ' checks')
    assert.equal(completion?.sealed, expected.sealed, name + ' completion')
    assert(completion.nativeTurnFences <= boundaryCases.completionBounds.nativeTurnFencesMax)
    assert(completion.timerCallbacks <= boundaryCases.completionBounds.timerCallbacksMax)
    for (const key of ['pendingTimers', 'pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks', 'timerErrors', 'unhandledErrors']) {
      const actual = key === 'pendingTimers' ? boundary.pendingTimers
        : key === 'timerErrors' || key === 'unhandledErrors' ? boundary[key].length : completion[key]
      if (Object.hasOwn(expected, key)) assert.equal(actual, expected[key], name + ' ' + key)
      if (Object.hasOwn(expected, key + 'Min')) assert(actual >= expected[key + 'Min'], name + ' ' + key + ' minimum')
    }
    if (expected.reasonIncludes) assert(completion.reason.includes(expected.reasonIncludes), name + ' reason')
    for (const key of ['unsupported', 'timerErrors', 'unhandledErrors']) {
      for (const substring of expected[key + 'Includes'] || expected[key + 'Include'] || []) {
        assert(boundary[key].some(value => value.includes(substring)), name + ' missing ' + substring)
      }
    }
    if (expected.forgedAbsent) assert(!Object.hasOwn(data, 'forged'), name + ' serializer forged receipt')
    if (expected.evidenceSentinel) assert.equal(data.evidence.sentinel, expected.evidenceSentinel)
    results.boundaryFixtureEvidence.push({mode, expected, checks: data.checks.length,
      sealed: completion.sealed, responseSha256: results.phases.at(-1).responseSha256})
    write('result.json', results)
  }
  results.boundaryFixtureChecksPassed = true

  const file = 'packages/convex-helpers/server/stream.property.test.ts'
  const prepared = await prepareManagedProject({project, file, seed: base.seed})
  write('project-selection.json', prepared.evidence)
  const config = prepared.config
  const testFile = path.join(project, file)
  results.testSourceSha256 = sha(fs.readFileSync(testFile))
  const options = {...base, vitest: config, file: '/project/' + file}

  const fixtureDir = path.join(project, '.quod-managed-fixtures')
  assert(!fs.existsSync(fixtureDir)); fs.mkdirSync(fixtureDir)
  const fixtureFile = path.join(fixtureDir, 'fixture.test.mjs')
  fs.copyFileSync(path.join(here, 'fixture.test.mjs'), fixtureFile)
  fs.writeFileSync(path.join(fixtureDir, 'vitest.config.mjs'),
    'export default {test:{include:[".quod-managed-fixtures/fixture.test.mjs"],environment:"node"}}\n')
  results.fixtureEvidence = []
  for (const mode of ['pass', 'failure', 'unsupported']) {
    const name = 'fixture-' + mode, env = {...base.env, QUOD_FIXTURE_MODE: mode}
    const fixtureOptions = {...base, env, vitest: structuredClone(config), file: '/project/.quod-managed-fixtures/fixture.test.mjs'}
    const built = await compile('runner.mjs', fixtureOptions, fixtureFile)
    write(name + '-build.json', {...built, bundle: undefined, options: fixtureOptions})
    let actual
    try { actual = await execute(name, built.bundle) }
    catch (error) { actual = {ok: false, error: {name: error?.name, message: error?.message}} }
    const isSupported = supported(actual)

    // These ordinary runs are controls. For unsupported mode this is also
    // the normal execution the dispatcher must select as its fallback.
    const report = path.join(out, name + '-ordinary.json'), log = fs.openSync(path.join(out, name + '-ordinary.log'), 'w')
    const started = performance.now()
    const ordinary = spawnSync(process.execPath, ['node_modules/vitest/vitest.mjs', 'run',
      '--config', path.join(fixtureDir, 'vitest.config.mjs'), '--reporter=json', '--outputFile.json=' + report],
      {cwd: project, env: {...process.env, ...env}, stdio: ['ignore', log, log], timeout: 30000})
    fs.closeSync(log)
    assert.equal(ordinary.status, mode === 'failure' ? 1 : 0, name + ' ordinary exit')
    const rows = JSON.parse(fs.readFileSync(report)).testResults.flatMap(f => f.assertionResults.map(a => [a.fullName, a.status])).sort()
    results.fixtureEvidence.push({mode, managedSupported: isSupported, managedPassed: actual.ok,
      fallbackExecuted: !isSupported, ordinaryExit: ordinary.status, ordinarySeconds: (performance.now() - started) / 1000,
      ordinaryReportSha256: sha(fs.readFileSync(report)), error: actual.error, rows})
    write('result.json', results)
    assert.equal(actual.rows?.length, 4, name + ' must execute all four real Vitest tests')
    assert.equal(isSupported, mode !== 'unsupported', name + ' support classification')
    assert.equal(actual.ok, mode !== 'failure', name + ' must preserve the actual test outcome')
    if (mode === 'unsupported') assert.deepEqual(actual.boundary.unsupported, ['global fetch'])
    assert.deepEqual(normalizedRows(actual.rows), rows, name + ' all names and states')
  }
  results.fixtureChecksPassed = true

  // Compilation checks alone cannot establish that a caught computed import
  // retains its flag. Exercise the emitted code with real Vitest as well.
  results.moduleFixtureEvidence = []
  const moduleDir = path.join(fixtureDir, 'modules'); fs.mkdirSync(moduleDir)
  for (const name of ['literal-import.test.mjs', 'literal-value.mjs', 'computed-import.test.mjs', 'evaluation.test.mjs', 'metadata.test.mjs']) {
    fs.copyFileSync(path.join(here, 'compiler-fixtures', name), path.join(moduleDir, name))
  }
  const moduleConfig = path.join(moduleDir, 'vitest.config.mjs')
  fs.writeFileSync(moduleConfig, 'export default {test:{include:[".quod-managed-fixtures/modules/*.test.mjs"],environment:"node"}}\n')
  for (const [mode, count, reason] of [['literal-import', 1, null], ['computed-import', 1, 'computed module import'],
    ['evaluation', 2, 'dynamic code evaluation'], ['metadata', 3, 'ambient module metadata']]) {
    const name = 'module-' + mode, source = path.join(moduleDir, mode + '.test.mjs')
    const moduleOptions = {...base, vitest: structuredClone(config), file: '/project/.quod-managed-fixtures/modules/' + mode + '.test.mjs'}
    const built = await compile('runner.mjs', moduleOptions, source)
    write(name + '-build.json', {...built, bundle: undefined, options: moduleOptions})
    const actual = await execute(name, built.bundle)
    const report = path.join(out, name + '-ordinary.json'), log = fs.openSync(path.join(out, name + '-ordinary.log'), 'w')
    let ordinary
    const started = performance.now()
    try {
      ordinary = spawnSync(process.execPath, ['node_modules/vitest/vitest.mjs', 'run', source,
        '--config', moduleConfig, '--reporter=json', '--outputFile.json=' + report],
        {cwd: project, env: {...process.env, ...base.env}, stdio: ['ignore', log, log], timeout: 30000})
    } finally { fs.closeSync(log) }
    assert.equal(ordinary.status, 0, name + ' ordinary control failed')
    const rows = JSON.parse(fs.readFileSync(report)).testResults.flatMap(f => f.assertionResults.map(a => [a.fullName, a.status])).sort()
    const evidence = {mode, managedSupported: supported(actual), managedPassed: actual.ok, rows,
      ordinaryExit: ordinary.status, ordinarySeconds: (performance.now() - started) / 1000,
      ordinaryReportSha256: sha(fs.readFileSync(report)), fallbackExecuted: !supported(actual)}
    results.moduleFixtureEvidence.push(evidence); write('result.json', results)
    assert.equal(actual.ok, true, name + ' must preserve passing outcome')
    assert.equal(actual.rows.length, count, name + ' count')
    assert.equal(supported(actual), !reason, name + ' support classification')
    assert.deepEqual(actual.boundary.unsupported, reason ? [reason] : [])
    assert.deepEqual(normalizedRows(actual.rows), rows, name + ' names and states')
  }
  results.moduleFixtureChecksPassed = true

  const compiled = await compile('runner.mjs', options, testFile)
  write('test-build.json', {...compiled, bundle: undefined, options, preparation: prepared.identity})
  const managed = await execute('managed', compiled.bundle)
  results.managedSupported = supported(managed)
  results.managedPassed = Boolean(managed.ok)
  results.managedRows = managed.rows || []
} catch (error) {
  results.unsupportedOrSetupError = {name: error?.name, message: error?.message, stack: error?.stack}
  results.managedSupported = false
} finally {
  // Do not spend another expensive ordinary run debugging a small fixture.
  // The selected real file has not been attempted if these gates failed;
  // the fixture's own ordinary control/fallback above is the matching one.
  if (!results.fixtureChecksPassed || !results.moduleFixtureChecksPassed) {
    results.expensiveTestAttempted = false
    write('result.json', results)
    console.log(JSON.stringify({...results, phases: results.phases.map(p => ({name: p.name, seconds: p.seconds, ok: p.data.ok}))}))
    throw new Error('Managed fixture gate failed before the expensive test; inspect retained evidence')
  }
  results.expensiveTestAttempted = true
  // The ordinary run is mandatory and is the fallback for unsupported attempts.
  const report = path.join(out, 'ordinary-report.json'), log = fs.openSync(path.join(out, 'ordinary.log'), 'w')
  const started = performance.now()
  const plain = spawnSync(process.execPath, ['node_modules/vitest/vitest.mjs', 'run',
    'packages/convex-helpers/server/stream.property.test.ts', '--reporter=json', '--outputFile.json=' + report],
    {cwd: project, env: process.env, stdio: ['ignore', log, log], timeout: 180000})
  fs.closeSync(log)
  results.ordinary = {exit: plain.status, seconds: (performance.now() - started) / 1000, error: plain.error?.message}
  if (fs.existsSync(report)) {
    const bytes = fs.readFileSync(report), data = JSON.parse(bytes)
    const rows = data.testResults.flatMap(f => f.assertionResults.map(a => [a.fullName, a.status])).sort()
    results.ordinary.reportSha256 = sha(bytes); results.ordinary.rows = rows
    if (results.managedRows) {
      const managedRows = normalizedRows(results.managedRows)
      results.namesAndStatesAgree = JSON.stringify(rows) === JSON.stringify(managedRows)
    }
  }
  results.fallbackExecuted = !results.managedSupported
  write('result.json', results)
  console.log(JSON.stringify({...results, phases: results.phases.map(p => ({name: p.name, seconds: p.seconds, ok: p.data.ok})), managedRows: undefined, ordinary: {...results.ordinary, rows: undefined}}))
  assert.equal(results.ordinary.exit, 0, 'ordinary control failed')
  assert.equal(results.ordinary.rows?.length, 47, 'ordinary test count changed')
  assert.equal(results.preflightPassed, true, 'capability gate failed; no managed eligibility')
  assert.equal(results.nativeOutboundDenied, true, 'runtime outbound gate failed')
  assert.equal(results.fixtureChecksPassed, true, 'real runner failure/fallback gate failed')
  assert.equal(results.managedSupported, true, 'managed execution did not complete under the hardened contract')
  assert.equal(results.namesAndStatesAgree, true, 'managed states differ')
}
