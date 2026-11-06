// Diagnostic bridge only. The live capture function exists behind a private
// Inspector handle. Its native implementation returns no application values.
import path from 'node:path'
import {randomBytes} from 'node:crypto'
const integer=Number.isInteger
const failureCodes=new WeakMap(),getFailure=Function.prototype.call.bind(WeakMap.prototype.get),
  setFailure=Function.prototype.call.bind(WeakMap.prototype.set)
function failure(code){const error=new Error(code);setFailure(failureCodes,error,code);return error}

export function nativeCaptureInspector(post,nonce,{ordinaryOwners=false}={}) {
  const addon=process.env.QUOD_NATIVE_CAPTURE_ADDON,group='quod-native-capture-'+nonce
  const contexts=new Map(),attemptedContexts=new Set()
  let handle=null,closed=false,contextQueries=0,contextQueryMs=0
  function contextCapture(contextId) {
    if(!integer(contextId)||contextId<=0)throw failure('native capture context unavailable')
    if(contexts.has(contextId))return contexts.get(contextId)
    if(attemptedContexts.has(contextId))throw failure('native capture context previously unavailable')
    if(attemptedContexts.size>=32)throw failure('native capture context capacity exceeded')
    attemptedContexts.add(contextId)
    const key=randomBytes(32).toString('hex')
    const holder=post('Runtime.evaluate',{contextId,
      expression:`({__proto__:null,${JSON.stringify(key)}:true,capture:void 0})`,
      objectGroup:group,silent:true,generatePreview:false})
    if(holder.exceptionDetails||!holder.result?.objectId)throw failure('native capture context holder unavailable')
    try {
      const started=process.hrtime.bigint();contextQueries++
      let installed
      try {installed=post('Runtime.callFunctionOn',{objectId:handle,
        functionDeclaration:'function(nonce,key,ordinary){return this.createContextCapture(nonce,key,ordinary)}',
        arguments:[{value:nonce},{value:key},{value:ordinaryOwners}],returnByValue:true,silent:true,generatePreview:false})}
      finally {contextQueryMs+=Number(process.hrtime.bigint()-started)/1e6}
      if(installed.exceptionDetails||installed.result?.value!==true)throw failure('native capture context installation failed')
      const fn=post('Runtime.callFunctionOn',{objectId:holder.result.objectId,objectGroup:group,
        functionDeclaration:'function(){const fn=this.capture;this.capture=null;return fn}',silent:true,generatePreview:false})
      if(fn.exceptionDetails||fn.result?.type!=='function'||!fn.result.objectId)throw failure('native capture context function unavailable')
      contexts.set(contextId,fn.result.objectId)
      return fn.result.objectId
    }finally {post('Runtime.releaseObject',{objectId:holder.result.objectId})}
  }
  function invoke(functionId,objectId) {
    return post('Runtime.callFunctionOn',{objectId:functionId,functionDeclaration:'function(payload){return this(payload)}',
      arguments:[{objectId}],returnByValue:true,silent:true,generatePreview:false})
  }
  return {
    initialize() {
      if(closed)throw Error('native capture closed')
      if(handle)return
      if(process.versions.node!=='24.18.0'||!addon||!path.isAbsolute(addon))throw Error('native capture requires the pinned remote-built addon')
      const expression=`process.getBuiltinModule('module').createRequire(${JSON.stringify(import.meta.url)})(${JSON.stringify(addon)}).createCapture(${JSON.stringify(nonce)},${JSON.stringify(ordinaryOwners)})`
      const r=post('Runtime.evaluate',{expression,objectGroup:group,silent:true,generatePreview:false})
      if(r.exceptionDetails||r.result?.type!=='function'||!r.result.objectId)throw Error('native capture setup failed')
      handle=r.result.objectId
    },
    capture(objectId,contextId) {
      if(closed||!handle)throw failure('native capture unavailable')
      let r
      try {r=invoke(contexts.get(contextId)??handle,objectId)}
      catch {
        // Unknown context IDs and already-failed context handles remain
        // unavailable. A new context gets one private, bounded native bridge.
        if(!integer(contextId)||contextId<=0||contexts.has(contextId))throw failure('native capture protocol request failed')
        try {r=invoke(contextCapture(contextId),objectId)}
        catch(error){throw getFailure(failureCodes,error)?error:failure('native capture context protocol request failed')}
      }
      if(r.exceptionDetails)throw failure('native capture evaluation failed')
      const value=r.result?.value
      if(!value||typeof value.owned!=='boolean'||typeof value.available!=='boolean'||typeof value.cleared!=='boolean')throw failure('invalid native capture result')
      if(!integer(value.rejection)||value.rejection<0||value.rejection>5)throw failure('invalid native rejection reason')
      if(typeof value.metadataValid!=='boolean'||!integer(value.failure)||value.failure<0||value.failure>6)throw failure('invalid native failure classification')
      if(!integer(value.excludedOwner)||value.excludedOwner<0||value.excludedOwner>2||
        (value.excludedOwner&&!ordinaryOwners))throw failure('invalid native owner exclusion')
      value.failureReason=['none','invalid-wrapper','api-wrapper','interceptor','callback-lookup','native-exception','cleanup'][value.failure]
      if(value.owned&&value.metadataValid) {
        if(!integer(value.kind)||value.kind<0||value.kind>2||!integer(value.mask)||value.mask<0||value.mask>7||
          typeof value.inspected!=='boolean')throw failure('invalid native argument summary')
        value.kind=['primitive','object','proxy'][value.kind]
        if(value.excludedOwner) {
          if(!value.available||value.inspected||value.mask!==0||value.kind!=='object')throw failure('invalid excluded owner summary')
          value.target={inspectionScope:'excluded-owner-type',ownerPolicy:'native-ordinary',
            ownerType:value.excludedOwner===1?'api-wrapper':'interceptor',watchedAccessorKeys:[]}
        }
        if(value.available&&value.inspected) {
          const keys=['seed','tmp','invalidate'].filter((_,index)=>value.mask&(1<<index))
          value.target={inspectionScope:'watched-properties',callbackPropertyCheck:'v8-own-callback',
            watchedAccessorCount:keys.length,watchedAccessorKeys:keys}
        }
      }
      return value
    },
    failureReason(error){return getFailure(failureCodes,error)??null},
    report:()=>({contexts:contexts.size,attemptedContexts:attemptedContexts.size,contextQueries,contextQueryMs}),
    close(){if(closed)return;closed=true;contexts.clear();if(handle)post('Runtime.releaseObjectGroup',{objectGroup:group})},
  }
}
