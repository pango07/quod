// GitHub runner only. Keep the pinned project's real configuration and execute
// one supported managed test alongside one native ordinary test.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {fileURLToPath} from 'node:url'
import {spawnSync} from 'node:child_process'

const here = path.dirname(fileURLToPath(import.meta.url))
const first = 'packages/convex-helpers/server/quod-managed-remainder.first.test.ts'
const second = 'packages/convex-helpers/server/quod-managed-remainder.second.test.ts'
const firstName = 'managed remainder candidate passes once'
const secondName = 'ordinary remainder preserves its outcome'
const candidateMarker = 'QUOD_MANAGED_REMAINDER_CANDIDATE'
const ordinaryMarker = 'QUOD_MANAGED_REMAINDER_ORDINARY'
const firstSource = `import {test, expect} from 'vitest'\ntest('${firstName}', () => {\n  console.log('QUOD_' + 'MANAGED_REMAINDER_CANDIDATE')\n  expect(2 + 3).toBe(5)\n})\n`
const secondSource = `import {test, expect} from 'vitest'\ntest('${secondName}', () => {\n  console.log('QUOD_' + 'MANAGED_REMAINDER_ORDINARY')\n  expect(process.env.QUOD_REMAINDER_MODE).toBe('pass')\n})\n`
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n')
const keys = specs => specs.map(spec => JSON.stringify([path.resolve(spec.absolute), spec.project || ''])).sort()
const rows = report => report.testResults.flatMap(file => file.assertionResults.map(test =>
  [file.name, test.fullName, test.status])).sort()
const occurrences = (text, marker) => text.split(marker).length - 1

