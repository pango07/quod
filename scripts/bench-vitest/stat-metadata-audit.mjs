// An adversarial replay regression against the packaged binary. A failing
// comparison is a correctness defect, not an expected-success benchmark.
import fs from 'node:fs'
import path from 'node:path'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'

const [projectArg, binary, proofMode = 'off'] = process.argv.slice(2)
assert.ok(['off', 'names', 'all'].includes(proofMode))
const project = path.resolve(projectArg)
const out = `/tmp/quod-stat-audit/evidence${proofMode === 'off' ? '' : '-'+proofMode}`
fs.mkdirSync(out, { recursive: true })
const input = path.join(project, 'metadata-input.txt')
fs.writeFileSync(input, 'These bytes never change.\n')
fs.chmodSync(input, 0o644)
fs.utimesSync(input, 1577836800, 1577836800)
fs.writeFileSync(path.join(project, 'vitest.config.mjs'), `export default {
  css: { postcss: {} }, test: { include: ['metadata.test.mjs'], maxWorkers: 1 }
};\n`)
fs.writeFileSync(path.join(project, 'metadata.test.mjs'), `
import fs from 'node:fs';
import { test, expect } from 'vitest';
const file = 'metadata-input.txt';
test('synchronous mtime', () => expect(fs.statSync(file).mtimeMs).toBe(1577836800000));
test('promise mtime', async () => expect((await fs.promises.stat(file)).mtimeMs).toBe(1577836800000));
test('callback mtime', async () => {
  const stat = await new Promise((resolve, reject) => fs.stat(file, (error, value) => error ? reject(error) : resolve(value)));
  expect(stat.mtimeMs).toBe(1577836800000);
});
test('lstat mtime', () => expect(fs.lstatSync(file).mtimeMs).toBe(1577836800000));
test('mode', () => expect(fs.statSync(file).mode & 0o777).toBe(0o644));
`)
const env = { ...process.env, QUOD_VITEST_LINEAGE: '0', QUOD_VITEST_TIMING: 'off',
  QUOD_VITEST_NAMES_PROOF: proofMode === 'off' ? '0' : '1',
  QUOD_VITEST_BOOLEAN_PROOF: proofMode === 'all' ? '1' : '0' }
const runs = []
function run(label, plain = false) {
  const report = path.join(out, `${label}-report.json`)
  const summary = path.join(out, `${label}-summary.json`)
  const args = plain
    ? ['node', 'node_modules/vitest/vitest.mjs', 'run', '--reporter=json', `--outputFile=${report}`]
    : [binary, 'vitest', '--dir', `/tmp/quod-stat-audit/state-${proofMode}`, '--audits', '0',
      '--report', report, '--summary', summary, '--', 'run']
  const result = spawnSync(args[0], args.slice(1), { cwd: project, env, encoding: 'utf8', timeout: 90000 })
  fs.writeFileSync(path.join(out, `${label}.log`), (result.stdout ?? '') + (result.stderr ?? ''))
  assert.equal(result.error, undefined)
  assert.ok(result.status === 0 || result.status === 1, `Unexpected exit: ${result.status}`)
  const data = JSON.parse(fs.readFileSync(report, 'utf8'))
  const states = data.testResults.flatMap(f => f.assertionResults.map(a => [a.fullName ?? a.title, a.status])).sort()
  assert.equal(states.length, 5, 'Missing test evidence')
  assert.ok(states.every(([name]) => typeof name === 'string' && name.length), 'Missing test names')
  const row = { label, exit: result.status, states,
    summary: plain ? null : JSON.parse(fs.readFileSync(summary, 'utf8')) }
  runs.push(row)
  fs.writeFileSync(path.join(out, 'runs.json'), JSON.stringify(runs, null, 2))
  console.log(JSON.stringify({ label, exit: row.exit, states, replayed: row.summary?.replayed }))
  return row
}
const before = run('before-plain', true)
assert.equal(before.exit, 0)
for (const label of ['record', 'confirm', 'warm']) {
  const row = run(label)
  assert.equal(row.exit, 0)
  assert.deepEqual(row.states, before.states)
}
assert.equal(runs.at(-1).summary.replayed, 1, 'Exercise an actually replayable record')
const bytes = fs.readFileSync(input)
fs.chmodSync(input, 0o755)
fs.utimesSync(input, 1609459200, 1609459200)
assert.deepEqual(fs.readFileSync(input), bytes, 'Mutation must preserve the content key')
const reused = run('metadata-changed-quod')
const actual = run('metadata-changed-plain', true)
assert.equal(actual.exit, 1, 'Mutation must change the actual test outcome')
assert.ok(actual.states.every(([, state]) => state === 'failed'))
const wrong = reused.states.filter((state, i) => JSON.stringify(state) !== JSON.stringify(actual.states[i])).length
fs.writeFileSync(path.join(out, 'result.json'), JSON.stringify({ wrong, quodExit: reused.exit,
  plainExit: actual.exit, replayed: reused.summary.replayed, contentChanged: false }, null, 2))
assert.deepEqual(reused.states, actual.states, 'Metadata-only change must invalidate replay')
assert.equal(reused.exit, actual.exit)
