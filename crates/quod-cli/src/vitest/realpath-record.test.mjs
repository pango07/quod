import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

function observe(expression,{enabled=true,buffer=false}={}) {
  const result=spawnSync(process.execPath,['--input-type=module','-e',`
import fs from 'node:fs';import os from 'node:os';import path from 'node:path';
import {createRequire} from 'node:module';
import {createRecorder,patchFs} from ${JSON.stringify(new URL('./record.mjs',import.meta.url).href)};
const require=createRequire(import.meta.url);
const root=fs.mkdtempSync(path.join(os.tmpdir(),'quod-realpath-record-'));
fs.mkdirSync(root+'/target');fs.writeFileSync(root+'/target/file','same');
fs.symlinkSync('target',root+'/link');
const query=root+'/link/../target/file',input=${buffer?'Buffer.from(query)':'query'};
const originalNative=fs.realpathSync.native,expected=originalNative(input);
const r=createRecorder({stateDir:'/not-quod-state',mode:'worker'});r.on=${enabled};patchFs(r);
try{const value=${expression};r.on=false;
console.log(JSON.stringify({value,expected,query,keys:[...r.keys],unobservable:[...r.unobservable]}));
}finally{r.on=false;fs.rmSync(root,{recursive:true,force:true})}
`],{encoding:'utf8',timeout:10000})
  assert.equal(result.status,0,result.stderr||String(result.error))
  return JSON.parse(result.stdout)
}

for(const [name,expression] of [
  ['sync','fs.realpathSync(input)'],
  ['native sync','fs.realpathSync.native(input)'],
  ['callback','await new Promise((resolve,reject)=>fs.realpath(input,(e,v)=>e?reject(e):resolve(v)))'],
  ['native callback','await new Promise((resolve,reject)=>fs.realpath.native(input,(e,v)=>e?reject(e):resolve(v)))'],
  ['promise','await fs.promises.realpath(input)'],
]) {
  test(name+' preserves the result and keys the complete raw path query',()=>{
    const row=observe(expression)
    assert.equal(row.value,row.expected);assert.deepEqual(row.unobservable,[])
    assert.ok(row.keys.includes('rp:'+row.query),'do not erase a symlink followed by ..')
  })
}
test('disabled realpath observation stays off',()=>{
  const row=observe('fs.realpathSync.native(input)',{enabled:false})
  assert.equal(row.value,row.expected);assert.deepEqual(row.keys,[]);assert.deepEqual(row.unobservable,[])
})
test('unsupported Buffer argument retains native behavior and prevents replay',()=>{
  const row=observe('fs.realpathSync.native(input)',{buffer:true})
  assert.equal(row.value,row.expected);assert.deepEqual(row.unobservable,['unsupported realpath argument'])
})
test('options getters retain their evaluation count and prevent replay',()=>{
  const row=observe(`(()=>{let reads=0;const options={get encoding(){reads++;return 'utf8'}};
originalNative(input,options);const ordinary=reads;reads=0;
const resolved=fs.realpathSync.native(input,options);return {resolved,ordinary,observed:reads}})()`)
  assert.equal(row.value.resolved,row.expected)
  assert.equal(row.value.observed,row.value.ordinary)
  assert.deepEqual(row.unobservable,['unsupported realpath options'])
})
test('ordinary data options preserve the operation without fallback',()=>{
  const row=observe("fs.realpathSync.native(input,{encoding:'utf8'})")
  assert.equal(row.value,row.expected);assert.deepEqual(row.unobservable,[])
  assert.ok(row.keys.includes('rp:'+row.query))
})
test('Node module resolution retains its cache and resolution dependency',()=>{
  const row=observe('require.resolve(input)')
  assert.equal(row.value,row.expected);assert.deepEqual(row.unobservable,[])
  assert.ok(row.keys.some(k=>k.startsWith('rp:')&&k.endsWith('/target/file')))
})

test('only a newly written main-process config bundle is an owned artifact',()=>{
  const result=spawnSync(process.execPath,['--input-type=module','-e',`
import fs from 'node:fs';import os from 'node:os';import path from 'node:path';
import {createRecorder,patchFs} from ${JSON.stringify(new URL('./record.mjs',import.meta.url).href)};
const root=fs.mkdtempSync(path.join(os.tmpdir(),'quod-realpath-bundle-'));
const dir=root+'/node_modules/.vite-temp';fs.mkdirSync(dir,{recursive:true});
const existing=dir+'/vitest.config.mjs.timestamp-123-aaa.mjs',fresh=dir+'/vitest.config.mjs.timestamp-456-bbb.mjs';
const other=dir+'/other.mjs';fs.writeFileSync(existing,'existing');
const r=createRecorder({stateDir:'/not-quod-state',mode:'main'});patchFs(r);
try{fs.writeFileSync(fresh,'generated');fs.writeFileSync(other,'other');
fs.realpathSync.native(fresh);fs.realpathSync.native(existing);fs.realpathSync.native(other);
r.on=false;console.log(JSON.stringify({keys:[...r.keys],fresh,existing,other}));
}finally{r.on=false;fs.rmSync(root,{recursive:true,force:true})}
`],{encoding:'utf8',timeout:10000})
  assert.equal(result.status,0,result.stderr||String(result.error))
  const row=JSON.parse(result.stdout)
  assert.ok(!row.keys.includes('rp:'+row.fresh))
  assert.ok(row.keys.includes('rp:'+row.existing))
  assert.ok(row.keys.includes('rp:'+row.other))
})
