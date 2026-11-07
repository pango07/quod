// Trusted native Node child for an exact Vitest 5 file/project selection.
// CLI exclusions can be replaced by individual project configurations. Keep
// the original TestSpecification objects instead of translating them to globs.
import fs from 'node:fs'
import path from 'node:path'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {pathToFileURL} from 'node:url'

const describe = specification => ({absolute: path.resolve(specification.moduleId),
  project: specification.project.config.name || ''})
const key = row => JSON.stringify([path.resolve(row.absolute), row.project || ''])
const keys = rows => rows.map(key).sort()
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n')

async function main() {
  const [requestFile] = process.argv.slice(2)
  assert(requestFile, 'ordinary remainder request required')
  const request = JSON.parse(fs.readFileSync(requestFile, 'utf8'))
  assert.equal(request.schema, 'quod-ordinary-remainder-request-v1')
  const project = fs.realpathSync(request.project)
  assert.equal(fs.realpathSync(process.cwd()), project, 'ordinary remainder cwd differs')
  assert(Array.isArray(request.selected) && request.selected.length > 1)
  assert(Array.isArray(request.filters) && request.filters.every(value => typeof value === 'string'))
  assert(Array.isArray(request.reporters) && request.reporters.includes('json') &&
    request.reporters.every(value => value === 'default' || value === 'json'))
  assert(path.isAbsolute(request.outputFile) && path.isAbsolute(request.evidenceFile))
  const evidence = {schema: 'quod-ordinary-remainder-selection-v1', selected: [], remainder: [], excluded: [],
    executed: [], matched: false, runStarted: false, completed: false}
  let ctx
  try {
    // Match the CLI's prepareVitest environment before resolving configuration.
    process.env.TEST = 'true'
    process.env.VITEST = 'true'
    process.env.NODE_ENV ??= 'test'
    const require = createRequire(path.join(project, 'package.json'))
    assert.equal(require('vitest/package.json').version, '5.0.0')
    const {createVitest} = await import(pathToFileURL(require.resolve('vitest/node')).href)
    ctx = await createVitest({root: project, watch: false, run: true,
      reporters: request.reporters, outputFile: {json: request.outputFile}})
    // This public API initializes the same coverage provider and onInit reporter
    // hooks as start(). runTestSpecifications then retains native pools, setup,
    // aliases, per-project configuration, reporting, and failure accounting.
    await ctx.standalone()
    const specifications = await ctx.globTestSpecifications(request.filters)
    evidence.selected = specifications.map(describe)
    assert.deepEqual(keys(evidence.selected), keys(request.selected),
      'native ordinary selection differs from actual preparation')
    const remainder = specifications.filter(specification => key(describe(specification)) !== key(request.excluded))
    evidence.excluded = specifications.filter(specification => key(describe(specification)) === key(request.excluded)).map(describe)
    assert.equal(evidence.excluded.length, 1, 'ordinary remainder must exclude exactly one actual file/project instance')
    evidence.remainder = remainder.map(describe)
    assert.equal(remainder.length, specifications.length - 1)
    // start() populates this public cache before runFiles; retain the native
    // sequencer's file-size inputs for the exact subset too.
    await ctx.cache.stats.populateStats(ctx.config.root, remainder)
    evidence.matched = true
    evidence.runStarted = true
    write(request.evidenceFile, evidence)
    const result = await ctx.runTestSpecifications(remainder, false)
    evidence.executed = result.testModules.map(describe)
    evidence.unhandledErrors = result.unhandledErrors.length
    evidence.failedModules = result.testModules.filter(module => !module.ok()).length
    const report = JSON.parse(fs.readFileSync(request.outputFile, 'utf8'))
    assert(Array.isArray(report.testResults), 'ordinary JSON reporter omitted testResults')
    if (result.unhandledErrors.length || evidence.failedModules || report.success !== true) process.exitCode = 1
    assert.deepEqual(keys(evidence.executed), keys(evidence.remainder),
      'native ordinary execution omitted or repeated selected file/project instances')
    evidence.completed = true
  } catch (error) {
    process.exitCode = 1
    evidence.error = {name: String(error?.name || 'Error'), message: String(error?.message || error)}
    console.error(error.stack || error)
  } finally {
    if (ctx) {
      try { await ctx.close() }
      catch (error) {
        process.exitCode = 1
        evidence.closeError = {name: String(error?.name || 'Error'), message: String(error?.message || error)}
        console.error(error.stack || error)
      }
    }
    evidence.exitCode = Number(process.exitCode || 0)
    write(request.evidenceFile, evidence)
  }
  // Match the CLI's final cleanup and its native teardown-timeout handling.
  if (ctx) await ctx.exit()
}

main().catch(error => { console.error(error.stack || error); process.exitCode = 1 })
