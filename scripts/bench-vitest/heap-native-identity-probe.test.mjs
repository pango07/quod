import test from 'node:test'
import assert from 'node:assert/strict'
import {Session} from 'node:inspector'
import {performance} from 'node:perf_hooks'
import {inspectPromiseObservers} from './promise-observers-probe.mjs'
import {observeHeapNativeIdentity} from './heap-native-identity-probe.mjs'

test('heap identities distinguish aliases and identical-source closures without compiled comparisons or snapshots',t=>{
  const session=new Session(),commands=[],scripts=[]
  let chunks=0
  function post(method,params={}) {
    commands.push(method)
    let done=false,error,result
    session.post(method,params,(e,r)=>{done=true;error=e;result=r})
    if(!done||error)throw error??Error('asynchronous inspector response')
    return result
  }
  const evaluate=expression=>post('Runtime.evaluate',{expression,objectGroup:'quod-heap-identity-fixture'}).result.objectId
  session.on('Debugger.scriptParsed',e=>scripts.push(e.params.scriptId))
  session.on('HeapProfiler.addHeapSnapshotChunk',()=>chunks++)
  session.connect();post('Debugger.enable')
  let p
  try {
    const left=evaluate('Object.getOwnPropertyDescriptor'),alias=evaluate('Object.getOwnPropertyDescriptor')
    const other=evaluate('Reflect.getOwnPropertyDescriptor')
    const closureA=evaluate('(()=>1)'),closureB=evaluate('(()=>1)')
    assert.notEqual(left,alias,'different remote handles must not be treated as different JS objects')
    p=observeHeapNativeIdentity(post);assert.equal(p.report().available,true,JSON.stringify(p.report()))
    assert.deepEqual(p.compare(left,alias),{available:true,same:true})
    assert.deepEqual(p.compare(left,other),{available:true,same:false})
    assert.deepEqual(p.compare(closureA,closureB),{available:true,same:false})
    const before=scripts.length,iterations=100,start=performance.now()
    for(let i=0;i<iterations;i++)assert.deepEqual(p.compare(left,alias),{available:true,same:true})
    const heapMs=performance.now()-start
    assert.equal(scripts.length,before,'identity reads must not compile JavaScript')
    const controlStart=performance.now()
    for(let i=0;i<iterations;i++) {
      const r=post('Runtime.callFunctionOn',{objectId:left,functionDeclaration:'function(other){return this===other}',
        arguments:[{objectId:alias}],returnByValue:true,silent:true})
      assert.equal(r.result.value,true)
    }
    const controlMs=performance.now()-controlStart
    assert(scripts.length>before,'control must exercise the source-compiling comparison')
    t.diagnostic(JSON.stringify({iterations,heapMs,controlMs,controlScriptEvents:scripts.length-before}))
    post('HeapProfiler.disable')
    const cleared=p.compare(left,alias)
    assert.equal(cleared.available,false,'two cleared/zero IDs must not establish identity')
    assert.equal(p.report().unknown,1)
    p.close();assert.equal(p.compare(left,alias).available,false)
    assert.equal(chunks,0)
    assert(!commands.includes('HeapProfiler.takeHeapSnapshot')&&!commands.includes('HeapProfiler.stopTrackingHeapObjects'))
  }finally{p?.close();post('Runtime.releaseObjectGroup',{objectGroup:'quod-heap-identity-fixture'});session.disconnect()}
})

test('native descriptor recovery uses heap identity and still rejects bound callees',()=>{
  const native=Object.getOwnPropertyDescriptor,target={get tmp(){throw Error('must not read')}},plain={tmp:7},key='tmp'
  const bound=native.bind(null,target)
  const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:true})
  try {
    const values=[]
    for(const current of [target,plain,target])values.push(native(current,key))
    assert.equal(typeof values[0].get,'function');assert.equal(values[1].value,7)
    assert.equal(typeof bound(key).get,'function')
    const report=p.descriptorAccess(),identity=p.nativeIdentity()
    assert.equal(report.available,true,JSON.stringify(report));assert.equal(identity.available,true,JSON.stringify(identity))
    const entries=report.entries.filter(e=>e.entries.includes('Object.getOwnPropertyDescriptor'))
    assert.equal(entries.reduce((n,e)=>n+e.count,0),4)
    assert(entries.some(e=>e.argument.available&&e.argument.target.watchedAccessorKeys.includes('tmp')))
    assert(entries.some(e=>e.argument.available&&e.argument.target.watchedAccessorKeys.length===0),
      'a new invocation at the same source location must resolve its current target')
    assert(entries.some(e=>!e.argument.available&&['native callee identity differs','native heap identity unavailable'].includes(e.argument.reason)))
    assert.equal(identity.mode,'heap-object-id');assert.equal(identity.snapshotsRequested,0);assert.equal(identity.replayEligible,false)
  }finally{p.close()}
})
