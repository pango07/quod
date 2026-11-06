import test from 'node:test'
import assert from 'node:assert/strict'
import {Session} from 'node:inspector'
import {observeNativeCallArguments} from './native-call-argument.mjs'

test('pinned Inspector releases argument descriptor handles on resume while retaining the watched intrinsic',()=>{
  assert.equal(process.versions.node,'24.18.0','review the pinned Inspector lifetime before changing Node')
  const session=new Session(),handles=new Set(),reports=[],errors=[]
  let individualReleases=0
  function post(method,params={}) {
    let done=false,error,result
    session.post(method,params,(e,r)=>{done=true;error=e;result=r})
    if(!done||error)throw error??Error('asynchronous inspector response')
    return result
  }
  const observe=observeNativeCallArguments((method,params)=>{
    const r=post(method,params)
    if(method==='Runtime.releaseObject')individualReleases++
    if(method==='Runtime.getProperties')for(const p of [...r.result??[],...r.internalProperties??[],...r.privateProperties??[]]) {
      for(const v of [p.value,p.get,p.set])if(v?.objectId)handles.add(v.objectId)
    }
    return r
  })
  const native=Object.getOwnPropertyDescriptor,key='tmp'
  let gets=0
  const target={get tmp(){gets++;return 'not-read'}},other={tmp:7}
  session.connect()
  try {
    post('Debugger.enable')
    const watched=post('Runtime.evaluate',{expression:'Object.getOwnPropertyDescriptor',objectGroup:'quod-lifetime-fixture'}).result.objectId
    const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:watched})
    session.on('Debugger.paused',({params})=>{
      try{reports.push(observe(params.callFrames[0],watched,{pureAdditionalArguments:true,describeOwnAccessors:true}))}
      catch(e){errors.push(e.message)}
      finally{post('Debugger.resume')}
    })
    const results=[]
    for(const current of [target,other,target])results.push(native(current,key))
    assert.equal(typeof results[0].get,'function');assert.equal(results[1].value,7)
    post('Debugger.removeBreakpoint',{breakpointId})
    assert.deepEqual(errors,[]);assert.equal(reports.length,3)
    assert(reports.every(r=>r.available),JSON.stringify(reports))
    assert.deepEqual(reports.map(r=>r.target.watchedAccessorKeys),[['tmp'],[],['tmp']],
      'a repeated call location must resolve each current target instead of caching a previous argument')
    assert.equal(gets,0);assert.equal(individualReleases,0)
    assert(handles.size>5,'exercise scopes, target objects and getter handles')
    for(const objectId of handles)assert.throws(()=>post('Runtime.getProperties',{objectId,ownProperties:true}),
      /Could not find object with given id/,'resuming must release every temporary argument descriptor handle')
    assert(Array.isArray(post('Runtime.getProperties',{objectId:watched,ownProperties:true}).result),
      'the long-lived watched intrinsic must remain usable')
    post('Runtime.releaseObjectGroup',{objectGroup:'quod-lifetime-fixture'})
  }finally{session.disconnect()}
})
