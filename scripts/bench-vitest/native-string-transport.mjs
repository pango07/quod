// Isolated diagnostic only. A realm-local lexical binding calls the existing
// native validator inside the condition. Only a sanitized primitive reaches
// Inspector; no per-entry object handles or protocol requests are needed.
// The new lexical bindings are observable through debugger capabilities and
// survive close until the realm dies. This is not an ownership certificate.
import path from 'node:path'
import {randomBytes} from 'node:crypto'
import {decodeNativeCaptureSummary} from './native-capture-summary.mjs'
const stringify=JSON.stringify,parse=JSON.parse

export function nativeStringTransport(post,nonce,{onSummary}={}) {
  if(!/^[a-f0-9]{64}$/.test(nonce)||typeof onSummary!=='function')throw Error('invalid primitive transport setup')
  const name='_quod_native_capture_'+nonce,group='quod-native-string-'+nonce,prefix=nonce+':'
  const contexts=new Map(),attempted=new Set(),entries=new Map()
  let handle=null,closed=false,processing=false,notifications=0,delivered=0,ignored=0,failures=0,failure=null
  let contextQueries=0,bindings=0
  function fail(reason){failures++;failure??=reason}
  function evaluate(expression,contextId) {
    const result=post('Runtime.evaluate',{expression,...(contextId===undefined?{}:{contextId}),
      objectGroup:group,silent:true,generatePreview:false})
    if(result.exceptionDetails)throw Error('primitive transport evaluation failed')
    return result.result
  }
  return {
    initialize() {
      if(closed||handle)throw Error('primitive transport initialization state')
      const addon=process.env.QUOD_NATIVE_CAPTURE_ADDON
      if(process.versions.node!=='24.18.0'||!addon||!path.isAbsolute(addon))throw Error('primitive transport requires pinned native helper')
      const expression=`const ${name}=process.getBuiltinModule('module').createRequire(${stringify(import.meta.url)})(${stringify(addon)}).createStringCapture(${stringify(nonce)},true);${name}`
      const value=evaluate(expression)
      if(value?.type!=='function'||!value.objectId)throw Error('primitive transport root helper unavailable')
      handle=value.objectId;bindings++
    },
    contextCreated(context) {
      if(closed)return
      try {
        if(!handle||!Number.isInteger(context?.id)||context.id<=0||typeof context.auxData?.isDefault!=='boolean')
          throw Error('primitive transport context unavailable')
        if(attempted.has(context.id))return
        if(attempted.size>=32)throw Error('primitive transport context capacity exceeded')
        attempted.add(context.id)
        if(context.auxData.isDefault) {
          if([...contexts.values()].includes('default'))throw Error('duplicate default context')
          contexts.set(context.id,'default');return
        }
        const key=randomBytes(32).toString('hex'),holderName='_quod_native_holder_'+key
        const holder=evaluate(`const ${holderName}=({__proto__:null,${stringify(key)}:true,capture:void 0});${holderName}`,context.id)
        bindings++
        if(!holder?.objectId)throw Error('primitive transport holder unavailable')
        try {
          contextQueries++
          const installed=post('Runtime.callFunctionOn',{objectId:handle,
            functionDeclaration:'function(nonce,key){return this.createContextCapture(nonce,key,true)}',
            arguments:[{value:nonce},{value:key}],returnByValue:true,silent:true,generatePreview:false})
          if(installed.exceptionDetails||installed.result?.value!==true)throw Error('primitive transport context helper unavailable')
          const value=evaluate(`const ${name}=${holderName}.capture;${holderName}.capture=null;typeof ${name}`,context.id)
          bindings++
          if(value?.value!=='function')throw Error('primitive transport context binding unavailable')
          contexts.set(context.id,'foreign')
        }finally {post('Runtime.releaseObject',{objectId:holder.objectId})}
      }catch{fail('primitive transport context installation failed')}
    },
    condition(api,{receiver=false,describeOwnAccessors=true}={}) {
      if(closed||!handle||typeof api!=='string'||entries.has(api)||typeof receiver!=='boolean'||typeof describeOwnAccessors!=='boolean')
        throw Error('invalid primitive entry registration')
      entries.set(api,{receiver,describeOwnAccessors})
      return `(()=>{throw ${name}({__proto__:null,quodNativeEntry:${stringify(nonce)},api:${stringify(api)},argument:${receiver?'this':'arguments.length?arguments[0]:void(0)'},argumentCount:arguments.length,receiverBased:${receiver},describeOwnAccessors:${describeOwnAccessors}})})()`
    },
    handleException(params) {
      if(closed)return false
      const remote=params.exceptionDetails?.exception
      if(remote?.type!=='string'||typeof remote.value!=='string'||!remote.value.startsWith(prefix)){ignored++;return false}
      notifications++
      if(processing){fail('reentrant primitive notification');return true}
      processing=true
      try {
        if(remote.objectId||!contexts.has(params.exceptionDetails.executionContextId))throw Error('primitive context unknown')
        const packed=parse(remote.value.slice(prefix.length)),value=decodeNativeCaptureSummary(packed)
        const config=entries.get(value.api)
        if(!value.owned||!value.available||!value.cleared||!value.metadataValid||value.failure!==0||!config||
          value.receiverBased!==config.receiver||value.describeOwnAccessors!==config.describeOwnAccessors)
          throw Error('primitive summary unavailable')
        onSummary(packed);delivered++
      }catch{fail('primitive summary callback or validation failed')}
      finally {processing=false}
      return true
    },
    report:()=>({transport:'primitive-string',enabled:handle!==null,closed,notifications,delivered,ignored,failures,failure,
      contexts:contexts.size,attemptedContexts:attempted.size,contextQueries,persistentLexicalBindings:bindings,
      perEntryProtocolRequests:0,perEntryObjectHandles:0,replayEligible:false,
      limitation:'Realm lexical bindings remain observable through debugger capabilities and persist until realm destruction; initial ownership and complete coverage remain unproven'}),
    close() {
      if(closed)return
      if(processing){fail('close requested during primitive notification');return}
      closed=true
      if(handle)post('Runtime.releaseObjectGroup',{objectGroup:group})
    },
  }
}
