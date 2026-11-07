// GitHub-only correctness investigation. No receipt admission or cache transport.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {fileURLToPath, pathToFileURL} from 'node:url'
import {spawn} from 'node:child_process'
import {createManagedCompiler, canonicalManagedJSON} from './compiler.mjs'
import {prepareManagedMatrix, describeManagedSpecification, canonicalizeManagedAliasReplacement} from './preparation.mjs'
import {createManagedMatrixPlan, matrixInstanceKey, compareManagedMatrixStates} from './matrix-planner.mjs'
import {createManagedExecutor} from './executor.mjs'
import {MANAGED_BOUNDARY_CONTRACT, MANAGED_INTRINSIC_POLICY} from './receipt.mjs'

const self = fileURLToPath(import.meta.url)
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const errorData = error => ({name: String(error?.name || 'Error'), message: String(error?.message || error),
  stack: String(error?.stack || '')})
const states = {passed: 'pass', failed: 'fail', skipped: 'skip', todo: 'todo', pending: 'pending', queued: 'queued'}
const base = {epoch: 1700000000000, seed: 42, env: {NODE_ENV: 'test'}}
const selectionKeys = rows => rows.map(row => canonicalManagedJSON([path.resolve(row.absolute), row.project])).sort()
const nativeErrors = errors => (errors || []).map(errorData)

// Vitest 5 public reporting APIs are pinned in this investigation:
// TestModule.children.allTests/allSuites; TestCase.parent/name/options/result;
// TestModule.project/moduleId/relativeModuleId/state/errors/ok. Native JSON is
// retained separately, without manufacturing nested-suite or timing metadata.
function nativeModuleData(module, project, selectedByKey) {
  const descriptor = {absolute: path.resolve(module.moduleId), project: module.project.name}
  const selected = selectedByKey.get(selectionKeys([descriptor])[0])
  assert(selected, 'native executed an unselected project/file instance')
  const ancestors = test => {
    const result = []
    for (let owner = test.parent; owner && owner.type !== 'module'; owner = owner.parent) result.unshift(owner.name)
    return result
  }
  const rows = Array.from(module.children.allTests(), test => {
    const result = test.result(), ancestorTitles = ancestors(test)
    assert(['passed', 'failed', 'skipped', 'pending'].includes(result.state), 'unknown native test state')
    return {name: [...ancestorTitles, test.name].join(' '),
      state: result.state === 'skipped' && test.options.mode === 'todo' ? 'todo' : states[result.state],
      title: test.name, ancestorTitles, displayFullName: test.fullName, mode: test.options.mode,
      errors: nativeErrors(result.errors)}
  })
  const suites = Array.from(module.children.allSuites(), suite => ({name: suite.name,
    ancestorTitles: ancestors(suite), state: suite.state(), mode: suite.options.mode,
    errors: nativeErrors(suite.errors())}))
  return {instanceId: selected.instanceId, file: path.relative(project, descriptor.absolute).split(path.sep).join('/'),
    absolute: descriptor.absolute, project: descriptor.project, pool: selected.pool,
    relativeModuleId: module.relativeModuleId, state: states[module.state()] || module.state(), ok: module.ok(),
    errors: nativeErrors(module.errors()), providedContext: module.project.getProvidedContext(), rows, suites}
}

