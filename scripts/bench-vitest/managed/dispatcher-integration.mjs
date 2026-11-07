// GitHub runner only: exercise the actual dispatcher with tiny real Vitest
// files inside the pinned project, preserving its reviewed preparation path.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {fileURLToPath} from 'node:url'
import {spawnSync} from 'node:child_process'
import {canonicalReceiptJSON} from './receipt.mjs'

const [projectArg, outArg] = process.argv.slice(2)
assert(projectArg && outArg, 'usage: dispatcher-integration.mjs PROJECT EMPTY_OUTPUT_DIRECTORY')
const project = fs.realpathSync(path.resolve(projectArg)), out = path.resolve(outArg)
assert(!fs.existsSync(out)); fs.mkdirSync(out, {recursive: true})
const here = path.dirname(fileURLToPath(import.meta.url))
const cache = path.join(out, 'cache'), authority = path.join(out, 'authority')
fs.mkdirSync(cache); fs.mkdirSync(authority)
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const write = (name, value) => fs.writeFileSync(path.join(out, name), JSON.stringify(value, null, 2) + '\n')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const first = 'packages/convex-helpers/server/quod-managed-dispatcher.fixture.test.ts'
const second = 'packages/convex-helpers/server/quod-managed-dispatcher.second.test.ts'
const firstName = 'managed dispatcher preserves the selected test outcome'
const secondName = 'managed dispatcher also executes the second selected file'
const fixturePaths = [first, second].map(file => path.join(project, file))
for (const file of fixturePaths) assert(!fs.existsSync(file), 'fixture target already exists')
fs.copyFileSync(path.join(here, 'dispatcher-fixture.test.ts'), fixturePaths[0])
fs.copyFileSync(path.join(here, 'dispatcher-second.test.ts'), fixturePaths[1])
const result = {schema: 'quod-managed-dispatcher-integration-v1', passed: false,
  node: process.version, image: `${process.env.ImageOS}/${process.env.ImageVersion}`,
  projectFixtureFiles: [first, second], fixtureSourceSha256: fixturePaths.map(file => sha(fs.readFileSync(file))),
  cases: [], checks: []}
const check = (name, pass) => { result.checks.push({name, pass: Boolean(pass)}); write('result.json', result); assert(pass, name) }
write('result.json', result)

function jsonFiles(directory) {
  const files = []
  for (const entry of fs.readdirSync(directory, {withFileTypes: true})) {
    const file = path.join(directory, entry.name)
    if (entry.isDirectory()) files.push(...jsonFiles(file))
    else if (entry.isFile() && file.endsWith('.json')) files.push(file)
  }
  return files
}
function emittedReceipts(directory) {
  return jsonFiles(directory).filter(file => {
    try { return read(file).schema === 'quod-managed-receipt-v1-research' } catch { return false }
  })
}
function ordinaryRows(report) {
  return report.testResults.flatMap(file => file.assertionResults.map(test => [test.fullName, test.status])).sort()
}
function invoke({name, mode = 'pass', decision, exitCode = 0, trust = [], files = [first], control = false,
  reporter = false}) {
  const caseOut = path.join(out, name), report = path.join(caseOut, 'report.json')
  const trustFile = path.join(authority, name + '.json')
  fs.writeFileSync(trustFile, JSON.stringify(trust, null, 2) + '\n')
  const originalArgs = ['run', '--reporter=json', ...(reporter ? ['--reporter=verbose'] : []), '--outputFile=' + report]
  const args = [path.join(here, 'dispatcher.mjs'), project, caseOut, cache,
    ...(control ? ['--control'] : []), '--', ...originalArgs]
  const env = {...process.env, QUOD_SHIM_ACTIVE: '1', QUOD_DISPATCHER_FIXTURE_MODE: mode,
    QUOD_MANAGED_FILES: JSON.stringify(files), QUOD_MANAGED_EXPECTED_COUNT: '1',
    QUOD_MANAGED_ENV: JSON.stringify({NODE_ENV: 'test', QUOD_DISPATCHER_FIXTURE_MODE: mode}),
    QUOD_MANAGED_TRUST_FILE: trustFile}
  const log = fs.openSync(path.join(out, name + '.log'), 'w'), started = performance.now()
  let command
  try { command = spawnSync(process.execPath, args, {cwd: project, env, stdio: ['ignore', log, log], timeout: 60000}) }
  finally { fs.closeSync(log) }
  assert(!command.error, name + ': ' + command.error?.message)
  const actual = read(path.join(caseOut, 'result.json')), rows = ordinaryRows(read(report))
  const receipts = emittedReceipts(caseOut)
  const row = {name, mode, expected: {decision, exitCode, tests: files.length},
    processExit: command.status, seconds: (performance.now() - started) / 1000,
    originalArgs, selectedFiles: files, reportSha256: sha(fs.readFileSync(report)),
    resultSha256: sha(fs.readFileSync(path.join(caseOut, 'result.json'))), rows,
    receiptFiles: receipts.map(file => path.relative(out, file)),
    trusted: trust.map(value => ({...value, file: path.relative(out, value.file)}))}
  result.cases.push(row); write('result.json', result)
  check(name + ' preserves process exit', command.status === exitCode && actual.exitCode === exitCode)
  check(name + ' uses the expected execution path', actual.files.length > 0 && actual.files.every(file => file.decision === decision))
  const expectedRows = [[firstName, exitCode ? 'failed' : 'passed'],
    ...(files.length === 2 ? [[secondName, 'passed']] : [])].sort()
  check(name + ' preserves every selected test name and state', JSON.stringify(rows) === JSON.stringify(expectedRows))
  if (decision === 'replay') {
    const preflightHashes = jsonFiles(caseOut).map(file => { try { return read(file) } catch { return {} } })
      .filter(data => data.identity?.entry?.endsWith('/preflight.mjs')).map(data => data.sha256)
    const workers = [...fs.globSync('**/worker.mjs', {cwd: caseOut})]
    check(name + ' skips managed execution', workers.length > 0 && workers.every(file =>
      preflightHashes.includes(sha(fs.readFileSync(path.join(caseOut, file))))))
  }
  if (exitCode !== 0 || decision === 'ordinary') check(name + ' emits no reusable receipt', receipts.length === 0)
  return {actual, row, receipts}
}

