import { test } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { certifyBooleanReads, certifyStatTypeReads } from './boolean-proof.mjs'

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

function recorded(expression, {extra='', prelude='', enabled=true, resolver=true, names=false, statEnabled=enabled} = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'quod-boolean-recorder-'))
  try {
    fs.writeFileSync(path.join(dir,'package.json'), JSON.stringify({name:'x', exports:{'.':'./a.js'}, devDependencies:{unused:'1'}}))
    const config = path.join(dir, resolver ? 'node_modules/vite/reader.mjs' : 'config.mjs')
    fs.mkdirSync(path.dirname(config), {recursive:true})
    fs.writeFileSync(config, `import fs, {readFileSync} from 'node:fs';
${prelude}
const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
${names ? 'const keys = Object.keys({...pkg.devDependencies});' : ''}
export const value = ${expression};
${extra}`)
    const recordUrl = new URL('./record.mjs', import.meta.url).href
    const runner = path.join(dir, 'runner.mjs')
    fs.writeFileSync(runner, `import {createRecorder, patchFs, settledKeys} from ${JSON.stringify(recordUrl)};
import {createBooleanProof} from ${JSON.stringify(new URL('./boolean-proof.mjs', import.meta.url).href)};
import {createNamesProof} from ${JSON.stringify(new URL('./names-proof.mjs', import.meta.url).href)};
const booleanProof = ${enabled} ? createBooleanProof({internalFiles:[${JSON.stringify(recordUrl)}]}) : null;
const statProof = ${statEnabled} ? booleanProof ?? createBooleanProof({internalFiles:[${JSON.stringify(recordUrl)}]}) : null;
const namesProof = ${names} ? createNamesProof() : null;
const r=createRecorder({stateDir:'/nonexistent-state',mode:'main',booleanProof,namesProof,statProof}); patchFs(r);
const m=await import(${JSON.stringify(config)});
const keys=[...settledKeys(r)].filter(k=>k.includes('/package.json')).map(k=>k.replace(process.cwd()+'/', ''));
statProof?.close(); if(booleanProof!==statProof)booleanProof?.close(); namesProof?.close(); console.log(JSON.stringify({keys,value:m.value,stats:statProof?.stats}));`)
    const result=spawnSync(process.execPath,[runner],{cwd:dir,encoding:'utf8',timeout:10000})
    assert.equal(result.status,0,result.stderr || String(result.error))
    return JSON.parse(result.stdout)
  } finally {fs.rmSync(dir,{recursive:true,force:true})}
}
const query = result => result.keys.filter(k=>k.startsWith('pjq:')).map(k=>JSON.parse(k.split('\0')[1]))
test('resolver boolean reads replace the broad key only after a watched parse', () => {
  const result=recorded('pkg.exports && pkg.name === "x" ? true : false')
  assert.equal(result.value,true)
  assert.deepEqual(query(result),[{values:[['name']],truthy:['exports']}])
  assert.equal(result.keys.some(k=>/^(f|pj):/.test(k)),false)
})
test('ordinary value readers retain full exports alongside boolean queries', () => {
  const result=recorded('pkg.exports ? Object.keys(pkg.exports) : []')
  assert.deepEqual(query(result),[{values:[['exports']],truthy:['exports']}])
})
test('an additional unparsed read keeps the conservative file input', () => {
  for (const resolver of [false,true]) {
    const result=recorded('!!pkg.exports',{resolver,extra:"export const raw = readFileSync('package.json','utf8');"})
    assert.deepEqual(query(result),[])
    assert.ok(result.keys.some(k=>k === `${resolver ? 'pj' : 'f'}:package.json`))
  }
})
test('a stat observation cannot silently be discarded by the JSON proof', () => {
  const result=recorded('!!pkg.exports',{extra:"export const size = fs.statSync('package.json').size;"})
  assert.deepEqual(query(result),[{values:[],truthy:['exports']}])
  assert.ok(result.keys.includes('sm:package.json'), 'JSON refinement must retain the separate metadata observation')
})
test('disabled boolean proof keeps the existing resolver and config behavior', () => {
  assert.ok(recorded('!!pkg.exports',{enabled:false}).keys.includes('pj:package.json'))
  const result=recorded('!!pkg.exports',{enabled:false,resolver:false})
  assert.ok(result.keys.some(k=>k.startsWith('pjf:') && k.includes('exports')))
})
test('the names proof remains conservative when the object has another use', () => {
  const result=recorded('!!pkg.exports',{resolver:false,names:true})
  const values=query(result)[0].values
  assert.ok(values.some(p=>p.join('.')==='devDependencies.unused'))
  assert.deepEqual(query(result)[0].truthy,['exports'])
})

