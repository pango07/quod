import { test } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { certifyNamesOnly } from './names-proof.mjs'

const parse = "const pkg = JSON.parse(readFileSync('package.json', 'utf8'));"
const names = 'export const value = Object.keys({...pkg.dependencies, ...pkg.devDependencies}).filter(n => n.startsWith("zod"));'

test('bounded syntax accepts a private parse followed immediately by names-only copy', () => {
  assert.equal(certifyNamesOnly(parse + names).length, 1)
})

for (const [label, source] of [
  ['version branch', parse + names + "if (pkg.devDependencies.unloaded === '1') run()"],
  ['escape', parse + names + 'consume(pkg)'],
  ['export', parse + names + 'export {pkg}'],
  ['alias', parse + names + 'const alias = pkg'],
  ['reviver', parse.replace("'utf8'))", "'utf8'), revive)") + names],
  ['intervening effect', parse + 'mutate();' + names],
  ['await', parse + names.replace('Object.keys', 'await Object.keys')],
  ['sequence', parse + names.replace('Object.keys', '(mutate(), Object.keys') + ')'],
  ['wrapper', parse + names.replace('Object.keys', 'consume(Object.keys') + ')'],
  ['Object parameter', parse + names + 'function f(Object) {}'],
  ['Object class expression', parse + names + 'const x = class Object {};'],
  ['Object import', "import Object from 'object';" + parse + names],
  ['eval', parse + names + "eval('pkg')"],
  ['computed member', parse + names.replace('pkg.dependencies', 'pkg["dependencies"]')],
  ['mixed values', parse + names.replace('{...pkg.', '{x: value(), ...pkg.')],
  ['multiple declarators', parse.replace(';', ', other = mutate();') + names],
  ['later second copy', parse + names + 'const x = Object.keys({...pkg.dependencies});'],
]) test(`retains values: ${label}`, () => assert.deepEqual(certifyNamesOnly(source), []))

function run(source, { prelude = '', enabled = true, rewrite = false } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'quod-names-proof-'))
  try {
    fs.writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ dependencies: { 'zod-4.1': '1' }, devDependencies: { unloaded: '1' } }))
    const cfg = path.join(dir, 'cfg.mjs')
    fs.writeFileSync(cfg, "import {readFileSync, writeFileSync} from 'node:fs';\n" + prelude + '\n' +
      (rewrite ? `writeFileSync(import.meta.filename, ${JSON.stringify(parse + names)});\n` : '') + source)
    fs.writeFileSync(path.join(dir, 'run.mjs'), `
import {createRecorder, patchFs, settledKeys} from ${JSON.stringify(new URL('./record.mjs', import.meta.url).href)};
import {createNamesProof} from ${JSON.stringify(new URL('./names-proof.mjs', import.meta.url).href)};
const namesProof = ${enabled} ? createNamesProof() : null;
const r = createRecorder({stateDir:'/nonexistent-quod-state', mode:'main', namesProof});
patchFs(r);
const m = await import('./cfg.mjs');
const keys = [...settledKeys(r)].filter(k => k.startsWith('pjf:')).map(k => JSON.parse(k.split('\\0')[1]));
namesProof?.close();
console.log(JSON.stringify({value:m.value, keys, stats:namesProof?.stats}));
`)
    const out = spawnSync(process.execPath, [path.join(dir, 'run.mjs')], { cwd:dir, encoding:'utf8', timeout:10000 })
    assert.equal(out.status, 0, out.stderr || String(out.error))
    return JSON.parse(out.stdout)
  } finally { fs.rmSync(dir, { recursive:true, force:true }) }
}

test('actual executed ESM copy drops only discarded primitive values', () => {
  const result = run(parse + '\n' + names)
  assert.deepEqual(result.value, ['zod-4.1'])
  assert.deepEqual(result.keys, [[['dependencies'], ['devDependencies']]])
  assert.equal(result.stats.bound, 1)
  assert.equal(result.stats.ignored, 2)
})

test('disabled proof retains every copied value', () => {
  const result = run(parse + '\n' + names, {enabled:false})
  assert.deepEqual(result.keys, [[['dependencies','zod-4.1'], ['dependencies'], ['devDependencies','unloaded'], ['devDependencies']]])
})

test('value-dependent branch remains keyed, including with a misleading disk source', () => {
  for (const rewrite of [false, true]) {
    const result = run(parse + '\n' + names + "\nif (pkg.devDependencies.unloaded !== '1') throw Error('changed branch');", {rewrite})
    assert.ok(result.keys[0].some(p => p.join('.') === 'devDependencies.unloaded'))
    assert.equal(result.stats.ignored, 0)
  }
})

test('replacement Object.keys cannot hide copied values', () => {
  const prelude = `const original = Object.keys;
Object.keys = obj => { const values = Object.values(obj); return original(obj); };`
  const result = run(parse + '\n' + names, {prelude})
  assert.ok(result.keys[0].some(p => p.join('.') === 'devDependencies.unloaded'))
  assert.equal(result.stats.ignored, 0)
})

test('a transient callee getter that restores native keys cannot grant proof', () => {
  const prelude = `const nativeKeys = Object.keys;
Object.defineProperty(Object, 'keys', {configurable:true, get() {
  Object.defineProperty(Object, 'keys', {value:nativeKeys, writable:true, configurable:true});
  return obj => { Object.values(obj); return nativeKeys(obj); };
}});`
  const result = run(parse + '\n' + names, {prelude})
  assert.ok(result.keys[0].some(p => p.join('.') === 'devDependencies.unloaded'))
  assert.equal(result.stats.ignored, 0)
})

test('a wrapper before names evaluation prevents promotion', () => {
  const result = run(parse + '\n' + names.replace('Object.keys', '(globalThis.prepare(), Object.keys').replace('.filter', ').filter'), {prelude:'globalThis.prepare = () => {};'} )
  assert.equal(result.stats.ignored, 0)
})
