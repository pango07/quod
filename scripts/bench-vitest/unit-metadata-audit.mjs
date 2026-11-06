import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'

const [binaryArg, fixtureArg, outputArg, expectation] = process.argv.slice(2)
assert(['stale', 'correct'].includes(expectation), 'usage: binary fixture output-dir stale|correct')
assert.equal(process.platform, 'linux')
const binary = fs.realpathSync(binaryArg)
const fixture = fs.realpathSync(fixtureArg)
const output = path.resolve(outputArg)
fs.mkdirSync(output, { recursive: true })
// bubblewrap has a private /tmp: keep the checkout on the real workspace.
const base = fs.mkdtempSync(path.join(process.cwd(), '.unit-metadata-audit-'))
const run = (cwd, command, args) => {
  const r = spawnSync(command, args, {
    cwd, env: { ...process.env, QUOD_DISABLE: '1' }, encoding: 'utf8',
    timeout: 30_000, maxBuffer: 2 * 1024 * 1024,
  })
  assert(!r.error, r.error?.message)
  assert.equal(r.signal, null)
  return { status: r.status, stdout: r.stdout, stderr: r.stderr }
}
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
const cases = [
  ['file-mtime', 'stat', 'file'],
  ['directory-mtime', 'stat', 'directory'],
  ['file-mode', 'mode', 'file'],
  ['symlink-mtime', 'lstat', 'link'],
  ['fd-mtime', 'fstat', 'file'],
  ['empty-path-mtime', 'empty-path', 'file'],
  ['statx-mtime', 'statx', 'file'],
  ['package-mtime', 'stat', 'package'],
  ['symlink-target', 'readlink', 'link'],
]
const results = []
for (const [name, method, kind] of cases) {
  const root = path.join(base, name, 'repo')
  const ledger = path.join(base, name, 'ledger')
  fs.mkdirSync(root, { recursive: true })
  const relative = kind === 'package' ? 'node_modules/example/subject' : 'subject'
  const subject = path.join(root, relative)
  fs.mkdirSync(path.dirname(subject), { recursive: true })
  if (kind === 'directory') fs.mkdirSync(subject)
  else if (kind === 'link') {
    fs.writeFileSync(path.join(root, 'target-a'), 'same bytes\n')
    fs.writeFileSync(path.join(root, 'target-b'), 'same bytes\n')
    fs.symlinkSync('target-a', subject)
  } else fs.writeFileSync(subject, 'same bytes\n')
  if (kind === 'package') fs.writeFileSync(path.join(path.dirname(subject), 'package.json'), '{"name":"example","version":"1.0.0"}')
  const touch = t => kind === 'link' ? fs.lutimesSync(subject, t, t) : fs.utimesSync(subject, t, t)
  touch(1_700_000_000)
  if (method === 'mode') fs.chmodSync(subject, 0o644)
  const beforeHash = kind === 'directory' ? null : hash(subject)
  const args = [method, relative]
  const plainBefore = run(root, fixture, args)
  assert.equal(plainBefore.status, 0, JSON.stringify(plainBefore))
  const quod = () => run(root, binary, ['unit', '--dir', ledger, '--root', root, '--', fixture, ...args])
  const warm = [quod(), quod(), quod()]
  for (const result of warm) {
    assert.equal(result.status, plainBefore.status, JSON.stringify(result))
    assert.equal(result.stdout, plainBefore.stdout, JSON.stringify(result))
  }
  if (method === 'mode') fs.chmodSync(subject, 0o600)
  else if (method === 'readlink') {
    fs.unlinkSync(subject)
    fs.symlinkSync('target-b', subject)
  } else touch(1_700_000_600)
  const afterHash = kind === 'directory' ? null : hash(subject)
  assert.equal(afterHash, beforeHash, 'the fixture must preserve the file contents')
  const plainAfter = run(root, fixture, args)
  assert.equal(plainAfter.status, 0)
  assert.notEqual(plainAfter.stdout, plainBefore.stdout, 'the metadata must be observable')
  const changed = quod()
  const stale = changed.status !== plainAfter.status || changed.stdout !== plainAfter.stdout
  const result = {
    name, method, contentsUnchanged: true, beforeHash, afterHash, plainBefore, plainAfter,
    warm, warmedReplay: warm[2].stderr.includes('answered from its record'), changed,
    replayedAfterChange: changed.stderr.includes('answered from its record'), stale,
  }
  results.push(result)
  fs.writeFileSync(path.join(output, `${name}.json`), JSON.stringify(result, null, 2) + '\n')
  console.log(JSON.stringify({ name, warmedReplay: result.warmedReplay, replayedAfterChange: result.replayedAfterChange, stale }))
}
const summary = { binarySha256: hash(binary), fixtureSha256: hash(fixture), expectation, cases: results.length, stale: results.filter(r => r.stale).length, results }
fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify(summary, null, 2) + '\n')
if (expectation === 'correct') assert.equal(summary.stale, 0, 'metadata-only edits must not return stale output')
else {
  const file = results.find(r => r.name === 'file-mtime')
  assert(file.warmedReplay && file.replayedAfterChange && file.stale, 'the shipped-binary counterexample was not reproduced')
}
