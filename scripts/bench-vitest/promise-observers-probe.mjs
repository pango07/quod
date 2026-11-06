// Diagnostic only: read Node's observer registries through Inspector.
// Never modifies a registry, exposes a reference, or authorizes replay.
// Private runtime shapes are not a supported product contract.
import {Session} from 'node:inspector'
import {createHash} from 'node:crypto'

export function inspectPromiseObservers() {
  const session=new Session()
  const scripts=new Map()
  session.on('Debugger.scriptParsed',({params})=>scripts.set(params.scriptId,{
    url:params.url,isModule:params.isModule===true,hasSourceURL:params.hasSourceURL===true}))
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
  function same(a,b) {
    if(!a || !b) return false
    const r=post('Runtime.callFunctionOn',{objectId:a,
      functionDeclaration:'function(other){return this===other}',
      arguments:[{objectId:b}],returnByValue:true,silent:true})
    return !r.exceptionDetails && r.result?.value===true
  }
  function functionSource(fn) {
    const location=properties(fn).internalProperties?.find(p=>p.name==='[[FunctionLocation]]')?.value?.value
    if(!location?.scriptId) return {location:null}
    const source=post('Debugger.getScriptSource',{scriptId:location.scriptId}).scriptSource
    if(typeof source!=='string') throw Error('function source unavailable')
    return {location,script:scripts.get(location.scriptId)??null,
      sourceSha256:createHash('sha256').update(source).digest('hex')}
  }
  const kinds=['init','before','after','settled']
  let lists={}, fields=null, indices={}, active=null, owned=null, nativeRefs=null
  try {
    session.connect()
    post('Debugger.enable')
    for(const kind of kinds) {
      const name='on'+kind[0].toUpperCase()+kind.slice(1)
      lists[kind]=binding(builtin(`process.getBuiltinModule('v8').promiseHooks.${name}`),'list').objectId
    }
    const publicCreate=builtin("process.getBuiltinModule('async_hooks').createHook")
    const getHooks=binding(publicCreate,'getHookArrays').objectId
    fields=binding(getHooks,'async_hook_fields').objectId
    active=binding(getHooks,'active_hooks').objectId
    for(const name of ['kInit','kBefore','kAfter','kDestroy','kPromiseResolve','kTotals']) {
      // kTotals is used by the public AsyncHook enable/disable methods,
      // and V8 may omit it from the internal function's lexical context.
      // Read the actual binding; never assume a numeric field offset.
      const index=binding(publicCreate,name).value
      if(!Number.isInteger(index)) throw Error('unknown hook field index')
      indices[name]=index
    }
  } catch(e) {failure=e.message}
  return {
    nativeStatProvenance() {
      // Capture before wrapping fs. Do not equate matching names/source
      // with native provenance: these are diagnostic facts for review.
      try {
        const stat=builtin("process.getBuiltinModule('fs').promises.stat")
        const convert=binding(stat,'getStatsFromBinding').objectId
        const ctor=binding(convert,'Stats').objectId
        const native=binding(stat,'binding').objectId
        const nativeStat=prop(native,'stat')
        const publicCtor=builtin("process.getBuiltinModule('fs').Stats")
        const prototype=prop(ctor,'prototype')?.objectId
        if(!prototype || nativeStat?.type!=='function') throw Error('native stat shape unavailable')
        nativeRefs={stat,convert,ctor,native,fn:nativeStat.objectId,prototype}
        return {available:true,stat:functionSource(stat),convert:functionSource(convert),constructor:functionSource(ctor),
          publicConstructor:functionSource(publicCtor),publicConstructorMatches:same(ctor,publicCtor),
          publicPrototypeMatches:same(prop(ctor,'prototype')?.objectId,prop(publicCtor,'prototype')?.objectId),
          bindingStat:functionSource(nativeStat?.objectId),
          bindingStatDescriptionSha256:createHash('sha256').update(nativeStat?.description??'').digest('hex'),
          replayEligible:false}
      } catch(e) {return {available:false,reason:e.message,replayEligible:false}}
    },
    nativeStatSnapshot() {
      if(failure || closed || !nativeRefs) return {available:false,unchanged:false,replayEligible:false}
      try {
        const n=nativeRefs
        // The public promise function can remain unchanged while its
        // closed-over binding's stat method is replaced. Compare actual
        // identities; names and native-looking text are insufficient.
        const unchanged=same(binding(n.stat,'getStatsFromBinding').objectId,n.convert) &&
          same(binding(n.convert,'Stats').objectId,n.ctor) &&
          same(binding(n.stat,'binding').objectId,n.native) &&
          same(prop(n.native,'stat')?.objectId,n.fn) &&
          same(prop(n.ctor,'prototype')?.objectId,n.prototype)
        return {available:true,unchanged,replayEligible:false}
      } catch(e) {return {available:false,unchanged:false,reason:e.message,replayEligible:false}}
    },
    promiseOutcome(kind) {
      // The owned hook keeps this short-lived reference in its private
      // lexical context. No global bridge, replacement Stats or promise
      // reaction is needed to inspect the native result before its await
      // consumer starts. Merely resolving a promise is too early: the
      // settled hook may still see a pending internal state.
      if(failure || closed || !owned || !nativeRefs) return {available:false,replayEligible:false}
      try {
        const context=binding(owned[kind],'checkpoint').objectId
        const promise=prop(context,'promise')?.objectId
        if(!promise) throw Error('owned promise reference unavailable')
        const internal=properties(promise).internalProperties
        const state=internal?.find(p=>p.name==='[[PromiseState]]')?.value?.value
        if(state==='rejected') return {available:true,state,projection:'ignored-error',replayEligible:false}
        if(state!=='fulfilled') return {available:false,state:state??'unknown',replayEligible:false}
        const result=internal.find(p=>p.name==='[[PromiseResult]]')?.value
        if(result?.type!=='object' || !result.objectId) throw Error('non-object stat result')
        const p=properties(result.objectId)
        const prototype=p.internalProperties?.find(p=>p.name==='[[Prototype]]')?.value?.objectId
        const size=p.result.find(p=>p.name==='size')
        if(!same(prototype,nativeRefs.prototype) || size?.get || size?.set || size?.value?.type!=='number' ||
          !Number.isSafeInteger(size.value.value) || size.value.value<0) throw Error('unknown stat result shape')
        return {available:true,state,projection:'size',size:size.value.value,replayEligible:false}
      } catch(e) {return {available:false,reason:e.message,replayEligible:false}}
    },
    captureOwned() {
      if(failure || closed || owned) return false
      try {
        const refs={}
        for(const kind of kinds) {
          if(prop(lists[kind],'length')?.value!==1) return false
          const fn=prop(lists[kind],'0')
          if(fn?.type!=='function' || !fn.objectId) return false
          refs[kind]=fn.objectId
        }
        owned=refs
        return true
      } catch(e) {failure=e.message;return false}
    },
    snapshot() {
      if(failure || closed) return {available:false,reason:failure??'closed',replayEligible:false}
      try {
        const promiseHooks={}, identities={}
        for(const kind of kinds) {
          const length=prop(lists[kind],'length')?.value
          if(!Number.isInteger(length) || length<0) throw Error('unknown promise registry shape')
          promiseHooks[kind]=length
          if(owned) {
            const fn=length===1 ? prop(lists[kind],'0') : null
            identities[kind]=false
            if(fn?.type==='function' && fn.objectId) {
              identities[kind]=same(owned[kind],fn.objectId)
            }
          }
        }
        const values=properties(fields).result, asyncHooks={}
        for(const [name,index] of Object.entries(indices)) {
          const value=values.find(p=>p.name===String(index))?.value?.value
          if(!Number.isInteger(value) || value<0) throw Error('unknown async registry shape')
          asyncHooks[name]=value
        }
        const pending=prop(active,'tmp_array')
        return {available:true,promiseHooks,asyncHooks,pendingAsyncHookEdit:pending?.subtype!=='null',
          onlyOwnedPromiseHooks:owned ? kinds.every(k=>identities[k]) : null,replayEligible:false}
      } catch(e) {failure=e.message;return {available:false,reason:failure,replayEligible:false}}
    },
    close() {
      closed=true
      try {post('Runtime.releaseObjectGroup',{objectGroup:'quod-observer-diagnostic'})} catch {}
      try {session.disconnect()} catch {}
    },
  }
}