const statHelper = 'function tryStatSync(file) { try { return fs.statSync(file, {throwIfNoEntry:false}); } catch {} }'
test('names-only mode can prove a stat type without refining manifest truthiness', () => {
  const result=recorded('!!pkg.exports',{enabled:false,statEnabled:true,names:true,
    prelude:`${statHelper}\nconst file='package.json'; export const type=tryStatSync(file)?.isFile();`})
  assert.ok(result.keys.includes('st:package.json'))
  assert.ok(result.keys.includes('pj:package.json'))
  assert.deepEqual(query(result),[])
  assert.equal(result.stats.boolean,0)
})
test('names-only mode keeps full metadata for an unknown stat consumer', () => {
  const result=recorded('!!pkg.exports',{enabled:false,statEnabled:true,names:true,
    prelude:`${statHelper}\nconst file='package.json'; export const time=tryStatSync(file).mtimeMs;`})
  assert.ok(result.keys.includes('sm:package.json'))
  assert.equal(result.keys.includes('st:package.json'),false)
  assert.deepEqual(query(result),[])
})
test('only an immediate native file-type consumer qualifies for stat narrowing', () => {
  for (const expression of ['tryStatSync(file)?.isFile()', 'tryStatSync(file).isDirectory()', '!tryStatSync(file)', 'tryStatSync(file) ? 1 : 0']) {
    const proof=certifyStatTypeReads(`${statHelper}\nconst value=${expression}`)
    assert.equal(proof.helpers.length,1)
    assert.equal(proof.calls.length,1,expression)
  }
  for (const expression of ['tryStatSync(file).size', 'tryStatSync(file).mtimeMs',
    'tryStatSync(file)', 'tryStatSync(file) || {}', 'tryStatSync(file)?.isSymbolicLink()', 'tryStatSync(file)?.[method]()',
    'tryStatSync(file)?.isFile(arg)', 'tryStatSync(makeFile())?.isFile()',
    'tryStatSync(file)?.isFile?.()']) {
    assert.equal(certifyStatTypeReads(`${statHelper}\nconst value=${expression}`).calls.length,0,expression)
  }
  for (const helper of [
    statHelper.replace('catch {}','catch (err) {}'),
    statHelper.replace('catch {}','catch {} finally { effect() }'),
    statHelper.replace('return fs.statSync(file, {throwIfNoEntry:false});','const s=fs.statSync(file, {throwIfNoEntry:false}); return s;'),
    statHelper.replace('false','true'), statHelper.replace('{throwIfNoEntry:false}','opts'),
    statHelper.replace('function tryStatSync','async function tryStatSync'),
    'export default function(file) { try { return fs.statSync(file, {throwIfNoEntry:false}); } catch {} }',
  ]) assert.equal(certifyStatTypeReads(helper+'\ntryStatSync(file)?.isFile()').helpers.length,0,helper)
})
test('actual resolver stat type reads coexist with the boolean manifest query', () => {
  for (const method of ['isFile','isDirectory']) {
    const result=recorded('!!pkg.exports', {prelude:`${statHelper}\nconst file='package.json'; export const type=tryStatSync(file)?.${method}();`})
    assert.deepEqual(query(result),[{values:[],truthy:['exports']}])
    assert.ok(result.keys.includes('st:package.json'),JSON.stringify(result))
    assert.equal(result.keys.includes('pj:package.json'),false)
    assert.equal(result.stats.statType,1)
  }
})
test('private helper results may feed only uninterrupted type branches', () => {
  for (const body of [
    'const s=tryStatSync(file); if(s?.isFile()) return "file"; if(s?.isDirectory()) return "directory"; return "other";',
    'const s=tryStatSync(file); if(!s || !s.isFile() && !s.isDirectory()) return "other"; return "exists";',
    'const s=tryStatSync(file); if(s?.isFile()) { return "file"; } else if(s?.isDirectory()) { return "directory"; } return "other";',
  ]) {
    const result=recorded('!!pkg.exports',{prelude:`${statHelper}\nfunction kind(file){${body}} export const type=kind('package.json');`})
    assert.ok(result.keys.includes('st:package.json'),body)
    assert.equal(result.keys.includes('sm:package.json'),false,body)
    assert.equal(result.stats.statType,1)
  }
})
test('private stat bindings reject escapes, intervening code and unknown consumers', () => {
  for (const body of [
    'const s=tryStatSync(file); effect(); if(s.isFile()) return "file";',
    'const s=tryStatSync(file); if(s.isFile()) effect(); if(s.isDirectory()) return "directory";',
    'const s=tryStatSync(file); if(s.isFile()) return s.mtimeMs;',
    'const s=tryStatSync(file); if(s.isFile()) return () => s.size;',
    'const s=tryStatSync(file); if(s.isFile()) return eval("s.size");',
    'const s=tryStatSync(file); if(effect() && s.isFile()) return "file";',
    'const s=tryStatSync(file); if(s?.isFIFO()) return "fifo";',
    'const s=tryStatSync(file); if(s.isFile()) return "file"; const alias=s;',
    'const s=tryStatSync(file); if(s.isFile()) return "file"; if(s.size) return "size";',
    'const s=tryStatSync(file); if(s.isFile()) { try { return "file"; } finally { effect() } } if(s.isDirectory()) return "directory";',
    'let s=tryStatSync(file); if(s.isFile()) return "file";',
  ]) {
    const proof=certifyStatTypeReads(`${statHelper}\nfunction kind(file){${body}}`)
    assert.equal(proof.calls.length,0,body)
  }
})
test('private type branches retain metadata after a Stats prototype change', () => {
  const result=recorded('!!pkg.exports',{prelude:`${statHelper}\nfs.Stats.prototype.isFile=function(){return this.mtimeMs>0};
function kind(file){const s=tryStatSync(file); if(s?.isFile()) return 'file'; return 'other';} export const type=kind('package.json');`})
  assert.ok(result.keys.includes('sm:package.json'))
  assert.equal(result.keys.includes('st:package.json'),false)
})
test('a native stat success check keeps type/existence without inventing a content read', () => {
  const result=recorded('!!pkg.exports',{prelude:`${statHelper}\nconst file='package.json'; export const missing=!tryStatSync(file);`})
  assert.deepEqual(query(result),[{values:[],truthy:['exports']}])
  assert.ok(result.keys.includes('st:package.json'))
  assert.equal(result.keys.includes('pj:package.json'),false)
  assert.equal(result.stats.statType,1)
})
test('escaping stats and metadata readers retain complete metadata inputs', () => {
  for (const consumer of ['export const size=tryStatSync(file).size;',
    'export const time=tryStatSync(file).mtimeMs;',
    'const s=tryStatSync(file); export const type=s.isFile();',
    'const alias=tryStatSync; export const type=alias(file)?.isFile();',
    'export const type=eval("tryStatSync(file)?.isFile()")']) {
    const result=recorded('!!pkg.exports',{prelude:`${statHelper}\nconst file='package.json'; ${consumer}`})
    assert.deepEqual(query(result),[{values:[],truthy:['exports']}])
    assert.ok(result.keys.includes('sm:package.json'),consumer)
    assert.equal(result.keys.includes('st:package.json'),false,consumer)
    assert.equal(result.stats.statType,0)
  }
})
test('modified Stats methods fail closed even at a certified source location', () => {
  const result=recorded('!!pkg.exports',{prelude:`${statHelper}\nfs.Stats.prototype.isFile=function(){return this.size>10};
const file='package.json'; export const type=tryStatSync(file)?.isFile();`})
  assert.deepEqual(query(result),[{values:[],truthy:['exports']}])
  assert.ok(result.keys.includes('sm:package.json'))
  assert.equal(result.keys.includes('st:package.json'),false)
  assert.equal(result.stats.statType,0)
})
test('replacing the Stats constructor prototype cannot retain a stale proof', () => {
  const result=recorded('!!pkg.exports',{prelude:`${statHelper}\nfs.Stats.prototype={isFile(){return this.size>10}};
const file='package.json'; export const type=tryStatSync(file)?.isFile();`})
  assert.deepEqual(query(result),[{values:[],truthy:['exports']}])
  assert.ok(result.keys.includes('sm:package.json'))
  assert.equal(result.keys.includes('st:package.json'),false)
  assert.equal(result.stats.statType,0)
})
