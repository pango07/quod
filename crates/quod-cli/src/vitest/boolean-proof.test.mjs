import { test } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { certifyBooleanReads } from './boolean-proof.mjs'

test('accepts only reads consumed by language truthiness', () => {
  for (const expression of ['if (pkg.exports) x()', 'const x = !pkg.exports',
    'const x = pkg?.data.exports && pkg.data.name === id ? pkg : null',
    'while (pkg.exports || other) x()', 'for (; pkg.exports;) x()']) {
    assert.equal(certifyBooleanReads(expression).length, 1, expression)
  }
  for (const expression of ['const x = pkg.exports', 'const x = pkg.exports || other',
    'const x = pkg.exports && other', 'const x = pkg.exports === other',
    'if (Object.keys(pkg.exports).length) x()', 'if (Boolean(pkg.exports)) x()',
    'if (pkg.exports["."]) x()', 'if (pkg["exports"]) x()',
    'if (getPkg().exports) x()', 'if ((effect(),pkg).exports) x()',
    'if (pkg.exports.exports) x()', 'if (pkg.exports ?? other) x()']) {
    assert.deepEqual(certifyBooleanReads(expression), [], expression)
  }
})

function run(expression, {rewrite=false, prelude=''} = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'quod-boolean-proof-'))
  try {
    const config = path.join(dir, 'config.mjs'), runner = path.join(dir, 'runner.mjs')
    fs.writeFileSync(config, `import fs from 'node:fs';\n${prelude}\n${rewrite ? `fs.writeFileSync(import.meta.filename, 'export const value = globalThis.pkg.exports ? 1 : 0');` : ''}\nexport const value = ${expression};\n`)
    fs.writeFileSync(runner, `import {createBooleanProof} from ${JSON.stringify(new URL('./boolean-proof.mjs', import.meta.url).href)};
const proof = createBooleanProof({internalFiles:[import.meta.url]});
const reads=[]; globalThis.pkg={get exports(){reads.push(proof.test('exports'));return {'.':'x'};}};
const m=await import('./config.mjs'); proof.close(); console.log(JSON.stringify({reads, value:m.value}));`)
    const result = spawnSync(process.execPath, [runner], {encoding:'utf8', timeout:10000})
    assert.equal(result.status, 0, result.stderr || String(result.error))
    return JSON.parse(result.stdout)
  } finally { fs.rmSync(dir, {recursive:true, force:true}) }
}
test('actual V8 source identifies the getter read in a conditional', () => {
  assert.deepEqual(run('globalThis.pkg.exports ? 1 : 0'), {reads:[true], value:1})
  assert.deepEqual(run('globalThis.pkg.exports && true ? 1 : 0'), {reads:[true], value:1})
})
test('logical expressions whose values escape retain the complete field', () => {
  assert.deepEqual(run('globalThis.pkg.exports || true').reads, [false])
  assert.deepEqual(run('Object.keys(globalThis.pkg.exports)').reads, [false])
})
test('rewritten disk source and spoofed sourceURL never grant boolean proof', () => {
  assert.deepEqual(run('Object.keys(globalThis.pkg.exports)', {rewrite:true}).reads, [false])
  assert.deepEqual(run('eval("globalThis.pkg.exports\\n//# sourceURL=trusted.mjs")').reads, [false])
})
test('a second non-boolean read is separately retained', () => {
  assert.deepEqual(run('(globalThis.pkg.exports ? Object.keys(globalThis.pkg.exports) : [])').reads, [true,false])
})
test('modified parser intrinsics fail closed', () => {
  assert.deepEqual(run('globalThis.pkg.exports ? 1 : 0', {prelude:'const prior=Object.values; Object.values=function(x){return prior(x)};'}).reads, [false])
})
