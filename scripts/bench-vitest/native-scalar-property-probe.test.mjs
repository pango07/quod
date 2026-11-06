import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import {createHash} from 'node:crypto'
import {createRequire} from 'node:module'
import {types} from 'node:util'
import {observeScalarProperty} from './scalar-property-probe.mjs'
import {observeModuleMetadata} from './worker-module-metadata-probe.mjs'
import {inspectPromiseObservers} from './promise-observers-probe.mjs'
import {nativeOwnerGuard} from './native-owner-guard.mjs'
import {validateNativeScalarEvidence} from './native-scalar-evidence.mjs'
import {validateCompilationCounts} from './compiled-script-log.mjs'
import {validateNativeEntryLog} from './native-entry-log.mjs'

const addon=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
test.beforeEach(t=>fs.writeSync(2,`native scalar fixture start: ${t.name}\n`))
test.afterEach(t=>fs.writeSync(2,`native scalar fixture end: ${t.name}\n`))
const ownerGuard=nativeOwnerGuard(),observation='native-data'
const ownSourceHash=createHash('sha256').update(fs.readFileSync(new URL(import.meta.url),'utf8'),'utf16le').digest('hex')
function graph() {
  return class Graph {
    ensureModule(){return {id:'/module',url:'/module',file:'/module',meta:undefined}}
    invalidateModule(){}
  }
}

test('native scalar adapter retains actual caller source across aliases, descriptors and foreign realms',()=>{
  const registry=inspectPromiseObservers(),target={seed:123},original=Object.getOwnPropertyDescriptor(target,'seed')
  const observed=[],p=observeScalarProperty(target,'seed',{observation,ownerGuard,
    sourceForScript:registry.sourceForScript,onOperation:e=>observed.push(e)})
  try {
    function nativeScalarCaller(){return target.seed}
    assert.equal(nativeScalarCaller(),123)
    assert.equal(Object.create(target).seed,123)
    assert.equal(Reflect.get(target,'seed',{}),123)
    assert.deepEqual(Object.getOwnPropertyDescriptor(target,'seed'),original)
    assert.equal(JSON.stringify(target),'{"seed":123}')
    assert.equal(vm.runInNewContext('target.seed',{target},{filename:'native-scalar-foreign-reader'}),123)
    const r=p.report()
    assert.deepEqual(r.gaps,[]);assert.equal(r.recordedReads,6);assert.equal(observed.length,6)
    validateNativeScalarEvidence(r)
    assert(r.events.some(e=>e.sites[0].function==='nativeScalarCaller'&&e.source.code.sha256===ownSourceHash))
    assert(r.events.some(e=>e.source.script.url==='native-scalar-foreign-reader'))
    assert.deepEqual(p.currentValue(),{available:true,value:123,writable:true,enumerable:true,configurable:true})
    assert.equal(p.report().recordedReads,6,'reporting and snapshots must not create application reads')
    assert(!r.completeOwnership&&!r.replayEligible&&r.nativeCallbackObservable&&!r.earlierCopiesExcluded)
  }finally{p.close();registry.close()}
  assert.deepEqual(Object.getOwnPropertyDescriptor(target,'seed'),original)
})

test('missing source and callback errors preserve scalar values but make evidence unusable',()=>{
  for(const sourceForScript of [null,()=>({available:false})]) {
    const target={seed:'private-scalar'},p=observeScalarProperty(target,'seed',{observation,ownerGuard,sourceForScript})
    try {
      assert.equal(target.seed,'private-scalar')
      assert.deepEqual(p.currentValue(),{available:false})
      assert(p.report().gaps.includes('native scalar caller source unavailable'))
      assert(!JSON.stringify(p.report()).includes('private-scalar'))
    }finally{p.close()}
  }
  const registry=inspectPromiseObservers(),target={seed:123},p=observeScalarProperty(target,'seed',{
    observation,ownerGuard,sourceForScript:registry.sourceForScript,onOperation(){throw Error('private callback failure')}})
  try {
    assert.equal(target.seed,123)
    assert.deepEqual(p.currentValue(),{available:false})
    assert(p.report().gaps.includes('native scalar read observation unavailable'))
    assert(!JSON.stringify(p.report()).includes('private callback failure'))
  }finally{p.close();registry.close()}
})