try {
  const seed = invoke({name: 'seed', decision: 'managed', control: true})
  check('supported passing seed emits exactly one receipt', seed.receipts.length === 1)
  const seedFile = seed.receipts[0], bytes = fs.readFileSync(seedFile), receipt = read(seedFile)
  const receiptSha256 = sha(bytes), key = receipt.execution.key
  const cached = path.join(cache, key + '-' + receiptSha256 + '.json')
  fs.copyFileSync(seedFile, cached)
  const trusted = [{key, receiptSha256, file: cached}]
  result.seed = {key, receiptSha256, file: path.relative(out, seedFile), cachedFile: path.relative(out, cached)}
  write('result.json', result)
  // A local cache manifest supplies no independent authority.
  fs.writeFileSync(path.join(cache, 'manifest.json'), JSON.stringify(trusted) + '\n')
  invoke({name: 'hit', decision: 'replay', trust: trusted})
  invoke({name: 'missing-trust', decision: 'managed'})
  invoke({name: 'wrong-trust', decision: 'managed', trust: [{...trusted[0], receiptSha256: '0'.repeat(64)}]})
  const altered = structuredClone(receipt)
  const worker = JSON.parse(Buffer.from(altered.response.bytes, 'base64').toString('utf8'))
  worker.rows[0].name = 'FORGED_DISPATCHER_TEST_SUCCESS'
  const workerBytes = Buffer.from(JSON.stringify(worker))
  altered.response.bytes = workerBytes.toString('base64'); altered.response.sha256 = sha(workerBytes)
  altered.resultDigest = sha('quod-managed-receipt-v1-research\n' + key + '\n' + altered.response.bytes)
  const forgedFile = path.join(cache, 'coherently-forged-receipt.json')
  fs.writeFileSync(forgedFile, canonicalReceiptJSON(altered))
  invoke({name: 'forged-receipt', decision: 'managed', trust: [{...trusted[0], file: forgedFile}]})
  invoke({name: 'unsupported-pass', mode: 'unsupported-pass', decision: 'ordinary'})
  invoke({name: 'guard-assertion', mode: 'guard-assertion', decision: 'ordinary'})
  invoke({name: 'failure', mode: 'failure', decision: 'ordinary', exitCode: 1})
  invoke({name: 'unsupported-failure', mode: 'unsupported-failure', decision: 'ordinary', exitCode: 1})
  invoke({name: 'multiple-files', decision: 'ordinary', files: [first, second]})
  const alternative = path.join(project, 'vitest.config.mjs')
  assert(!fs.existsSync(alternative), 'alternative fixture config already exists')
  try {
    fs.writeFileSync(alternative, 'export default {test:{include:["packages/convex-helpers/server/quod-managed-dispatcher*.test.ts"],environment:"node"}}\n')
    invoke({name: 'custom-config', decision: 'ordinary'})
  } finally { fs.rmSync(alternative, {force: true}) }
  invoke({name: 'custom-reporter', decision: 'ordinary', reporter: true})
  result.passed = true
} catch (error) {
  result.error = {name: error.name, message: error.message, stack: error.stack}
  process.exitCode = 1
} finally {
  for (const file of fixturePaths) fs.rmSync(file, {force: true})
  write('result.json', result)
  console.log(JSON.stringify({...result, cases: result.cases.map(row => ({name: row.name, seconds: row.seconds, processExit: row.processExit}))}))
}
