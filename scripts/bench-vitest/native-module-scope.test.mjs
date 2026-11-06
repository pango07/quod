import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const observer=new URL('./promise-observers-probe.mjs',import.meta.url).href
function fixture(body,mode='descriptors',heap=true) {
  // Export the ballast so V8 cannot omit unused bindings from the module
  // context; the regression must exercise an actually large scope.
  const ballast=Array.from({length:2000},(_,i)=>`export const unused${i}=${i};`).join('\n')
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(observer)};
const EVALUATE=${mode==='evaluate'},LOOKUP=${mode==='lookup'},HEAP=${heap};
${ballast}
let gets=0;const target={get tmp(){gets++;return 'do-not-read'}},plain={tmp:7},key='tmp';
${body}`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error))
  const data=JSON.parse(r.stdout)
  assert.equal(data.gets,0);assert.equal(data.report.available,true)
  assert.equal(data.report.overflow,false);assert.equal(data.report.replayEligible,false)
  return data
}

for(const mode of ['descriptors','evaluate','lookup']) {
const suffix=' ('+mode+')'
test('a missing module binding skips a large scope and still leaves globals unknown'+suffix,()=>{
  const data=fixture(`const native=Object.getOwnPropertyDescriptor;
globalThis.__quodNativeScopeTarget=target;
function invoke(key){return native(__quodNativeScopeTarget,key)}
const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:true,moduleBindingEvaluation:EVALUATE,moduleBindingLookup:LOOKUP});
const value=invoke(key);const report=p.descriptorAccess();p.close();delete globalThis.__quodNativeScopeTarget;
console.log(JSON.stringify({report,gets,found:typeof value.get==='function'}));`,mode)
  assert.equal(data.found,true)
  assert.equal(data.report.calls,1)
  assert.equal(data.report.entries[0].argument.available,false)
  assert.equal(data.report.entries[0].argument.reason,'unsupported argument scope')
  assert(data.report.argumentDiagnostics.skippedModuleScopes>0,JSON.stringify(data.report))
  assert.equal(data.report.argumentDiagnostics.propertyRequests.module??0,0)
})

test('mutable module aliases and local targets are read anew at the same call location'+suffix,()=>{
  const data=fixture(`let native=Object.getOwnPropertyDescriptor;
function invoke(target,key){return native(target,key)}
const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:true,moduleBindingEvaluation:EVALUATE,moduleBindingLookup:LOOKUP});
const a=invoke(target,key);native=Reflect.getOwnPropertyDescriptor;
const b=invoke(plain,key);native=Object.getOwnPropertyDescriptor;const c=invoke(target,key);
const report=p.descriptorAccess();p.close();
console.log(JSON.stringify({report,gets,values:[typeof a.get,b.value,typeof c.get]}));`,mode)
  assert.deepEqual(data.values,['function',7,'function'])
  assert.equal(data.report.calls,3)
  const entries=data.report.entries
  assert(entries.every(e=>e.argument.available&&e.argument.kind==='object'),JSON.stringify(entries))
  assert(entries.some(e=>e.entries.includes('Reflect.getOwnPropertyDescriptor')&&e.argument.target.watchedAccessorKeys.length===0))
  assert(entries.some(e=>e.entries.includes('Object.getOwnPropertyDescriptor')&&e.argument.target.watchedAccessorKeys.includes('tmp')))
  if(mode==='evaluate') {
    assert.equal(data.report.argumentDiagnostics.moduleBindingEvaluations,3)
    assert.equal(data.report.argumentDiagnostics.moduleBindingEvaluationErrors,0)
    assert.equal(data.report.argumentDiagnostics.propertyRequests.module??0,0)
  }else if(mode==='lookup') {
    assert.equal(data.report.argumentDiagnostics.moduleBindingLookups,3)
    assert.equal(data.report.argumentDiagnostics.moduleBindingLookupErrors,0)
    assert.equal(data.report.argumentDiagnostics.propertyRequests.module??0,0)
  }else assert(data.report.argumentDiagnostics.propertyEntries.module>2000,
    'declared bindings must still use current module descriptors: '+JSON.stringify(data.report.argumentDiagnostics))
})

test('destructuring declarations and imported live aliases remain visible'+suffix,()=>{
  const imported='data:text/javascript,'+encodeURIComponent('export let native=Object.getOwnPropertyDescriptor;export function flip(){native=Reflect.getOwnPropertyDescriptor}')
  const data=fixture(`import {native as live,flip} from ${JSON.stringify(imported)};
