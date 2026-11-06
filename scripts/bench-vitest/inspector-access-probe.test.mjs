import {test} from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {certifyAsyncStatSizeReads} from '../../crates/quod-cli/src/vitest/boolean-proof.mjs'

const module=new URL('./promise-observers-probe.mjs',import.meta.url).href
const consumer='async function size(file){try{const s=await fs.promises.stat(file);sink(s.size)}catch{}}'
function run(kind) {
  assert.equal(certifyAsyncStatSizeReads(consumer).length,1,'exercise an accepted local consumer')
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
import {Session} from 'node:inspector';import fs from 'node:fs';
const kind=${JSON.stringify(kind)};
// Cache the actual native constructor before the diagnostic exists.
const Connection=process.binding('inspector').Connection;
let foreign=null;
if(kind==='existing'){foreign=new Session();foreign.connect()}
const p=inspectPromiseObservers(),before=p.inspectorAccess(),registries=p.snapshot();
let results=new Map(),nextId=0;
if(kind==='public'){foreign=new Session();foreign.connect()}
if(kind==='cached') foreign=new Connection(message=>{const r=JSON.parse(message);if(r.id)results.set(r.id,r)});
function post(method,params={}){
 if(kind==='cached'){
   const id=++nextId;foreign.dispatch(JSON.stringify({id,method,params}));
   const r=results.get(id);results.delete(id);if(!r||r.error)throw Error('native dispatch failed');return r.result;
 }
 let result,error;foreign.post(method,params,(e,r)=>{error=e;result=r});
 if(error||!result)throw Error('session dispatch failed');return result;
}
let sawMetadata=false,sizeValue=null;
function sink(size){
 sizeValue=size;
 if(!foreign)return size;
 const prototype=post('Runtime.evaluate',{expression:"process.getBuiltinModule('fs').Stats.prototype"}).result.objectId;
 const objects=post('Runtime.queryObjects',{prototypeObjectId:prototype}).objects.objectId;
 for(const row of post('Runtime.getProperties',{objectId:objects,ownProperties:true}).result){
   if(!/^\\d+$/.test(row.name)||!row.value.objectId)continue;
   const props=post('Runtime.getProperties',{objectId:row.value.objectId,ownProperties:true}).result;
   if(props.some(p=>p.name==='size'&&p.value?.value===size)&&props.some(p=>p.name==='mtimeMs'&&typeof p.value?.value==='number'))sawMetadata=true;
 }
 return size;
}
// The lexical consumer exposes only a primitive. Inspector can still
// obtain the live Stats through the heap while the sink executes.
${consumer}
await size(${JSON.stringify(new URL('./inspector-access-probe.test.mjs',import.meta.url).pathname)});
if(foreign)foreign.disconnect();
const after=p.inspectorAccess(),afterRegistries=p.snapshot();p.close();
console.log(JSON.stringify({before,after,registries,afterRegistries,sawMetadata,sizeValue}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:15000})
  assert.equal(r.status,0,r.stderr||String(r.error))
  return JSON.parse(r.stdout)
}

let baselineCount=null
for(const kind of ['none','existing','public','cached']) test('Inspector entry diagnostic: '+kind,t=>{
  const r=run(kind)
  assert.ok(r.sizeValue>0)
  assert.equal(r.before.available,true,JSON.stringify(r.before))
  assert.equal(r.after.available,true,JSON.stringify(r.after))
  assert.equal(r.after.replayEligible,false)
  assert.deepEqual(r.registries.promiseHooks,{init:0,before:0,after:0,settled:0})
  assert.deepEqual(r.afterRegistries.promiseHooks,r.registries.promiseHooks)
  assert.equal(r.afterRegistries.asyncHooks.kTotals,0)
  assert.equal(r.before.initial.listenerOpen,false,'a local session need not open a network listener')
  if(kind==='none') baselineCount=r.before.initial.localConnectionObjects
  assert.ok(baselineCount>=1)
  assert.equal(r.before.initial.localConnectionObjects,baselineCount+(kind==='existing'?1:0),JSON.stringify(r))
  assert.equal(r.sawMetadata,kind!=='none','a local size consumer does not exclude heap observation')
  const entries=r.after.entries.flatMap(e=>e.entries)
  assert.deepEqual(entries,kind==='public'||kind==='cached'?['Connection']:[],JSON.stringify(r))
  t.diagnostic(JSON.stringify({kind,initial:r.before.initial,entries,sawMetadata:r.sawMetadata}))
})