async function nativeChild(requestFile) {
  const request = read(requestFile)
  assert.equal(request.schema, 'quod-managed-matrix-native-request-v1')
  const project = fs.realpathSync(request.project)
  assert.equal(fs.realpathSync(process.cwd()), project)
  const evidence = {schema: 'quod-managed-matrix-native-v1', selected: [], executed: [],
    matched: false, runStarted: false, completed: false, unhandledErrors: [], failedModules: 0}
  let ctx
  try {
    process.env.TEST = 'true'
    process.env.VITEST = 'true'
    process.env.NODE_ENV ??= 'test'
    const require = createRequire(path.join(project, 'package.json'))
    assert.equal(require('vitest/package.json').version, '5.0.0')
    const {createVitest} = await import(pathToFileURL(require.resolve('vitest/node')).href)
    ctx = await createVitest({root: project, watch: false, run: true,
      reporters: ['default', 'json'], outputFile: {json: request.outputFile}})
    await ctx.standalone()
    const specs = await ctx.globTestSpecifications([])
    evidence.selected = specs.map(specification => describeManagedSpecification(specification, project))
    if (request.selected) assert.deepEqual(selectionKeys(evidence.selected), selectionKeys(request.selected),
      'complete native selection differs from actual matrix preparation')
    const selectedByKey = new Map(evidence.selected.map(row => [selectionKeys([row])[0], row]))
    assert.equal(selectedByKey.size, evidence.selected.length, 'duplicate native project/file instance')
    await ctx.cache.stats.populateStats(ctx.config.root, specs)
    evidence.matched = true
    evidence.runStarted = true
    write(request.evidenceFile, evidence)
    const result = await ctx.runTestSpecifications(specs, false)
    evidence.executed = result.testModules.map(module => nativeModuleData(module, project, selectedByKey))
    evidence.unhandledErrors = nativeErrors(result.unhandledErrors)
    evidence.failedModules = result.testModules.filter(module => !module.ok()).length
    assert.deepEqual(selectionKeys(evidence.executed), selectionKeys(evidence.selected),
      'complete native execution omitted or repeated a project/file instance')
    const report = read(request.outputFile)
    assert(Array.isArray(report.testResults) && report.testResults.length === specs.length,
      'native JSON omitted an actual file/project instance')
    // Public per-project reporting rows must account for the unchanged JSON
    // reporter's complete full-name/status multiset, including project repeats.
    const apiRows = evidence.executed.flatMap(file => file.rows.map(row =>
      canonicalManagedJSON([file.absolute, row.name, row.state]))).sort()
    const jsonRows = report.testResults.flatMap(file => file.assertionResults.map(row =>
      canonicalManagedJSON([path.resolve(file.name), row.fullName, states[row.status] || row.status]))).sort()
    assert.deepEqual(apiRows, jsonRows, 'project-labelled native rows differ from native JSON reporter')
    if (evidence.unhandledErrors.length || evidence.failedModules || report.success !== true) process.exitCode = 1
    evidence.reportSha256 = sha(fs.readFileSync(request.outputFile))
    evidence.completed = true
  } catch (error) {
    process.exitCode = 1
    evidence.error = errorData(error)
    console.error(error.stack || error)
  } finally {
    if (ctx) {
      try { await ctx.close() }
      catch (error) { process.exitCode = 1; evidence.closeError = errorData(error) }
    }
    evidence.exitCode = Number(process.exitCode || 0)
    write(request.evidenceFile, evidence)
  }
  if (ctx) await ctx.exit()
}

async function runNative(project, out, selected) {
  const requestFile = path.join(out, 'native-request.json'), outputFile = path.join(out, 'native-report.json')
  const evidenceFile = path.join(out, 'native-selection.json'), logFile = path.join(out, 'native.log')
  write(requestFile, {schema: 'quod-managed-matrix-native-request-v1', project, selected: selected || null,
    outputFile, evidenceFile})
  const fd = fs.openSync(logFile, 'w'), started = performance.now()
  let timedOut = false, child, forceKill
  const watchdog = setTimeout(() => {
    timedOut = true
    child?.kill('SIGTERM')
    forceKill = setTimeout(() => child?.kill('SIGKILL'), 1000)
  }, 300000)
  try {
    const exit = await new Promise((resolve, reject) => {
      child = spawn(process.execPath, [self, '--native-child', requestFile], {cwd: project,
        env: {...process.env, QUOD_SHIM_ACTIVE: '1'}, stdio: ['ignore', 'pipe', 'pipe']})
      for (const stream of [child.stdout, child.stderr]) stream.on('data', bytes => {
        fs.writeSync(fd, bytes)
        process.stdout.write(bytes)
      })
      child.once('error', reject)
      child.once('close', (code, signal) => resolve({exitCode: code ?? 1, signal}))
    })
    const result = {...exit, timedOut, seconds: (performance.now() - started) / 1000,
      logFile, outputFile, evidenceFile}
    write(path.join(out, 'native-process.json'), result)
    assert(!timedOut, 'complete native matrix watchdog exhausted')
    return result
  } finally {
    clearTimeout(watchdog)
    clearTimeout(forceKill)
    fs.closeSync(fd)
  }
}