test('native scalar adapter refuses existing callbacks and proxies without executing application code',()=>{
  let gets=0,traps=0
  const fixture=addon.createNativeDataFixture()
  const targets=[fixture.target,{get tmp(){gets++;return 17}},new Proxy({tmp:17},{getOwnPropertyDescriptor(){traps++;throw Error('trap')}})]
  for(const target of targets) {
    const p=observeScalarProperty(target,'tmp',{observation,ownerGuard})
    assert.equal(p.report().installed,false);assert.deepEqual(p.currentValue(),{available:false});p.close()
  }
  assert.equal(gets,0);assert.equal(traps,0);assert.equal(fixture.state.calls,0)
})

test('native module metadata retains object identity and snapshots without counting private reads',()=>{
  const registry=inspectPromiseObservers(),Graph=graph(),p=observeModuleMetadata(Graph,{observation,ownerGuard,sourceForScript:registry.sourceForScript})
  const raw={tmp:'/private-path',invalidate:false,code:'private-code'},node=new Graph().ensureModule()
  try {
    assert.equal(p.observeMetadata(raw,'rpc:fetch'),raw)
    assert.equal(raw.tmp,'/private-path');node.meta=raw
    assert.equal(node.meta,raw);assert.equal(node.meta.invalidate,false)
    assert.deepEqual(Object.getOwnPropertyDescriptor(raw,'tmp'),{value:'/private-path',writable:true,enumerable:true,configurable:true})
    assert.equal(Object.create(raw).tmp,'/private-path')
    assert.deepEqual(p.metadataForNode(node),{available:true,value:{tmp:'/private-path',invalidate:false,code:'private-code'}})
    const r=p.report()
    assert.deepEqual(r.gaps,[]);assert.deepEqual(r.errors,[])
    assert.equal(r.events.reduce((n,e)=>n+e.count,0),4)
    for(const m of r.metadata)for(const s of m.scalars)
      validateNativeScalarEvidence(s,r.events.filter(e=>e.metadata===m.id&&e.key===s.key))
    assert(!JSON.stringify(r).includes('/private-path'));assert(!JSON.stringify(r).includes('private-code'))
    assert.equal(r.completeOwnership,false);assert.equal(r.replayEligible,false)
  }finally{p.close();registry.close()}
  assert.equal(node.meta,raw);assert.equal(raw.tmp,'/private-path')
})

test('native metadata callback loss cannot be hidden by restoring the original data descriptor',()=>{
  for(const mutate of [raw=>{raw.tmp='/original'},raw=>{const d=Object.getOwnPropertyDescriptor(raw,'tmp');delete raw.tmp;Object.defineProperty(raw,'tmp',d)}]) {
    const registry=inspectPromiseObservers(),Graph=graph(),p=observeModuleMetadata(Graph,{observation,ownerGuard,sourceForScript:registry.sourceForScript})
    const raw={tmp:'/original'},node=new Graph().ensureModule();node.meta=raw
    try {
      mutate(raw)
      assert.equal(raw.tmp,'/original');assert.equal(p.metadataForNode(node).available,false)
      assert(p.report().gaps.includes('metadata scalar descriptor unavailable: tmp'))
    }finally{p.close();registry.close()}
  }
})

