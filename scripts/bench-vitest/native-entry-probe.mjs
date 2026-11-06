// Diagnostic only. Pinned V8 evaluates native entry conditions with the
// actual arguments, and reports caught condition exceptions to Inspector.
// No application globals or public callbacks are installed. This is not an
// input-ownership certificate and must never authorize a replay by itself.
import {randomBytes} from 'node:crypto'
import {getCallSites} from 'node:util'

export function observeNativeEntryArguments(post,{onEntry=()=>{},handleGroups=false,targetAccessorsOnly=false}={}) {
  const nonce=randomBytes(32).toString('hex'),entries=new Map()
  const errors=[]
  let calls=0,ignored=0,cleared=0,released=0,groupsReleased=0,events=0,closed=false
  const supported=process.versions.node==='24.18.0'
  function fail(reason) {if(errors.length<20)errors.push(reason)}
  return {
    condition(api,{receiver=false,describeOwnAccessors=false}={}) {
      if(!supported||closed)throw Error('native entry conditions require the pinned active diagnostic')
      if(typeof api!=='string'||entries.has(api))throw Error('invalid native entry registration')
      entries.set(api,{receiver,describeOwnAccessors})
      return `(()=>{throw {__proto__:null,quodNativeEntry:${JSON.stringify(nonce)},api:${JSON.stringify(api)},args:arguments,receiver:this}})()`
    },
    handleException(params) {
      if(!supported||closed)return false
      const handles=new Set()
      let owned=false,payload=null,canClear=false,group=null,original=null
      function properties(id,{accessorsOnly=false}={}) {
        if(!id)throw Error('entry object unavailable')
        handles.add(id)
        const result=post('Runtime.getProperties',{objectId:id,ownProperties:true,generatePreview:false,
          ...(accessorsOnly?{accessorPropertiesOnly:true}:{})})
        if(result.exceptionDetails)throw Error('entry descriptors unavailable')
        for(const p of [...result.result??[],...result.internalProperties??[],...result.privateProperties??[]]) {
          for(const v of [p.value,p.get,p.set])if(v?.objectId)handles.add(v.objectId)
        }
        return result.result
      }
      function data(fields,name) {
        const p=fields.find(p=>p.name===name)
        if(!p||p.get||p.set||!p.value||p.isOwn!==true)throw Error('entry field is not own data')
        return p.value
      }
      try {
        let remote=params.exceptionDetails?.exception
        if(!remote?.objectId||remote.subtype==='proxy'){ignored++;return false}
        if(handleGroups) {
          // getProperties (including internal/private properties) inherits
          // the inspected handle's object group on the pinned V8 runtime.
          // Give this event a private group without reading any properties;
          // one release then drops every temporary handle, including values
          // discovered recursively. Never clear the shared console group.
          original=remote.objectId
          if(events===Number.MAX_SAFE_INTEGER)throw Error('native event capacity exceeded')
          group='quod-native-entry-'+nonce+'-'+(++events)
          const copied=post('Runtime.callFunctionOn',{objectId:original,objectGroup:group,
            functionDeclaration:'function(){return this}',silent:true,generatePreview:false})
          if(copied.exceptionDetails||!copied.result?.objectId)throw Error('native event group unavailable')
          remote=copied.result
        }
        const fields=properties(remote.objectId)
        if(fields.find(p=>p.name==='quodNativeEntry')?.value?.value!==nonce){ignored++;return false}
        owned=true;payload=remote.objectId;calls++
        const api=data(fields,'api').value,configuration=entries.get(api)
        if(!configuration)throw Error('unregistered native entry')
        for(const name of ['quodNativeEntry','api','args','receiver']) {
          const p=fields.find(p=>p.name===name)
          if(!p||p.get||p.set||p.isOwn!==true||p.writable!==true)throw Error('entry payload cannot be cleared safely')
        }
        canClear=true
        const args=properties(data(fields,'args').objectId),argumentCount=data(args,'length').value
        if(!Number.isSafeInteger(argumentCount)||argumentCount<0)throw Error('native argument count unavailable')
        const argument=configuration.receiver?data(fields,'receiver'):
          argumentCount?data(args,'0'):{type:'undefined'}
        const kind=argument.subtype==='proxy'?'proxy':argument.objectId?'object':'primitive'
        let target
        if(configuration.describeOwnAccessors&&kind==='object') {
          const accessors=properties(argument.objectId,{accessorsOnly:targetAccessorsOnly})
            .filter(p=>p.isOwn===true&&(p.get?.type==='function'||p.set?.type==='function'))
          target={ownAccessorCount:accessors.length,
            watchedAccessorKeys:accessors.map(p=>p.name).filter(k=>['seed','tmp','invalidate'].includes(k))}
        }
        // Native call-site records avoid constructing every lexical scope.
        // Keep all bounded frames for now; no filename grants provenance.
        const frames=getCallSites(100,{sourceMap:false}).map(s=>({name:s.functionName,scriptId:s.scriptId,
          scriptName:s.scriptName,line:s.lineNumber,column:s.columnNumber}))
        onEntry({api,argumentCount,receiverBased:configuration.receiver,kind,...(target?{target}:{}),frames,replayEligible:false})
        return true
      }catch {fail('native entry capture failed');return owned}
      finally {
        if(owned&&payload&&canClear)try {
          // V8's console storage retains condition exceptions. Clear only
          // our fresh wrapper, never the synthetic arguments or target, so
          // stored messages do not keep application values alive.
          const result=post('Runtime.callFunctionOn',{objectId:payload,
            functionDeclaration:'function(){this.args=null;this.receiver=null;this.quodNativeEntry=null;this.api=null}',silent:true,returnByValue:true})
          if(result.exceptionDetails)throw Error('clear failed')
          cleared++
        }catch {fail('native entry payload cleanup failed')}
        if(group)try {post('Runtime.releaseObjectGroup',{objectGroup:group});groupsReleased++;released+=handles.size}
        catch {fail('native entry group cleanup failed')}
        else for(const objectId of handles)try {post('Runtime.releaseObject',{objectId});released++}
        catch {fail('native entry handle cleanup failed')}
        if(original)try {post('Runtime.releaseObject',{objectId:original});released++}
        catch {fail('native entry original handle cleanup failed')}
      }
    },
    report:()=>({available:supported&&!closed&&!errors.length,calls,ignored,cleared,released,groupsReleased,
      handleMode:handleGroups?'private-event-group':'individual',targetAccessorsOnly,errors:[...errors],
      replayEligible:false,limitation:'Diagnostic exception transport is visible to other Inspector sessions; capability exclusion and complete native coverage remain unproven'}),
    close(){closed=true},
  }
}
