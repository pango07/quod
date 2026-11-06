// Diagnostic only: read Node's observer registries through Inspector.
// Never modifies a registry, exposes a reference, or authorizes replay.
// Private runtime shapes are not a supported product contract.
import {Session} from 'node:inspector'

export function inspectPromiseObservers() {
  const session=new Session()
  let failure=null, closed=false
  function post(method,params={}) {
    let done=false,result,error
    session.post(method,params,(e,r)=>{done=true;error=e;result=r})
    if (!done || error) throw Error(!done ? 'asynchronous inspector response' : 'inspector request failed')
    return result
  }
  function properties(id) {
    if (!id) throw Error('missing runtime object')
    return post('Runtime.getProperties',{objectId:id,ownProperties:true,generatePreview:false})
  }
  function prop(id,name) {return properties(id).result.find(p=>p.name===name)?.value}
  function builtin(expression) {
    const r=post('Runtime.evaluate',{expression,objectGroup:'quod-observer-diagnostic',silent:true})
    if (r.exceptionDetails) throw Error('builtin unavailable')
    return r.result.objectId
  }
  function binding(fn,name) {
    const scopes=properties(fn).internalProperties?.find(p=>p.name==='[[Scopes]]')?.value?.objectId
    if (!scopes) throw Error('function scopes unavailable')
    for(const p of properties(scopes).result) {
      if (!/^\d+$/.test(p.name) || p.value?.description==='Global') continue
      const value=prop(p.value?.objectId,name)
      if(value) return value
    }
    throw Error('runtime binding unavailable: '+name)
  }
  const kinds=['init','before','after','settled']
  let lists={}, fields=null, indices={}, active=null
  try {
    session.connect()
    for(const kind of kinds) {
      const name='on'+kind[0].toUpperCase()+kind.slice(1)
      lists[kind]=binding(builtin(`process.getBuiltinModule('v8').promiseHooks.${name}`),'list').objectId
    }
    const publicCreate=builtin("process.getBuiltinModule('async_hooks').createHook")
    const getHooks=binding(publicCreate,'getHookArrays').objectId
    fields=binding(getHooks,'async_hook_fields').objectId
    active=binding(getHooks,'active_hooks').objectId
    for(const name of ['kInit','kBefore','kAfter','kDestroy','kPromiseResolve','kTotals']) {
      const index=binding(getHooks,name).value
      if(!Number.isInteger(index)) throw Error('unknown hook field index')
      indices[name]=index
    }
  } catch(e) {failure=e.message}
  return {
    snapshot() {
      if(failure || closed) return {available:false,reason:failure??'closed',replayEligible:false}
      try {
        const promiseHooks={}
        for(const kind of kinds) {
          const length=prop(lists[kind],'length')?.value
          if(!Number.isInteger(length) || length<0) throw Error('unknown promise registry shape')
          promiseHooks[kind]=length
        }
        const values=properties(fields).result, asyncHooks={}
        for(const [name,index] of Object.entries(indices)) {
          const value=values.find(p=>p.name===String(index))?.value?.value
          if(!Number.isInteger(value) || value<0) throw Error('unknown async registry shape')
          asyncHooks[name]=value
        }
        const pending=prop(active,'tmp_array')
        return {available:true,promiseHooks,asyncHooks,pendingAsyncHookEdit:pending?.subtype!=='null',replayEligible:false}
      } catch(e) {failure=e.message;return {available:false,reason:failure,replayEligible:false}}
    },
    close() {
      closed=true
      try {post('Runtime.releaseObjectGroup',{objectGroup:'quod-observer-diagnostic'})} catch {}
      try {session.disconnect()} catch {}
    },
  }
}