if (process.argv[2] === '--case') {
  const [, project, out, cacheDir, mode] = process.argv.slice(2)
  assert(['pass', 'fail'].includes(mode))
  process.env.QUOD_REMAINDER_MODE = mode
  process.env.QUOD_SHIM_ACTIVE = '1'
  const {dispatchManaged} = await import('./dispatcher.mjs')
  let recordCalls = 0
  const result = await dispatchManaged({project, out, cacheDir,
    args: ['run', '--reporter=default', '--reporter=json', '--outputFile=' + path.join(out, 'report.json')],
    files: [first], suiteFiles: [first, second], expectedCount: 1,
    env: {NODE_ENV: 'test', QUOD_REMAINDER_MODE: mode},
    record: async () => { recordCalls++ }})
  write(path.join(out, 'record-callback.json'), {recordCalls})
  process.exitCode = result.exitCode
} else {
  const [projectArg, outArg] = process.argv.slice(2)
  assert(projectArg && outArg, 'usage: dispatcher-remainder-integration.mjs PROJECT EMPTY_OUTPUT_DIRECTORY')
  const project = fs.realpathSync(path.resolve(projectArg)), out = path.resolve(outArg)
  assert(!fs.existsSync(out), 'integration OUT must not already exist')
  fs.mkdirSync(out, {recursive: true})
  const fixturePaths = [first, second].map(file => path.join(project, file))
  for (const file of fixturePaths) assert(!fs.existsSync(file), 'fixture target already exists')
  const result = {schema: 'quod-managed-remainder-integration-v1', passed: false,
    node: process.version, image: `${process.env.ImageOS}/${process.env.ImageVersion}`, project,
    fixtureFiles: [first, second], fixtureSourceSha256: [sha(firstSource), sha(secondSource)], cases: [], checks: []}
  const save = () => write(path.join(out, 'result.json'), result)
  const check = (name, pass) => { result.checks.push({name, pass: Boolean(pass)}); save(); assert(pass, name) }
  fs.mkdirSync(path.join(out, 'fixtures'))
  save()
  try {
    for (const [index, source] of [firstSource, secondSource].entries()) {
      fs.writeFileSync(fixturePaths[index], source, {flag: 'wx'})
      fs.writeFileSync(path.join(out, 'fixtures', path.basename(fixturePaths[index])), source)
    }
    for (const mode of ['pass', 'fail']) {
      const name = 'ordinary-' + mode, caseOut = path.join(out, name)
      const logPath = path.join(out, name + '.log'), fd = fs.openSync(logPath, 'w')
      const started = performance.now()
      let child
      try {
        child = spawnSync(process.execPath, [fileURLToPath(import.meta.url), '--case', project,
          caseOut, path.join(out, name + '-cache'), mode], {
          cwd: project, env: {...process.env, QUOD_SHIM_ACTIVE: '1'},
          stdio: ['ignore', fd, fd], timeout: 120000})
      } finally { fs.closeSync(fd) }
      assert(!child.error, child.error?.message)
      const actual = read(path.join(caseOut, 'result.json'))
      const report = read(path.join(caseOut, 'report.json'))
      const ordinary = read(path.join(caseOut, 'ordinary-report.json'))
      const selection = read(path.join(caseOut, 'ordinary-remainder-selection.json'))
      const preparation = read(path.join(caseOut, 'preparation.json'))
      const response = read(path.join(caseOut, 'selected-response.json'))
      const log = fs.readFileSync(logPath, 'utf8')
      const ordinaryLog = fs.readFileSync(actual.ordinaryRuns[0].logPath, 'utf8')
      const exitCode = mode === 'pass' ? 0 : 1
      const expectedRows = [[fixturePaths[0], firstName, 'passed'],
        [fixturePaths[1], secondName, mode === 'pass' ? 'passed' : 'failed']].sort()
      result.cases.push({name, mode, processExit: child.status, seconds: (performance.now() - started) / 1000,
        expectedExit: exitCode, rows: rows(report), resultSha256: sha(fs.readFileSync(path.join(caseOut, 'result.json'))),
        reportSha256: sha(fs.readFileSync(path.join(caseOut, 'report.json'))),
        selectionSha256: sha(fs.readFileSync(path.join(caseOut, 'ordinary-remainder-selection.json')))})
      save()
      check(name + ' preserves the process and complete report outcome', child.status === exitCode &&
        actual.exitCode === exitCode && report.success === (exitCode === 0))
      check(name + ' runs one managed candidate without fallback', actual.managedExecutions === 1 &&
        actual.hit === false && actual.fallback === false && actual.files.length === 1 &&
        actual.files[0].file === first && actual.files[0].decision === 'managed')
      check(name + ' runs the ordinary remainder exactly once', actual.ordinaryRuns.length === 1 &&
        actual.ordinaryRuns[0].exitCode === exitCode && actual.ordinaryRepeatedCandidate === false)
      check(name + ' retains both actual selected project specifications', preparation.selected.length === 2 &&
        keys(preparation.selected).join('\n') === keys(selection.selected).join('\n') &&
        preparation.selected.every(spec => spec.project === 'convex-helpers'))
      check(name + ' excludes only the candidate before native execution', selection.matched === true &&
        selection.runStarted === true && selection.completed === true && selection.exitCode === exitCode &&
        selection.excluded.length === 1 && selection.excluded[0].absolute === fixturePaths[0] &&
        selection.remainder.length === 1 && selection.remainder[0].absolute === fixturePaths[1] &&
        selection.remainder[0].project === 'convex-helpers' &&
        keys(selection.executed).join('\n') === keys(selection.remainder).join('\n'))
      check(name + ' ordinary JSON contains only the second file', ordinary.testResults.length === 1 &&
        ordinary.testResults[0].name === fixturePaths[1] &&
        JSON.stringify(rows(ordinary)) === JSON.stringify(expectedRows.filter(row => row[0] === fixturePaths[1])))
      check(name + ' complete JSON preserves both names and states', report.testResults.length === 2 &&
        report.numTotalTests === 2 && actual.rows.length === 2 && JSON.stringify(rows(report)) === JSON.stringify(expectedRows) &&
        JSON.stringify(actual.rows.map(row => [row.file, row.name, {pass: 'passed', fail: 'failed'}[row.state]]).sort()) === JSON.stringify(expectedRows))
      check(name + ' ordinary report is preserved in the merged output',
        JSON.stringify(report.testResults.find(file => file.name === fixturePaths[1])) === JSON.stringify(ordinary.testResults[0]))
      check(name + ' candidate passes with one captured console marker', response.ok === true &&
        response.rows.length === 1 && response.rows[0].name === firstName && response.rows[0].state === 'pass' &&
        response.boundary.logs.filter(([, values]) => values.join(' ').includes(candidateMarker)).length === 1 &&
        occurrences(log, candidateMarker) === 1 && occurrences(ordinaryLog, candidateMarker) === 0 &&
        occurrences(ordinaryLog, ordinaryMarker) === 1)
      check(name + ' failed whole commands never publish passing candidate receipts',
        actual.receipts.length === (exitCode === 0 ? 1 : 0) &&
        fs.existsSync(path.join(caseOut, 'receipt.json')) === (exitCode === 0) &&
        read(path.join(caseOut, 'record-callback.json')).recordCalls === (exitCode === 0 ? 1 : 0))
    }
    result.passed = true
  } catch (error) {
    result.error = {name: error.name, message: error.message, stack: error.stack}
    process.exitCode = 1
  } finally {
    for (const file of fixturePaths) fs.rmSync(file, {force: true})
    save()
    console.log(JSON.stringify({...result, checks: result.checks.length}))
  }
}
