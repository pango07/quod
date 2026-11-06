import {test} from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

function run(lateObserver,changePrototype=false,{missing=false,changeBinding=false,observe=true,inheritedSetter=false}={}) {
  const input=new URL('./async-stat-probe.mjs',import.meta.url).pathname+(missing?'.missing':'')
  const source=`import {observeAsyncStat} from ${JSON.stringify(new URL('./async-stat-probe.mjs',import.meta.url).href)};
import fs from 'node:fs';import {promiseHooks} from 'node:v8';
let escaped=null;
if(${inheritedSetter}) Object.defineProperty(fs.Stats.prototype,'mtimeMs',{configurable:true,set(value){
  escaped=this;Object.defineProperty(this,'mtimeMs',{value,writable:true,enumerable:true,configurable:true});
}});
const probe=${observe?'observeAsyncStat(process.cwd())':'({report(){return {calls:[]}},close(){}})'};const NativeStats=fs.Stats;
const binding=process.binding('fs'), originalStat=binding.stat;
if(${changeBinding}) binding.stat=function(...args){return Reflect.apply(originalStat,this,args)};
const originalThen=Promise.prototype.then;let thenCalls=0;
Promise.prototype.then=function(...args){thenCalls++;
  if(!${missing}) throw Error('diagnostic inserted a reaction');
  return Reflect.apply(originalThen,this,args);
};
const root=fs.promises.stat(${JSON.stringify(input)});
binding.stat=originalStat;
const internal=NativeStats.prototype.constructor,originalPrototype=internal.prototype;
if(${changePrototype}) internal.prototype=Object.create(originalPrototype);
let foreign=0,stop=()=>{};
if(${lateObserver}) stop=promiseHooks.onSettled(()=>{foreign++;stop()});
let value=null,caught=false;try{value=await root}catch{caught=true}
internal.prototype=originalPrototype;
await new Promise(resolve=>setImmediate(resolve));
Promise.prototype.then=originalThen;
const report=probe.report();probe.close();
const descriptor=value && Object.getOwnPropertyDescriptor(value,'mtimeMs');
console.log(JSON.stringify({report,thenCalls,foreign,caught,escaped:escaped===value && value!==null,size:value?.size,nativeStats:value instanceof NativeStats,
 nativePrototype:value && Object.getPrototypeOf(value)===NativeStats.prototype,metadataData:descriptor && typeof descriptor.value==='number' && descriptor.get===undefined}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:15000})
  assert.equal(r.status,0,r.stderr||String(r.error));return JSON.parse(r.stdout)
}
test('passive stat tracing preserves native Stats and adds no then reaction',()=>{
  const r=run(false),op=r.report.calls[0]
  assert.equal(r.thenCalls,0)
  assert.equal(r.nativeStats,true);assert.equal(r.nativePrototype,true);assert.equal(r.metadataData,true)
  assert.equal(r.report.eventMismatch,false);assert.equal(r.report.replayEligible,false)
  assert.equal(r.report.nativeStatProvenance.available,true,JSON.stringify(r.report.nativeStatProvenance))
  assert.equal(r.report.nativeStatProvenance.stat.script.url,'node:internal/fs/promises')
  assert.equal(r.report.nativeStatProvenance.stat.script.hasSourceURL,false)
  // Node 24 exposes a deprecated public constructor wrapper. Native stat
  // uses the internal constructor, while both share the Stats prototype.
  assert.equal(r.report.nativeStatProvenance.publicConstructorMatches,false)
  assert.equal(r.report.nativeStatProvenance.publicPrototypeMatches,true)
  assert.equal(op.rootSettledFromOwnReaction,true)
  assert.equal(op.observedOnlyOwnedHooks,true)
  assert.equal(op.observedNativeUnchanged,true)
  assert.deepEqual(r.report.nativeStatProvenance.construction,{knownSources:true,fieldsAbsent:true,replayEligible:false})
  assert.equal(op.observedConstructionFieldsAbsent,true)
  assert.deepEqual(op.outcome,{available:true,state:'fulfilled',projection:'size',size:r.size,replayEligible:false})
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
test('an awaited native rejection is observed without a new reaction or error reader',()=>{
  // Node's handleErrorFromBinding returns PromiseReject(error), whose
  // adoption can invoke the public then method. Compare a plain baseline;
  // throwing from then would break native rejection handling itself.
  const baseline=run(false,false,{missing:true,observe:false})
  const r=run(false,false,{missing:true}),op=r.report.calls[0]
  assert.equal(baseline.caught,true);assert.equal(r.caught,true)
  assert.equal(r.thenCalls,baseline.thenCalls,'the diagnostic must add no public then reactions')
  assert.equal(op.observedNativeUnchanged,true)
  assert.deepEqual(op.outcome,{available:true,state:'rejected',projection:'ignored-error',replayEligible:false})
  assert.equal(r.report.replayEligible,false)
})
test('replacing the closed-over native binding cannot hide behind the public promise function',()=>{
  const r=run(false,false,{changeBinding:true}),op=r.report.calls[0]
  assert.equal(r.caught,false);assert.equal(r.nativeStats,true);assert.equal(r.thenCalls,0)
  assert.equal(op.before.native.unchanged,false)
  assert.equal(op.observedNativeUnchanged,false)
  assert.ok(op.creatorBefore.every(s=>s.native.unchanged),'restoration after dispatch must not erase earlier evidence')
  assert.equal(r.report.replayEligible,false)
})
test('a constructor-field setter installed before observation exposes an additional consumer',()=>{
  const r=run(false,false,{inheritedSetter:true}),op=r.report.calls[0]
  assert.equal(r.escaped,true,'the inherited setter obtained the actual native Stats')
  assert.equal(op.before.pristine,true,'an initial snapshot alone misses this escape')
  assert.equal(op.outcome.available,true,'a native own size field alone also misses it')
  assert.equal(r.report.nativeStatProvenance.construction.knownSources,true)
  assert.equal(r.report.nativeStatProvenance.construction.fieldsAbsent,false)
  assert.equal(op.observedConstructionFieldsAbsent,false)
  assert.equal(r.report.replayEligible,false)
})
test('a transient internal Stats prototype change is seen before native construction',()=>{
  const r=run(false,true),op=r.report.calls[0]
  assert.equal(op.before.pristine,true)
  assert.ok(op.creatorBefore.some(s=>s.pristine===false))
  assert.equal(r.nativePrototype,false,'native stat used the temporarily changed internal prototype')
  assert.equal(r.report.replayEligible,false)
})
