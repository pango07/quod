// Candidate investigation only: can V8's native data-property callback
// avoid replacing a scalar with a visible JavaScript accessor? This is a
// differential inventory, not an input-ownership or replay certificate.
import test from 'node:test'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {inspect} from 'node:util'
import {serialize,deserialize} from 'node:v8'
import vm from 'node:vm'

const addon=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
const original='host callback value'
function make(native) {
  if(native)return addon.createNativeDataFixture()
  return {target:Object.defineProperty(Object.create(null),'tmp',
    {value:original,writable:true,enumerable:true,configurable:true}),state:{calls:0}}
}
function snapshot(target) {
  return {descriptor:Object.getOwnPropertyDescriptor(target,'tmp'),keys:Reflect.ownKeys(target),
    extensible:Object.isExtensible(target),frozen:Object.isFrozen(target),sealed:Object.isSealed(target),
    nullPrototype:Object.getPrototypeOf(target)===null}
}
function run(native,operation) {
  const {target,state}=make(native)
  let result,error=null
  try{result=operation(target)}catch(e){error=e.name}
  const operationReads=state.calls,after=snapshot(target)
  return {result,error,after,operationReads,snapshotReads:state.calls-operationReads}
}
const reads=[
  ['direct',t=>t.tmp],['reflect',t=>Reflect.get(t,'tmp')],
  ['foreign receiver',t=>Reflect.get(t,'tmp',{})],
  ['inherited',t=>Object.create(t).tmp],['proxy receiver',t=>new Proxy(t,{}).tmp],
  ['descriptor',t=>Object.getOwnPropertyDescriptor(t,'tmp')],
  ['reflect descriptor',t=>Reflect.getOwnPropertyDescriptor(t,'tmp')],
  ['all descriptors',t=>Object.getOwnPropertyDescriptors(t)],
  ['descriptor clone',t=>Object.create(null,Object.getOwnPropertyDescriptors(t)).tmp],
  ['assign',t=>Object.assign({},t)],['spread',t=>({...t})],['entries',t=>Object.entries(t)],
  ['values',t=>Object.values(t)],['json',t=>JSON.stringify(t)],
  ['structured clone',t=>structuredClone(t)],['v8 serialization',t=>deserialize(serialize(t))],
  ['inspect',t=>inspect(t)],['inspect getters false',t=>inspect(t,{getters:false})],
  ['foreign realm',t=>vm.runInNewContext('value.tmp',{value:t})],
  ['legacy getter lookup',t=>Object.prototype.__lookupGetter__.call(t,'tmp')],
  ['legacy setter lookup',t=>Object.prototype.__lookupSetter__.call(t,'tmp')],
  ['keys only',t=>Object.keys(t)],['has only',t=>Object.hasOwn(t,'tmp')],
]
const writes=[
  ['strict assignment',t=>{t.tmp=42;return t.tmp}],
  ['sloppy assignment',t=>Function('t','t.tmp=42; return t.tmp')(t)],
  ['reflect assignment',t=>[Reflect.set(t,'tmp',42),t.tmp]],
  ['foreign receiver',t=>{const r={};return [Reflect.set(t,'tmp',42,r),r.tmp,t.tmp]}],
  ['inherited receiver',t=>{const r=Object.create(t);return [Reflect.set(t,'tmp',42,r),r.tmp,t.tmp]}],
  ['frozen receiver',t=>Reflect.set(t,'tmp',42,Object.freeze({}))],
  ['primitive receiver',t=>Reflect.set(t,'tmp',42,7)],
  ['getter receiver',t=>Reflect.set(t,'tmp',42,{get tmp(){return 19}})],
  ['readonly receiver',t=>Reflect.set(t,'tmp',42,Object.defineProperty({},'tmp',{value:19}))],
  ['proxy rejected definition',t=>{const events=[],r=new Proxy({}, {
    getOwnPropertyDescriptor(o,k){events.push(['descriptor',k]);return Reflect.getOwnPropertyDescriptor(o,k)},
    defineProperty(o,k,d){events.push(['define',k,d]);return false}});
    return [Reflect.set(t,'tmp',42,r),events]}],
  ['proxy accepted definition',t=>{const events=[],r=new Proxy({}, {
    getOwnPropertyDescriptor(o,k){events.push(['descriptor',k]);return Reflect.getOwnPropertyDescriptor(o,k)},
    defineProperty(o,k,d){events.push(['define',k,d]);return Reflect.defineProperty(o,k,d)}});
    return [Reflect.set(t,'tmp',42,r),events,r.tmp]}],
  ['define value',t=>{Object.defineProperty(t,'tmp',{value:42});return t.tmp}],
  ['reflect define value',t=>[Reflect.defineProperty(t,'tmp',{value:42}),t.tmp]],
  ['define same value',t=>{Object.defineProperty(t,'tmp',{value:original});return t.tmp}],
  ['define readonly',t=>{Object.defineProperty(t,'tmp',{writable:false});return Reflect.set(t,'tmp',42)}],
  ['define nonconfigurable',t=>{Object.defineProperty(t,'tmp',{configurable:false});return t.tmp}],
  ['delete',t=>delete t.tmp],['reflect delete',t=>Reflect.deleteProperty(t,'tmp')],
  ['freeze',t=>{Object.freeze(t);return [t.tmp,Reflect.set(t,'tmp',42)]}],
  ['seal',t=>{Object.seal(t);return [Reflect.set(t,'tmp',42),t.tmp]}],
  ['prevent extensions',t=>{Object.preventExtensions(t);return [Reflect.set(t,'tmp',42),t.tmp]}],
]

test('inventory native data-property reads and mutations against ordinary data',t=>{
  assert.equal(process.versions.node,'24.18.0')
  const rows=[]
  for(const [kind,operations] of [['read',reads],['write',writes]])for(const [name,operation] of operations) {
    const plain=run(false,operation),native=run(true,operation)
    let equal=true
    try{assert.deepEqual({result:native.result,error:native.error,after:native.after},
      {result:plain.result,error:plain.error,after:plain.after})}catch{equal=false}
    assert(Number.isSafeInteger(native.operationReads)&&native.operationReads>=0)
    assert(Number.isSafeInteger(native.snapshotReads)&&native.snapshotReads>=0)
    rows.push({kind,name,equal,plain,native})
  }
  // Inventory differences rather than assuming this API behaves like data.
  // A green test here means all cases were measured, not that they matched.
  assert.equal(rows.length,44)
  t.diagnostic(JSON.stringify({nativeDataPropertyInventory:rows,
    differences:rows.filter(r=>!r.equal).map(({kind,name})=>({kind,name})),
    replayEligible:false,scope:'Pinned native fixture only; no application instrumentation or ownership proof'}))
})
