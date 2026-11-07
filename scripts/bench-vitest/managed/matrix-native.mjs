// Exact ordinary TestSpecification batches for the separate matrix research API.
// Every original actual project/file/pool identity is checked before filtering.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {fileURLToPath, pathToFileURL} from 'node:url'
import {spawn} from 'node:child_process'
import {canonicalManagedJSON} from './compiler.mjs'
import {describeManagedSpecification} from './preparation.mjs'

const self = fileURLToPath(import.meta.url)
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n')
const errorData = error => ({name: String(error?.name || 'Error'), message: String(error?.message || error), stack: String(error?.stack || '')})
const errors = value => (value || []).map(errorData)
const states = {passed: 'pass', failed: 'fail', skipped: 'skip', todo: 'todo', pending: 'pending'}
const ids = rows => rows.map(row => canonicalManagedJSON([row.instanceId, row.project, row.file, row.pool])).sort()

function moduleData(module, project, selected) {
  const absolute = path.resolve(module.moduleId)
  const matches = selected.filter(row => row.absolute === absolute && row.project === module.project.name)
  assert.equal(matches.length, 1, 'native module does not match one actual selected instance')
  const spec = matches[0]
  const ancestors = test => {
    const names = []
    for (let owner = test.parent; owner && owner.type !== 'module'; owner = owner.parent) names.unshift(owner.name)
    return names
  }
  const rows = Array.from(module.children.allTests(), test => {
    const result = test.result(), ancestorTitles = ancestors(test)
    assert(['passed', 'failed', 'skipped', 'pending'].includes(result.state))
    return {name: [...ancestorTitles, test.name].join(' '), title: test.name, ancestorTitles,
      displayFullName: test.fullName, mode: test.options.mode,
      state: result.state === 'skipped' && test.options.mode === 'todo' ? 'todo' : states[result.state], errors: errors(result.errors)}
  })
  return {instanceId: spec.instanceId, file: path.relative(project, absolute).split(path.sep).join('/'), absolute,
    project: module.project.name, pool: spec.pool, relativeModuleId: module.relativeModuleId,
    state: states[module.state()] || module.state(), ok: module.ok(), errors: errors(module.errors()),
    providedContext: module.project.getProvidedContext(), rows,
    suites: Array.from(module.children.allSuites(), suite => ({name: suite.name, ancestorTitles: ancestors(suite),
      state: suite.state(), mode: suite.options.mode, errors: errors(suite.errors())}))}
}

async function nativeChild(requestFile) {
  const request = read(requestFile), project = fs.realpathSync(request.project)
  assert.equal(request.schema, 'quod-managed-matrix-native-batch-request-v1')
  assert.equal(fs.realpathSync(process.cwd()), project)
  const data = {schema: 'quod-managed-matrix-native-v1', originalSelected: [], selected: [], executed: [],
    omittedAsHits: [], matched: false, runStarted: false, completed: false, unhandledErrors: [], failedModules: 0}
  let ctx
  try {
    process.env.TEST = 'true'; process.env.VITEST = 'true'; process.env.NODE_ENV ??= 'test'
    const require = createRequire(path.join(project, 'package.json'))
    assert.equal(require('vitest/package.json').version, '5.0.0')
    const {createVitest} = await import(pathToFileURL(require.resolve('vitest/node')).href)
    ctx = await createVitest({root: project, watch: false, run: true,
      reporters: ['default', 'json'], outputFile: {json: request.reportFile}})
    await ctx.standalone()
    const actual = await ctx.globTestSpecifications(request.filters || [])
    data.originalSelected = actual.map(spec => describeManagedSpecification(spec, project))
    assert.equal(new Set(data.originalSelected.map(row => row.instanceId)).size, data.originalSelected.length)
    if (request.selected) assert.deepEqual(ids(data.originalSelected), ids(request.selected), 'complete original selection differs from matrix preparation')
    const wanted = request.instanceIds === null ? new Set(data.originalSelected.map(row => row.instanceId)) : new Set(request.instanceIds)
    assert(wanted.size > 0 && (request.instanceIds === null || wanted.size === request.instanceIds.length), 'unique nonempty native batch IDs required')
    assert([...wanted].every(id => data.originalSelected.some(row => row.instanceId === id)), 'native batch requested an unselected instance')
    const specs = actual.filter((_, index) => wanted.has(data.originalSelected[index].instanceId))
    data.selected = data.originalSelected.filter(row => wanted.has(row.instanceId))
    data.omittedAsHits = data.originalSelected.filter(row => !wanted.has(row.instanceId))
    assert.equal(specs.length, wanted.size)
    data.matched = true
    await ctx.cache.stats.populateStats(ctx.config.root, specs)
    data.runStarted = true; write(request.evidenceFile, data)
    const result = await ctx.runTestSpecifications(specs, false)
    data.executed = result.testModules.map(module => moduleData(module, project, data.selected))
    data.unhandledErrors = errors(result.unhandledErrors)
    data.failedModules = result.testModules.filter(module => !module.ok()).length
    assert.deepEqual(ids(data.executed), ids(data.selected), 'exact ordinary batch omitted/repeated a specification')
    const report = read(request.reportFile)
    assert(Array.isArray(report.testResults) && report.testResults.length === data.selected.length)
    const api = data.executed.flatMap(file => file.rows.map(row => canonicalManagedJSON([file.absolute, row.name, row.state]))).sort()
    const json = report.testResults.flatMap(file => file.assertionResults.map(row => canonicalManagedJSON([
      path.resolve(file.name), row.fullName, states[row.status] || row.status]))).sort()
    assert.deepEqual(api, json, 'actual ordinary public APIs differ from unchanged native JSON')
    data.reportSha256 = sha(fs.readFileSync(request.reportFile)); data.completed = true
    if (data.unhandledErrors.length || data.failedModules || report.success !== true) process.exitCode = 1
  } catch (error) {
    process.exitCode = 1; data.error = errorData(error); console.error(error.stack || error)
  } finally {
    if (ctx) {
      try { await ctx.close() } catch (error) {process.exitCode = 1; data.closeError = errorData(error)}
    }
    data.exitCode = Number(process.exitCode || 0); write(request.evidenceFile, data)
  }
  if (ctx) await ctx.exit()
}

