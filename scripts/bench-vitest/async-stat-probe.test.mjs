import {test} from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

function run(lateObserver) {
  const source=`import {observeAsyncStat} from ${JSON.stringify(new URL('./async-stat-probe.mjs',import.meta.url).href)};
import fs from 'node:fs';import {promiseHooks} from 'node:v8';
const probe=observeAsyncStat(process.cwd());const NativeStats=fs.Stats;
const originalThen=Promise.prototype.then;let thenCalls=0;
Promise.prototype.then=function(){thenCalls++;throw Error('diagnostic inserted a reaction')};
const root=fs.promises.stat(${JSON.stringify(new URL('./async-stat-probe.mjs',import.meta.url).pathname)});
let foreign=0,stop=()=>{};
if(${lateObserver}) stop=promiseHooks.onSettled(()=>{foreign++;stop()});
const value=await root;
await new Promise(resolve=>setImmediate(resolve));
Promise.prototype.then=originalThen;
const report=probe.report();probe.close();
const descriptor=Object.getOwnPropertyDescriptor(value,'mtimeMs');
console.log(JSON.stringify({report,thenCalls,foreign,nativeStats:value instanceof NativeStats,
 nativePrototype:Object.getPrototypeOf(value)===NativeStats.prototype,metadataData:typeof descriptor.value==='number' && descriptor.get===undefined}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:15000})
  assert.equal(r.status,0,r.stderr||String(r.error));return JSON.parse(r.stdout)
}
test('passive stat tracing preserves native Stats and adds no then reaction',()=>{
  const r=run(false),op=r.report.calls[0]
  assert.equal(r.thenCalls,0)
  assert.equal(r.nativeStats,true);assert.equal(r.nativePrototype,true);assert.equal(r.metadataData,true)
  assert.equal(r.report.eventMismatch,false);assert.equal(r.report.replayEligible,false)
  assert.equal(op.rootSettledFromOwnReaction,true)
  assert.equal(op.observedOnlyOwnedHooks,true)
  assert.ok(op.promises.some(p=>!p.nativeCall),'the awaiting consumer must also be observed')
})
test('an observer installed during I/O cannot disappear between root checkpoints',()=>{
  const r=run(true),op=r.report.calls[0]
  assert.equal(r.foreign,1)
  assert.equal(op.before.observers.onlyOwnedPromiseHooks,true)
  assert.ok(op.creatorBefore.every(s=>s.observers.onlyOwnedPromiseHooks===true))
  assert.equal(op.observedOnlyOwnedHooks,false,'retain the earlier binding-promise observation')
  assert.ok(op.promises.some(p=>p.settled?.observers.onlyOwnedPromiseHooks===false))
  assert.equal(r.report.replayEligible,false)
})
