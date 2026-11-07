// Research-only, fresh-runner receipt experiment. Not used by the Action.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {fileURLToPath} from 'node:url'
import {createManagedCompiler, canonicalManagedJSON} from './compiler.mjs'
import {prepareManagedProject} from './preparation.mjs'
import {createManagedExecutor} from './executor.mjs'
import {makeExecutionIdentity, validatePassingResponse, recordReceipt, lookupReceipt} from './receipt.mjs'

const [role, projectArg, outArg, receiptArg, trustedReceiptSha256, controlArg] = process.argv.slice(2)
assert(['seed', 'control', 'replay'].includes(role), 'role must be seed, control, or replay')
const project = fs.realpathSync(path.resolve(projectArg)), out = path.resolve(outArg)
assert(!fs.existsSync(out)); fs.mkdirSync(out, {recursive: true})
process.chdir(project)
const here = path.dirname(fileURLToPath(import.meta.url))
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const write = (name, data) => fs.writeFileSync(path.join(out, name), JSON.stringify(data, null, 2) + '\n')
const result = {schema: 'quod-managed-fresh-runner-v1-research', role,
  productionEnabled: false, phases: [], startedAt: new Date().toISOString()}
const started = performance.now()
write('result.json', result)
const executor = createManagedExecutor({out, watchdogMs: 300000, onPhase(row) {
  result.phases.push(row); write('result.json', result)
}})
const compile = createManagedCompiler({project})
const base = {epoch: 1700000000000, seed: 42, env: {NODE_ENV: 'test'}}
try {
  // Repeat capability validation on each actual consumer host. Compilation and
  // preparation are never skipped by a receipt hit.
  const preflightBuild = await compile('preflight.mjs', base)
  write('preflight-build.json', {...preflightBuild, bundle: undefined})
  const preflight = await executor.execute('preflight', preflightBuild.bundle)
  assert(preflight.ok && preflight.checks.every(check => check.pass), 'consumer runtime capability checks failed')
  assert(preflight.boundary.completion.sealed && !preflight.boundary.unguardable.length)

  const file = 'packages/convex-helpers/server/stream.property.test.ts'
  const testFile = path.join(project, file)
  assert.equal(sha(fs.readFileSync(testFile)), '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75',
    'unchanged expensive workload required')
  const prepared = await prepareManagedProject({project, file, seed: base.seed})
  write('project-selection.json', prepared.evidence)
  const options = {...base, vitest: prepared.config, file: '/project/' + file}
  const built = await compile('runner.mjs', options, testFile)
  write('test-build.json', {...built, bundle: undefined, options, preparation: prepared.identity})
  const expected = {count: 47, file: path.posix.relative(prepared.config.root, options.file)}
  const execution = makeExecutionIdentity({built, prepared, runtimePolicy: {...executor.runtimePolicy, nativeFingerprintSha256: sha(canonicalManagedJSON({
      preflightIdentity: preflightBuild.identitySha256, inventory: preflight.boundary.inventory,
      symbols: preflight.boundary.symbolInventory}))},
    supervisorPolicy: {receiptValidatorSha256: sha(fs.readFileSync(path.join(here, 'receipt.mjs'))),
      experimentSha256: sha(fs.readFileSync(fileURLToPath(import.meta.url)))}, host: executor.host})
  write('execution-identity.json', execution)
  result.executionKey = execution.key
  result.host = executor.host
  result.preparationSeconds = prepared.evidence.seconds
  result.compilationSeconds = built.seconds
  result.preReplaySeconds = (performance.now() - started) / 1000
  let data
  if (role === 'replay') {
    assert(receiptArg && trustedReceiptSha256 && controlArg, 'replay needs receipt, trusted SHA and independent control')
    const lookupStarted = performance.now()
    const lookup = lookupReceipt({file: path.resolve(receiptArg), execution, trustedReceiptSha256, expected})
    result.lookupSeconds = (performance.now() - lookupStarted) / 1000
    result.hit = lookup.hit
    result.missReason = lookup.reason
    // Retain an actual managed fallback when admission declines. A green
    // experiment still requires a genuine hit; fallback is never called replay.
    data = lookup.hit ? lookup.data : await executor.execute('managed-miss', built.bundle)
    if (lookup.hit) fs.writeFileSync(path.join(out, 'replayed-response.json'), lookup.responseBytes)
    result.replayed = lookup.hit
    result.actualManagedExecutions = lookup.hit ? 0 : 1
  } else {
    data = await executor.execute('managed', built.bundle)
    result.actualManagedExecutions = 1
    result.replayed = false
  }
  validatePassingResponse(data, expected)
  // Compare complete logical results and required console effects. Timing and
  // private completion-loop counters are diagnostics, not published effects.
  const effects = {ok: data.ok, rows: data.rows, files: data.files,
    logs: data.boundary.logs, virtualTime: data.boundary.virtualTime}
  result.effects = effects
  result.effectSha256 = sha(canonicalManagedJSON(effects))
  write('selected-response.json', data)
  if (role === 'seed') {
    const receipt = recordReceipt({file: path.join(out, 'receipt.json'), execution,
      responseBytes: fs.readFileSync(path.join(out, 'managed/response.json')), expected})
    result.receipt = receipt
    if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT,
      `receipt_sha256=${receipt.receiptSha256}\neffect_sha256=${result.effectSha256}\n`)
  }
  if (role === 'replay') {
    const control = JSON.parse(fs.readFileSync(path.resolve(controlArg)))
    assert.equal(control.role, 'control')
    assert.equal(control.passed, true, 'uncached managed control incomplete')
    assert.equal(control.replayed, false)
    assert.equal(control.actualManagedExecutions, 1)
    assert.equal(control.executionKey, result.executionKey, 'uncached control native/input class differs')
    assert.deepEqual(effects, control.effects, 'replay differs from independent uncached managed control')
    assert.equal(result.hit, true, 'no genuine fresh-runner hit; inspect miss reason')
    result.namesStatesAndEffectsAgree = true
  }
  result.passed = true
} catch (error) {
  result.passed = false
  result.error = {name: error.name, message: error.message, stack: error.stack}
  process.exitCode = 1
} finally {
  result.commandSeconds = (performance.now() - started) / 1000
  result.finishedAt = new Date().toISOString()
  write('result.json', result)
  console.log(JSON.stringify({...result, phases: result.phases.map(p => ({name: p.name, seconds: p.seconds})), effects: undefined}))
}
