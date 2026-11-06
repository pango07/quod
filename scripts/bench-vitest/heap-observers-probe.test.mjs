import {test} from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {certifyAsyncStatSizeReads} from '../../crates/quod-cli/src/vitest/boolean-proof.mjs'

const module=new URL('./promise-observers-probe.mjs',import.meta.url).href
const consumer='async function size(file){try{const s=await fs.promises.stat(file);sink(s.size)}catch{}}'
function run(kind) {
  assert.equal(certifyAsyncStatSizeReads(consumer).length,1)
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
import fs from 'node:fs';import v8 from 'node:v8';import os from 'node:os';import path from 'node:path';
const kind=${JSON.stringify(kind)},cachedQuery=v8.queryObjects;
const dir=fs.mkdtempSync(path.join(os.tmpdir(),'quod-heap-observer-')),file=path.join(dir,'input');
fs.writeFileSync(file,'x'.repeat(7013));fs.utimesSync(file,1577836800,1577836800);
const p=inspectPromiseObservers(),before=p.opaqueSnapshot();
if(kind==='cached')v8.queryObjects=()=>{throw Error('replaced public wrapper')};
let sawMetadata=false,sizeValue=null,stream=null,snapshotBytes=0;
function sink(size){
 sizeValue=size;
 if(kind==='public'||kind==='cached'){
   const rows=(kind==='cached'?cachedQuery:v8.queryObjects)(fs.Stats,{format:'summary'});
   sawMetadata=rows.some(row=>row.includes('size: 7013')&&row.includes('mtimeMs: 1577836800000'));
 }
 if(kind==='stream')stream=v8.getHeapSnapshot();
 if(kind==='file')snapshotBytes=fs.statSync(v8.writeHeapSnapshot(path.join(dir,'private.heapsnapshot'))).size;
 if(kind==='flags')v8.setFlagsFromString('');
 if(kind==='near-limit')v8.setHeapSnapshotNearHeapLimit(1);
}
${consumer}
await size(file);
if(stream)for await(const chunk of stream)snapshotBytes+=chunk.length;
const after=p.opaqueSnapshot(),access=p.opaqueAccess(),inspector=p.inspectorAccess(),registries=p.snapshot();
p.close();fs.rmSync(dir,{recursive:true,force:true});
console.log(JSON.stringify({before,after,access,inspector,registries,sawMetadata,sizeValue,snapshotBytes}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:25000})
  assert.equal(r.status,0,r.stderr||String(r.error));return JSON.parse(r.stdout)
}
for(const [kind,entry] of [['none',null],['public','v8.queryObjects'],['cached','v8.queryObjects'],
  ['stream','v8.getHeapSnapshot'],['file','v8.writeHeapSnapshot'],['flags','v8.setFlagsFromString'],
  ['near-limit','v8.setHeapSnapshotNearHeapLimit']])test('native heap observer: '+kind,()=>{
  const r=run(kind)
  assert.equal(r.sizeValue,7013)
  assert.equal(r.before.available,true,JSON.stringify(r.before))
  assert.equal(r.after.available,true,JSON.stringify(r.after))
  assert.equal(r.after.replayEligible,false)
  assert.deepEqual(r.inspector.entries,[],'public heap APIs need no Inspector connection')
  assert.deepEqual(r.registries.promiseHooks,{init:0,before:0,after:0,settled:0})
  assert.equal(r.registries.asyncHooks.kTotals,0)
  assert.equal(r.sawMetadata,kind==='public'||kind==='cached',
    'queryObjects must demonstrate a metadata reader outside the accepted local size consumer')
  assert.deepEqual(r.access.entries.flatMap(e=>e.entries),entry?[entry]:[])
  if(entry)assert.equal(r.after.counts[entry],1)
  if(kind==='stream'||kind==='file')assert.ok(r.snapshotBytes>0)
})