function completedBoundary(data) {
  const boundary = data?.boundary, completion = boundary?.completion
  return boundary?.contract === MANAGED_BOUNDARY_CONTRACT && boundary.intrinsicPolicy === MANAGED_INTRINSIC_POLICY &&
    completion?.sealed === true && completion.intrinsicLockdown === true && completion.reason === '' &&
    boundary.pendingTimers === 0 && ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'].every(key => completion[key] === 0) &&
    ['unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors'].every(key => Array.isArray(boundary[key]) && !boundary[key].length)
}
function supported(data) {
  return !data?.error && completedBoundary(data) && Array.isArray(data.boundary.unsupported) && !data.boundary.unsupported.length &&
    Array.isArray(data.rows) && data.rows.every(row => typeof row.name === 'string' && ['pass', 'fail', 'skip', 'todo'].includes(row.state) && Array.isArray(row.errors)) &&
    Array.isArray(data.files) && data.files.length === 1
}

async function probe(projectArg, outArg) {
  assert(projectArg && outArg, 'usage: matrix-probe.mjs PROJECT EMPTY_OUTPUT')
  const project = fs.realpathSync(path.resolve(projectArg)), out = path.resolve(outArg)
  fs.mkdirSync(out, {recursive: true})
  assert.equal(fs.readdirSync(out).length, 0, 'matrix output must be empty')
  process.chdir(project)
  const result = {schema: 'quod-managed-matrix-probe-v1-research', passed: false,
    productionEnabled: false, cacheEnabled: false, receiptAdmission: false,
    isolation: 'fresh workerd process per admitted actual project/file instance',
    node: process.version, inputs: base, instances: [], phases: [], checks: []}
  const save = () => write(path.join(out, 'result.json'), result)
  const check = (name, pass) => { result.checks.push({name, pass: Boolean(pass)}); save(); assert(pass, name) }
  const started = performance.now()
  let suite, stickyManagedFailure = false
  save()
  try {
    // Pure path controls exercise Vite's actual FS_PREFIX join convention and
    // reject lookalike checkout prefixes before resolving the real matrix.
    const fixtureProject = '/reviewed/checkout'
    for (const [name, input, expected] of [
      ['ordinary absolute alias path is canonical', fixtureProject + '/node_modules/vite/dist/client/env.mjs', '/project/node_modules/vite/dist/client/env.mjs'],
      ['Vite filesystem URL alias path is canonical', '/@fs' + fixtureProject + '/node_modules/vite/dist/client/client.mjs', '/@fs/project/node_modules/vite/dist/client/client.mjs'],
      ['unrelated alias string is preserved', 'zod-4.0', 'zod-4.0'],
    ]) check(name, canonicalizeManagedAliasReplacement(input, fixtureProject) === expected)
    for (const input of [fixtureProject + '-sibling/client.mjs', '/@fs' + fixtureProject + '-sibling/client.mjs',
      'arbitrary:' + fixtureProject + '/client.mjs']) {
      let rejected = false
      try { canonicalizeManagedAliasReplacement(input, fixtureProject) }
      catch (error) { rejected = error.name === 'AssertionError' && error.message === 'unreviewed checkout path in alias' }
      check('non-path or sibling checkout alias is rejected: ' + input, rejected)
    }
    suite = await prepareManagedMatrix({project, seed: base.seed})
    write(path.join(out, 'preparation.json'), suite.evidence)
    const plan = createManagedMatrixPlan(suite)
    write(path.join(out, 'matrix-plan.json'), plan)
    result.planKey = plan.key
    result.selected = plan.selected; result.prepared = plan.prepared; result.preparationDeclined = plan.declined
    const compile = createManagedCompiler({project})
    // These options must fail before esbuild starts, for the specific canonical
    // validation reason. They grant no alias/context or private adapter access.
    result.compilerValidationChecks = []
    for (const fixture of [
      {name: 'unreviewed alias is rejected', alias: 'zod-4.7', project: 'zod-4.7',
        context: {expectedZodAlias: 'zod-4.7'}, message: 'unreviewed managed Zod alias'},
      {name: 'alias and actual project must agree', alias: 'zod-4.0', project: 'zod-4.1',
        context: {expectedZodAlias: 'zod-4.0'}, message: 'Zod alias differs from actual project'},
      {name: 'alias and provided context must agree', alias: 'zod-4.0', project: 'zod-4.0',
        context: {expectedZodAlias: 'zod-4.1'}, message: 'Zod context differs from alias'},
      {name: 'root provided context cannot request an alias', project: 'convex-helpers',
        context: {expectedZodAlias: 'zod-4.0'}, message: 'unreviewed managed provided context'},
      {name: 'private alias strings cannot request compiler capabilities', alias: '__QUOD_TEST_ENTRY__',
        project: '__QUOD_TEST_ENTRY__', context: {expectedZodAlias: '__QUOD_TEST_ENTRY__'},
        message: 'unreviewed managed Zod alias'},
    ]) {
      let rejection
      try {
        await compile('runner.mjs', {...base, vitest: {name: fixture.project},
          providedContext: fixture.context, ...(fixture.alias ? {zodAlias: fixture.alias} : {})}, suite.selected[0].absolute)
      } catch (error) { rejection = errorData(error) }
      const pass = rejection?.name === 'AssertionError' && rejection.message.includes(fixture.message)
      result.compilerValidationChecks.push({name: fixture.name, pass, rejection: rejection || null})
      write(path.join(out, 'compiler-validation.json'), result.compilerValidationChecks)
      check(fixture.name, pass)
    }
    for (const [name, changed] of [
      ['planner rejects an extra prepared instance', {...suite, prepared: [...suite.prepared,
        {instanceId: 'unselected-prepared-instance'}]}],
      ['planner rejects an extra declined instance', {...suite, declined: [...suite.declined,
        {instanceId: 'unselected-declined-instance', reason: 'negative control'}]}],
    ]) {
      let rejected = false
      try { createManagedMatrixPlan(changed) } catch (error) { rejected = error.name === 'AssertionError' }
      check(name, rejected)
    }
    const executor = createManagedExecutor({out, watchdogMs: 30000, onPhase(row) {
      result.phases.push({name: row.name, seconds: row.seconds, responseSha256: row.responseSha256}); save()
    }})
    result.runtimePolicy = executor.runtimePolicy; result.host = executor.host
    const preflight = await compile('preflight.mjs', base)
    write(path.join(out, 'preflight-build.json'), {...preflight, bundle: undefined, options: base})
    const first = await executor.execute('preflight-one', preflight.bundle)
    const second = await executor.execute('preflight-two', preflight.bundle)
    check('both fresh preflights retain all 31 capability checks and complete effects', [first, second].every(data =>
      data.ok === true && data.checks.length === 31 && data.checks.every(row => row.pass === true) &&
      new Set(data.checks.map(row => row.name)).size === 31 && completedBoundary(data)))
    check('fresh preflight random outputs and ambient inventories agree', canonicalManagedJSON(first.random) === canonicalManagedJSON(second.random) &&
      canonicalManagedJSON(first.boundary.inventory) === canonicalManagedJSON(second.boundary.inventory) &&
      canonicalManagedJSON(first.boundary.symbolInventory) === canonicalManagedJSON(second.boundary.symbolInventory))
    result.preflightPassed = true
    const prepared = new Map(suite.prepared.map(row => [row.instanceId, row]))
    const declined = new Map(suite.declined.map(row => [row.instanceId, row]))
    fs.mkdirSync(path.join(out, 'instances'))
    for (const selection of suite.selected) {
      const row = {instanceId: selection.instanceId, file: selection.file, project: selection.project,
        pool: selection.pool, decision: 'ordinary', supported: false}
      result.instances.push(row)
      const candidate = prepared.get(selection.instanceId)
      if (!candidate) { row.stage = 'preparation'; row.reason = declined.get(selection.instanceId).reason; save(); continue }
      const directory = path.join(out, 'instances', selection.instanceId)
      fs.mkdirSync(directory)
      row.evidenceDirectory = 'instances/' + selection.instanceId
      const options = {...base, vitest: candidate.config, file: '/project/' + candidate.file,
        providedContext: candidate.providedContext, ...(candidate.zodAlias ? {zodAlias: candidate.zodAlias} : {})}
      write(path.join(directory, 'preparation.json'), candidate.identity)
      row.stage = 'compilation'
      try {
        const built = await compile('runner.mjs', options, candidate.absolute)
        write(path.join(directory, 'test-build.json'), {...built, bundle: undefined, options, preparation: candidate.identity})
        row.compilationSeconds = built.seconds; row.bundleSha256 = built.sha256
        row.stage = 'execution'
        const instanceExecutor = createManagedExecutor({out: directory,
          watchdogMs: candidate.file.endsWith('/stream.property.test.ts') ? 180000 : 30000,
          onPhase(phase) { row.workerSeconds = phase.seconds; row.responseSha256 = phase.responseSha256; save() }})
        const data = await instanceExecutor.execute('managed', built.bundle)
        row.supported = supported(data)
        row.managedPassed = data.ok === true
        row.unsupported = data.boundary?.unsupported || []
        if (row.supported) {
          // Passed retry/expected-failure tasks may retain diagnostic errors.
          // Preserve the runner's actual fail state as the sticky outcome.
          const failed = data.rows.some(test => test.state === 'fail') || data.files.some(file => file.state === 'fail')
          stickyManagedFailure ||= failed
          row.decision = failed ? 'supported-failure' : data.ok === true ? 'managed' : 'ordinary'
          row.reason = failed ? 'supported managed test failed' : data.ok === true ? null : 'managed result did not declare a passing module'
        } else row.reason = 'closed boundary did not admit managed execution'
        row.stage = 'complete'
      } catch (error) { row.error = errorData(error); row.reason = error.message }
      save()
      console.log(JSON.stringify({instance: row.instanceId, project: row.project, file: row.file,
        decision: row.decision, supported: row.supported, workerSeconds: row.workerSeconds,
        reason: row.reason, unsupported: row.unsupported}))
    }
  } catch (error) { result.error = errorData(error); save() }
  // Always execute every original native spec, including all declined instances.
  // This is the control and the ordinary fallback authority for this probe.
  try {
    result.native = await runNative(project, out, suite?.selected)
    const native = read(path.join(out, 'native-selection.json'))
    check('complete native selection and reporting finished without errors', result.native.exitCode === 0 &&
      native.completed === true && native.matched === true && native.exitCode === 0 &&
      native.unhandledErrors.length === 0 && native.failedModules === 0)
    if (suite) {
      check('all actual project/file instances were retained', canonicalManagedJSON(selectionKeys(native.executed)) ===
        canonicalManagedJSON(selectionKeys(suite.selected)))
      const actual = new Map(native.executed.map(row => [matrixInstanceKey(row), row]))
      assert.equal(actual.size, native.executed.length)
      for (const row of result.instances) {
        const nativeInstance = actual.get(matrixInstanceKey(row))
        assert(nativeInstance, 'missing project-labelled native fallback instance')
        row.nativeCount = nativeInstance.rows.length
        row.nativeState = nativeInstance.state
        row.nativeExecuted = true
        if (!row.supported) continue
        const data = read(path.join(out, row.evidenceDirectory, 'managed/response.json'))
        row.comparison = compareManagedMatrixStates(data.rows, nativeInstance.rows)
        row.fileNameMatches = data.files[0].name === nativeInstance.relativeModuleId
        row.fileStateMatches = data.files[0].state === nativeInstance.state
      }
      result.coverage = {selected: suite.selected.length, prepared: suite.prepared.length,
        supported: result.instances.filter(row => row.supported).length,
        managedPassing: result.instances.filter(row => row.decision === 'managed').length,
        ordinary: result.instances.filter(row => row.decision === 'ordinary').length,
        supportedFailures: result.instances.filter(row => row.decision === 'supported-failure').length,
        disagreements: result.instances.filter(row => row.supported && (!row.comparison?.equal || !row.fileNameMatches || !row.fileStateMatches)).length,
        nativeAssertions: native.executed.reduce((sum, row) => sum + row.rows.length, 0)}
      check('every prepared or declined specification remains visible and native-executed',
        result.instances.length === suite.selected.length && result.instances.every(row => row.nativeExecuted))
      check('every supported actual managed name/state multiset agrees per project/file', result.coverage.disagreements === 0)
      check('supported managed failures cannot be erased by ordinary execution', !stickyManagedFailure)
      check('at least one complete original managed instance passes', result.coverage.managedPassing > 0)
    }
    result.passed = !result.error && result.preflightPassed === true && result.checks.every(row => row.pass)
  } catch (error) { result.nativeError = errorData(error) }
  result.seconds = (performance.now() - started) / 1000
  save()
  console.log(JSON.stringify({schema: result.schema, passed: result.passed, coverage: result.coverage,
    seconds: result.seconds, error: result.error, nativeError: result.nativeError}))
  if (!result.passed) process.exitCode = 1
}

if (process.argv[2] === '--native-child') {
  nativeChild(process.argv[3]).catch(error => { console.error(error.stack || error); process.exitCode = 1 })
} else {
  probe(...process.argv.slice(2)).catch(error => { console.error(error.stack || error); process.exitCode = 1 })
}
