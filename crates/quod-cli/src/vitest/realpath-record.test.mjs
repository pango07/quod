import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

function observe(expression,{enabled=true,buffer=false}={}) {
  const result=spawnSync(process.execPath,['--input-type=module','-e',`
import fs from 'node:fs';import os from 'node:os';import path from 'node:path';
import {createRecorder,patchFs} from ${JSON.stringify(new URL('./record.mjs',import.meta.url).href)};
const root=fs.mkdtempSync(path.join(os.tmpdir(),'quod-realpath-record-'));
fs.mkdirSync(root+'/target');fs.writeFileSync(root+'/target/file','same');
fs.symlinkSync('target',root+'/link');
const query=root+'/link/../target/file',input=${buffer?'Buffer.from(query)':'query'};
const expected=fs.realpathSync.native(input);
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