test('native metadata rejects replacement getters and newly added unobserved fields',()=>{
  const registry=inspectPromiseObservers(),Graph=graph(),p=observeModuleMetadata(Graph,{observation,ownerGuard,sourceForScript:registry.sourceForScript})
  const raw={tmp:'/original'},node=new Graph().ensureModule();node.meta=raw
  let gets=0
  try {
    Object.defineProperty(raw,'tmp',{get(){gets++;return '/replacement'},configurable:true})
    raw.invalidate=true
    assert.equal(p.metadataForNode(node).available,false);assert.equal(gets,0)
    const r=p.report();assert(r.gaps.includes('unobserved metadata field added: invalidate'))
    assert(r.gaps.includes('metadata scalar descriptor unavailable: tmp'))
  }finally{p.close();registry.close()}
  assert.equal(gets,0)
})

test('native scalar evidence rejects missing events, missing source frames and mismatched script identity',()=>{
  const registry=inspectPromiseObservers(),target={seed:123},p=observeScalarProperty(target,'seed',{observation,ownerGuard,sourceForScript:registry.sourceForScript})
  try {
    assert.equal(target.seed,123);const r=p.report();validateNativeScalarEvidence(r)
    assert.throws(()=>validateNativeScalarEvidence(r,[]))
    for(const corrupt of [
      r=>r.events[0].sources.pop(),
      r=>{r.events[0].sources[0].scriptId='wrong-script'},
      r=>{r.native.calls++},
      r=>{r.native.available=false},
    ]) {const copy=structuredClone(r);corrupt(copy);assert.throws(()=>validateNativeScalarEvidence(copy))}
  }finally{p.close();registry.close()}
})

test('direct scalar reads coexist with actual native proxy arguments and exact compiled source capture',()=>{
  fs.writeSync(2,'native scalar combined: registry setup\n')
  const registry=inspectPromiseObservers({proxyAccess:true,hookAccess:true,compiledScripts:true,
    nativeEntries:true,ordinaryOwners:true,nativeTransport:'string',descriptorAccess:false,
    onInspectorCommand(command){
      if(['Runtime.evaluate','Runtime.queryObjects'].includes(command.method))
        fs.writeSync(2,`native scalar combined command: ${JSON.stringify(command)}\n`)
    }})
  fs.writeSync(2,'native scalar combined: registry ready\n')
  const target={seed:123},p=observeScalarProperty(target,'seed',{observation,ownerGuard,sourceForScript:registry.sourceForScript})
  fs.writeSync(2,'native scalar combined: scalar installed\n')
  try {
    assert.equal(target.seed,123)
    fs.writeSync(2,'native scalar combined: direct read\n')
    assert.equal(types.isProxy(target),false)
    fs.writeSync(2,'native scalar combined: proxy census\n')
    assert.equal(Object.getOwnPropertyDescriptor(target,'seed').value,123)
    fs.writeSync(2,'native scalar combined: descriptor read\n')
    assert.equal(vm.runInThisContext('40 + 2',{filename:'native-scalar-compiled-reader'}),42)
    fs.writeSync(2,'native scalar combined: compilation\n')
    const scalar=p.report();validateNativeScalarEvidence(scalar);assert.equal(scalar.recordedReads,2)
    const descriptors=registry.descriptorAccess(),proxy=registry.proxyAccess(),identity=registry.nativeIdentity()
    assert.equal(descriptors.available,false);assert.equal(descriptors.calls,0)
    assert.deepEqual(descriptors.entries,[]);assert.equal(proxy.available,true)
    validateNativeEntryLog(descriptors);validateNativeEntryLog(proxy)
    assert.equal(identity.calls,proxy.calls);assert.equal(identity.calls,identity.cleared)
    assert.equal(identity.released,0);assert(identity.available&&identity.calls>0)
    assert.equal(identity.contextCapture.failures,0)
    assert(registry.hookAccess().available&&registry.opaqueAccess().available&&registry.inspectorAccess().available)
    const compiled=registry.compiledScriptReport();validateCompilationCounts(compiled)
    assert(compiled.enabled&&!compiled.overflow&&!compiled.errors.length&&!compiled.replayEligible)
  }finally{fs.writeSync(2,'native scalar combined: cleanup\n');p.close();registry.close()}
})
