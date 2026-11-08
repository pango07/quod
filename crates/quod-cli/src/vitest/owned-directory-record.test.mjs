import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const recorder = new URL('./record.mjs', import.meta.url).href

// Every probe has fresh native fs exports and a private fixture tree. The
// subprocess bound also catches an accidentally retained watcher or callback.
function probe(body) {
  const source = `
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import assert from 'node:assert/strict'
import {createRecorder, patchFs} from ${JSON.stringify(recorder)}
const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'quod-dir-read-')))
const workspace = path.join(root, 'workspace')
fs.mkdirSync(workspace)
const r = createRecorder({stateDir:'/not-quod-state', mode:'worker', wsRoot:workspace+'/'})
try {
${body}
} finally {
  r.on = false
  fs.rmSync(root, {recursive:true, force:true})
}
`
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', source], {
    encoding: 'utf8', timeout: 10000, maxBuffer: 1024 * 1024,
  })
  assert.equal(result.error, undefined, result.error?.message)
  assert.equal(result.signal, null)
  assert.equal(result.status, 0, result.stderr)
  return JSON.parse(result.stdout)
}

const has = (row, kind, file) => assert(row.keys.includes(`${kind}:${file}`), `${kind} observation must remain keyed`)

test('existing recursive mkdir keeps reads despite mutable options and matches native getter calls', () => {
  const row = probe(`
const existing = path.join(root, 'existing'), input = path.join(existing, 'input')
fs.mkdirSync(existing)
fs.writeFileSync(input, 'pre-existing input')
const nativeMkdir = fs.mkdir
let nativeReads = 0
const nativeOptions = {get recursive() {nativeReads++; return true}}
await new Promise((resolve, reject) => nativeMkdir(existing, nativeOptions, (error, first) => {
  if (error) return reject(error)
  assert.equal(first, undefined)
  resolve()
}))
patchFs(r)
let patchedReads = 0
const patchedOptions = {get recursive() {patchedReads++; return true}}
await new Promise((resolve, reject) => fs.mkdir(existing, patchedOptions, (error, first) => {
  if (error) return reject(error)
  assert.equal(first, undefined)
  resolve()
}))
const options = {recursive:true}
const completion = new Promise((resolve, reject) => fs.mkdir(existing, options, (error, first) => {
  if (error) return reject(error)
  assert.equal(first, undefined, 'recursive mkdir of an existing directory creates nothing')
  assert.equal(fs.readFileSync(input, 'utf8'), 'pre-existing input')
  fs.readdirSync(existing)
  fs.statSync(input)
  resolve()
}))
options.recursive = false
await completion
console.log(JSON.stringify({keys:[...r.keys], input, existing, nativeReads, patchedReads}))
`)
  assert(row.nativeReads > 0)
  assert.equal(row.patchedReads, row.nativeReads, 'the recorder must not evaluate native options again after completion')
  has(row, 'f', row.input)
  has(row, 'sm', row.input)
  has(row, 'd', row.existing)
})

test('a relative symlink inside a fresh directory retains content, metadata and resolution dependencies', () => {
  const row = probe(`
const external = path.join(root, 'source.txt')
fs.writeFileSync(external, 'before')
patchFs(r)
const directory = fs.mkdtempSync(path.join(root, 'owned-')), link = path.join(directory, 'link')
fs.symlinkSync('../source.txt', link)
const before = fs.readFileSync(link, 'utf8')
fs.statSync(link)
fs.lstatSync(link)
assert.equal(fs.existsSync(link), true)
fs.accessSync(link, fs.constants.F_OK)
assert.equal(fs.realpathSync(link), external)
fs.writeFileSync(external, 'after')
const after = fs.readFileSync(link, 'utf8')
console.log(JSON.stringify({keys:[...r.keys], link, before, after}))
`)
  assert.equal(row.before, 'before')
  assert.equal(row.after, 'after')
  for (const kind of ['f', 'sm', 'sl', 'e', 'a0', 'rp']) has(row, kind, row.link)
})

test('fresh directory metadata remains native and fully keyed after a permission change', () => {
  const row = probe(`
patchFs(r)
const directory = fs.mkdtempSync(path.join(root, 'metadata-')), input = path.join(directory, 'input')
fs.writeFileSync(input, 'unchanged bytes')
const before = fs.statSync(directory)
const mode = (before.mode & 0o777) ^ 0o010
fs.chmodSync(directory, mode)
const after = await fs.promises.stat(directory)
const callbackMode = await new Promise((resolve, reject) => fs.stat(directory, (error, stats) => {
  if (error) return reject(error)
  resolve(stats.mode & 0o777)
}))
fs.lstatSync(directory)
assert.equal(fs.readFileSync(input, 'utf8'), 'unchanged bytes')
const inputMetadata = fs.statSync(input)
fs.lstatSync(input)
console.log(JSON.stringify({keys:[...r.keys], directory, input, beforeMode:before.mode & 0o777,
  afterMode:after.mode & 0o777, callbackMode, mode, size:inputMetadata.size}))
`)
  assert.notEqual(row.beforeMode, row.mode)
  assert.equal(row.afterMode, row.mode)
  assert.equal(row.callbackMode, row.mode)
  assert.equal(row.size, Buffer.byteLength('unchanged bytes'))
  for (const file of [row.directory, row.input]) {
    has(row, 'sm', file)
    has(row, 'sl', file)
    assert(!row.keys.includes(`st:${file}`), 'creation must not downgrade complete metadata to type only')
  }
  has(row, 'f', row.input)
})

test('native watches under fresh directories remain reported and respect recording state', () => {
  const row = probe(`
patchFs(r)
const direct = fs.mkdtempSync(path.join(root, 'watch-'))
const polled = fs.mkdtempSync(path.join(root, 'watch-file-'))
fs.watch(direct).close()
const listener = () => {}
fs.watchFile(polled, listener)
fs.unwatchFile(polled, listener)
r.on = false
const disabled = fs.mkdtempSync(path.join(root, 'disabled-'))
fs.watch(disabled).close()
console.log(JSON.stringify({watches:[...r.watches].sort(), expected:[direct, polled].sort()}))
`)
  assert.deepEqual(row.watches, row.expected)
})
