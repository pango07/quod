// Diagnostic bridge only. The live capture function exists behind a private
// Inspector handle. Its native implementation returns no application values.
import path from 'node:path'
const integer=Number.isInteger
const failureCodes=new WeakMap(),getFailure=Function.prototype.call.bind(WeakMap.prototype.get),
  setFailure=Function.prototype.call.bind(WeakMap.prototype.set)
function failure(code){const error=new Error(code);setFailure(failureCodes,error,code);return error}

export function nativeCaptureInspector(post,nonce) {
  const addon=process.env.QUOD_NATIVE_CAPTURE_ADDON,group='quod-native-capture-'+nonce
  let handle=null,closed=false
  return {
    initialize() {
      if(closed)throw Error('native capture closed')
      if(handle)return
      if(process.versions.node!=='24.18.0'||!addon||!path.isAbsolute(addon))throw Error('native capture requires the pinned remote-built addon')
      const expression=`process.getBuiltinModule('module').createRequire(${JSON.stringify(import.meta.url)})(${JSON.stringify(addon)}).createCapture(${JSON.stringify(nonce)})`
      const r=post('Runtime.evaluate',{expression,objectGroup:group,silent:true,generatePreview:false})
      if(r.exceptionDetails||r.result?.type!=='function'||!r.result.objectId)throw Error('native capture setup failed')
      handle=r.result.objectId
    },
    capture(objectId) {
      if(closed||!handle)throw failure('native capture unavailable')
      let r
      try {r=post('Runtime.callFunctionOn',{objectId:handle,functionDeclaration:'function(payload){return this(payload)}',
        arguments:[{objectId}],returnByValue:true,silent:true,generatePreview:false})}
      catch {throw failure('native capture protocol request failed')}
      if(r.exceptionDetails)throw failure('native capture evaluation failed')
      const value=r.result?.value
      if(!value||typeof value.owned!=='boolean'||typeof value.available!=='boolean'||typeof value.cleared!=='boolean')throw failure('invalid native capture result')
      if(!integer(value.rejection)||value.rejection<0||value.rejection>5)throw failure('invalid native rejection reason')
      if(value.owned&&value.available) {
        if(!integer(value.kind)||value.kind<0||value.kind>2||!integer(value.mask)||value.mask<0||value.mask>7||
          typeof value.inspected!=='boolean')throw failure('invalid native argument summary')
        value.kind=['primitive','object','proxy'][value.kind]
        if(value.inspected) {
          const keys=['seed','tmp','invalidate'].filter((_,index)=>value.mask&(1<<index))
          value.target={inspectionScope:'watched-properties',callbackPropertyCheck:'v8-own-callback',
            watchedAccessorCount:keys.length,watchedAccessorKeys:keys}
        }
      }
      return value
    },
    failureReason(error){return getFailure(failureCodes,error)??null},
    close(){if(closed)return;closed=true;if(handle)post('Runtime.releaseObjectGroup',{objectGroup:group})},
  }
}