const {getOwnPropertyDescriptor:destructured}=Object;
function invoke(target,key){return live(target,key)}
const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:true,moduleBindingEvaluation:EVALUATE,moduleBindingLookup:LOOKUP});
destructured(target,key);invoke(target,key);flip();invoke(plain,key);
const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets}));`,mode)
  assert.equal(data.report.calls,3)
  assert(data.report.entries.every(e=>e.argument.available),JSON.stringify(data.report))
  assert(data.report.entries.some(e=>e.entries.includes('Reflect.getOwnPropertyDescriptor')&&e.argument.target.watchedAccessorKeys.length===0))
})

test('dynamic local bindings and shadowed closure bindings are never excluded by module syntax'+suffix,()=>{
  const data=fixture(`const dynamic=Function('target','key','eval("var native=Object.getOwnPropertyDescriptor");return native(target,key)');
let native=Object.getOwnPropertyDescriptor;
function factory(native){return function invoke(target,key){return native(target,key)}}
const shadowed=factory(Reflect.getOwnPropertyDescriptor);
const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:true,moduleBindingEvaluation:EVALUATE,moduleBindingLookup:LOOKUP});
dynamic(target,key);shadowed(plain,key);
const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets}));`,mode)
  assert.equal(data.report.calls,2)
  assert(data.report.entries.every(e=>e.argument.available),JSON.stringify(data.report))
  assert(data.report.entries.some(e=>e.entries.includes('Object.getOwnPropertyDescriptor')&&e.argument.target.watchedAccessorKeys.includes('tmp')))
  assert(data.report.entries.some(e=>e.entries.includes('Reflect.getOwnPropertyDescriptor')&&e.argument.target.watchedAccessorKeys.length===0))
})
}

for(const mode of ['evaluate','lookup']) test('binding access does not flush a queued target replacement during the native pause ('+mode+')',()=>{
  const data=fixture(`let first=target,microtasks=0;const native=Object.getOwnPropertyDescriptor;
function invoke(){return native(first,key)}
const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:true,moduleBindingEvaluation:EVALUATE,moduleBindingLookup:LOOKUP});
queueMicrotask(()=>{first=plain;microtasks++});const value=invoke();
const report=p.descriptorAccess();p.close();const during=microtasks;await Promise.resolve();
console.log(JSON.stringify({report,gets,during,after:microtasks,moved:first===plain,found:typeof value.get==='function'}));`,mode)
  assert.equal(data.during,0);assert.equal(data.after,1);assert.equal(data.moved,true);assert.equal(data.found,true)
  assert.equal(data.report.calls,1)
  assert(data.report.entries[0].argument.available,JSON.stringify(data.report))
  assert(data.report.entries[0].argument.target.watchedAccessorKeys.includes('tmp'))
  const count=mode==='evaluate'?'moduleBindingEvaluations':'moduleBindingLookups'
  const errors=mode==='evaluate'?'moduleBindingEvaluationErrors':'moduleBindingLookupErrors'
  assert(data.report.argumentDiagnostics[count]>0)
  assert.equal(data.report.argumentDiagnostics[errors],0)
})

test('compare scope-read overhead with equal calls, values and native identity checks',t=>{
  const measurements=[]
  for(const [mode,heap] of [['descriptors',true],['evaluate',true],['lookup',true],['lookup',false]]) {
    const data=fixture(`const native=Object.getOwnPropertyDescriptor;
function invoke(target,key){return native(target,key)}
const now=performance.now.bind(performance);now();
const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:HEAP,moduleBindingEvaluation:EVALUATE,moduleBindingLookup:LOOKUP});
let accessorCount=0,valueCount=0;const start=now();
for(let i=0;i<300;i++) {const value=invoke(i%2?target:plain,key);if(value.get)accessorCount++;else valueCount+=value.value}
const elapsedMs=now()-start,report=p.descriptorAccess(),identity=p.nativeIdentity();p.close();
console.log(JSON.stringify({report,identity,gets,accessorCount,valueCount,elapsedMs}));`,mode,heap)
    assert.equal(data.accessorCount,150);assert.equal(data.valueCount,1050)
    assert.equal(data.report.calls,300)
    const counters=data.report.argumentDiagnostics
    const identityChecks=heap?data.identity.comparisons:counters.compiledIdentityComparisons
    assert.equal(identityChecks,300)
    if(heap)assert.equal(data.identity.unknown,0)
    assert(data.report.entries.every(e=>e.argument.available),JSON.stringify(data.report))
    if(mode==='descriptors')assert(counters.propertyEntries.module>=600000)
    else {
      assert.equal(counters.propertyRequests.module??0,0)
      assert.equal(counters[mode==='evaluate'?'moduleBindingEvaluations':'moduleBindingLookups'],300)
    }
    measurements.push({mode,heapIdentity:heap,calls:data.report.calls,elapsedMs:data.elapsedMs,identityChecks,
      transferredModuleEntries:counters.propertyEntries.module??0,replayEligible:false})
  }
  // Timing is evidence, not a flaky pass/fail threshold or product claim.
  t.diagnostic(JSON.stringify({scopeReadMeasurements:measurements}))
})
