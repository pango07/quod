import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

function observe(body,enabled=true) {
  const source=`
import fs from 'node:fs';import os from 'node:os';import path from 'node:path';
import {createRecorder,patchFs} from ${JSON.stringify(new URL('./record.mjs',import.meta.url).href)};
const directory=fs.mkdtempSync(path.join(os.tmpdir(),'quod-fd-record-'));
const file=path.join(directory,'input');fs.writeFileSync(file,'fixture');
fs.utimesSync(file,1577836800,1577836800);
const expected=fs.statSync(file).mtimeMs;
const r=createRecorder({stateDir:'/not-quod-state',mode:'worker'});r.on=${enabled};patchFs(r);
try {${body}
r.on=false;console.log(JSON.stringify({value,expected,unobservable:[...r.unobservable],keys:[...r.keys]}));
}finally{r.on=false;fs.rmSync(directory,{recursive:true,force:true})}
`
  const result=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:10000})
  assert.equal(result.status,0,result.stderr||String(result.error))
  return JSON.parse(result.stdout)
}

for(const [name,body,reason] of [
  ['synchronous fstat',`const fd=fs.openSync(file,'r');let value;try{value=fs.fstatSync(fd).mtimeMs}finally{fs.closeSync(fd)}`,'descriptor metadata is not tracked'],
  ['callback fstat',`const fd=fs.openSync(file,'r');let value;try{value=await new Promise((resolve,reject)=>fs.fstat(fd,(e,s)=>e?reject(e):resolve(s.mtimeMs)))}finally{fs.closeSync(fd)}`,'descriptor metadata is not tracked'],
  ['FileHandle stat',`const handle=await fs.promises.open(file,'r');let value;try{value=(await handle.stat()).mtimeMs}finally{await handle.close()}`,'FileHandle operations are not tracked'],
]) {
  test(name+' preserves values and prevents recording',()=>{
    const r=observe(body);assert.equal(r.value,r.expected);assert.deepEqual(r.unobservable,[reason])
  })
  test(name+' leaves disabled observation off',()=>{
    const r=observe(body,false);assert.equal(r.value,r.expected);assert.deepEqual(r.unobservable,[])
  })
}

test('ordinary pathname reads remain observable',()=>{
  const r=observe(`const value=fs.readFileSync(file,'utf8')`)
  assert.equal(r.value,'fixture');assert.deepEqual(r.unobservable,[])
  assert.ok(r.keys.some(k=>k.startsWith('f:')&&k.endsWith('/input')))
})

test('an unlinked descriptor keeps its actual metadata and rejects recording',()=>{
  const r=observe(`const fd=fs.openSync(file,'r');fs.unlinkSync(file);
let value;try{value=fs.fstatSync(fd).mtimeMs}finally{fs.closeSync(fd)}`)
  assert.equal(r.value,r.expected);assert.deepEqual(r.unobservable,['descriptor metadata is not tracked'])
})