export async function runMatrixNativeBatch({project: projectArg, out: outArg, selected, instanceIds,
  suiteFiles, node = process.execPath, watchdogMs = 300000}) {
  const project = fs.realpathSync(path.resolve(projectArg)), out = path.resolve(outArg)
  assert.equal(fs.realpathSync(node), fs.realpathSync(process.execPath), 'reviewed same native Node executable required')
  assert(Number.isSafeInteger(watchdogMs) && watchdogMs > 0)
  assert(selected === undefined || Array.isArray(selected))
  assert(suiteFiles === undefined || Array.isArray(suiteFiles) && suiteFiles.every(file => typeof file === 'string'))
  assert(instanceIds === undefined || Array.isArray(instanceIds) && instanceIds.every(id => /^[a-f0-9]{64}$/.test(id)))
  fs.mkdirSync(out, {recursive: true}); assert.equal(fs.readdirSync(out).length, 0, 'native batch output must be empty')
  if (instanceIds?.length === 0) return {skipped: true, exitCode: 0, seconds: 0, selected: [], executed: []}
  const requestFile = path.join(out, 'request.json'), reportFile = path.join(out, 'native-report.json')
  const evidenceFile = path.join(out, 'native-selection.json'), logFile = path.join(out, 'native.log')
  write(requestFile, {schema: 'quod-managed-matrix-native-batch-request-v1', project,
    selected: selected || null, instanceIds: instanceIds || null,
    filters: suiteFiles?.map(file => path.resolve(project, file)) || null, reportFile, evidenceFile})
  const fd = fs.openSync(logFile, 'w'), started = performance.now()
  let child, timedOut = false, forceKill
  const watchdog = setTimeout(() => {
    timedOut = true; child?.kill('SIGTERM')
    forceKill = setTimeout(() => child?.kill('SIGKILL'), 1000)
  }, watchdogMs)
  try {
    const exit = await new Promise((resolve, reject) => {
      child = spawn(node, [self, '--child', requestFile], {cwd: project,
        env: {...process.env, QUOD_SHIM_ACTIVE: '1'}, stdio: ['ignore', 'pipe', 'pipe']})
      for (const stream of [child.stdout, child.stderr]) stream.on('data', bytes => {fs.writeSync(fd, bytes); process.stdout.write(bytes)})
      child.once('error', reject)
      child.once('close', (code, signal) => resolve({exitCode: code ?? 1, signal}))
    })
    const result = {...exit, timedOut, seconds: (performance.now() - started) / 1000, logFile, reportFile, evidenceFile,
      data: fs.existsSync(evidenceFile) ? read(evidenceFile) : null}
    write(path.join(out, 'process.json'), {...result, data: undefined})
    return result
  } finally {clearTimeout(watchdog); clearTimeout(forceKill); fs.closeSync(fd)}
}

if (process.argv[2] === '--child') nativeChild(process.argv[3]).catch(error => {console.error(error.stack || error); process.exitCode = 1})
