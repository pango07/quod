// Diagnostic only. Pinned V8 evaluates native entry conditions with the
// actual arguments, and reports caught condition exceptions to Inspector.
// No application globals or public callbacks are installed. This is not an
// input-ownership certificate and must never authorize a replay by itself.
import {randomBytes} from 'node:crypto'
import {getCallSites} from 'node:util'
import {watchedAccessorInspector} from './watched-accessor-inspector.mjs'
import {nativeCaptureInspector} from './native-capture-inspector.mjs'
const hasOwn=Object.hasOwn,isSafeInteger=Number.isSafeInteger
const own=(object,key)=>object&&hasOwn(object,key)?object[key]:undefined

export function observeNativeEntryArguments(post,{onEntry=()=>{},handleGroups=false,targetAccessorsOnly=false,directArguments=false,targetInspection='protocol',nativeCapture=false}={}) {
  if(!['protocol','watched','adaptive'].includes(targetInspection))throw Error('invalid target inspection mode')
  if(nativeCapture&&(!directArguments||handleGroups||targetInspection!=='protocol'))throw Error('native capture requires direct arguments and its own inspection mode')
  const nonce=randomBytes(32).toString('hex'),entries=new Map()
  const scoped=targetInspection==='protocol'?null:watchedAccessorInspector(post,nonce),wideSites=new Set()
  const nativeInspector=nativeCapture?nativeCaptureInspector(post,nonce):null
  const errors=[]
  let calls=0,ignored=0,cleared=0,released=0,groupsReleased=0,events=0,closed=false,scopedInspections=0
  const supported=process.versions.node==='24.18.0'
  function fail(reason) {if(errors.length<20)errors.push(reason)}
  function note(entry,frames) {
    onEntry({...entry,frames,replayEligible:false})
  }
  return {
    condition(api,{receiver=false,describeOwnAccessors=false}={}) {
      if(!supported||closed)throw Error('native entry conditions require the pinned active diagnostic')
      if(typeof api!=='string'||entries.has(api))throw Error('invalid native entry registration')
      scoped?.initialize()
      nativeInspector?.initialize()
      entries.set(api,{receiver,describeOwnAccessors})
      if(nativeCapture)return `(()=>{throw {__proto__:null,quodNativeEntry:${JSON.stringify(nonce)},api:${JSON.stringify(api)},argument:${receiver?'this':'arguments.length?arguments[0]:void 0'},argumentCount:arguments.length,receiverBased:${JSON.stringify(receiver)},describeOwnAccessors:${JSON.stringify(describeOwnAccessors)}})()`
      if(directArguments)return `(()=>{throw {__proto__:null,quodNativeEntry:${JSON.stringify(nonce)},api:${JSON.stringify(api)},argument:${receiver?'this':'arguments.length?arguments[0]:void 0'},argumentCount:arguments.length}})()`
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
        for(const p of [...own(result,'result')??[],...own(result,'internalProperties')??[],...own(result,'privateProperties')??[]]) {
          for(const v of [own(p,'value'),own(p,'get'),own(p,'set')])if(v?.objectId)handles.add(v.objectId)
        }
        return result.result
      }
      function data(fields,name) {
        const p=fields.find(p=>p.name===name),value=own(p,'value')
        if(!p||own(p,'get')||own(p,'set')||!value||own(p,'isOwn')!==true)throw Error('entry field is not own data')
        return value
      }
      try {
        let remote=params.exceptionDetails?.exception
        if(!remote?.objectId||remote.subtype==='proxy'){ignored++;return false}
        if(nativeInspector) {
          handles.add(remote.objectId)
          const value=nativeInspector.capture(remote.objectId)
          if(!value.owned) {
            if(!value.available)throw Error('native exception inspection unavailable')
            ignored++;return false
          }
          owned=true;calls++
          if(value.cleared)cleared++
          const configuration=entries.get(value.api)
          if(!value.available||!value.cleared||!configuration||value.receiverBased!==configuration.receiver||
            value.describeOwnAccessors!==configuration.describeOwnAccessors||!isSafeInteger(value.argumentCount)||value.argumentCount<0||
            value.inspected!==(value.kind==='object'&&configuration.describeOwnAccessors))throw Error('native capture summary unavailable')
          if(value.inspected)scopedInspections++
          note({api:value.api,argumentCount:value.argumentCount,receiverBased:value.receiverBased,kind:value.kind,
            ...(value.target?{target:value.target}:{})},getCallSites(100,{sourceMap:false}).map(s=>({name:s.functionName,scriptId:s.scriptId,
              scriptName:s.scriptName,line:s.lineNumber,column:s.columnNumber})))
          return true
        }
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
        if(own(own(fields.find(p=>p.name==='quodNativeEntry'),'value'),'value')!==nonce){ignored++;return false}
        owned=true;payload=remote.objectId;calls++
        const api=data(fields,'api').value,configuration=entries.get(api)
        if(!configuration)throw Error('unregistered native entry')
        for(const name of ['quodNativeEntry','api',...(directArguments?['argument','argumentCount']:['args','receiver'])]) {
          const p=fields.find(p=>p.name===name)
          if(!p||own(p,'get')||own(p,'set')||own(p,'isOwn')!==true||own(p,'writable')!==true)throw Error('entry payload cannot be cleared safely')
        }
        canClear=true
        const args=directArguments?null:properties(data(fields,'args').objectId)
        const argumentCount=directArguments?data(fields,'argumentCount').value:data(args,'length').value
        if(!isSafeInteger(argumentCount)||argumentCount<0)throw Error('native argument count unavailable')
        const argument=directArguments?data(fields,'argument'):configuration.receiver?data(fields,'receiver'):
          argumentCount?data(args,'0'):{type:'undefined'}
        const kind=argument.subtype==='proxy'?'proxy':argument.objectId?'object':'primitive'
        // Capture once, at the same handler depth as the protocol path. The
        // adaptive key only selects an inspection implementation; collisions
        // cannot authorize reuse or omit the watched descriptor checks.
        const frames=getCallSites(100,{sourceMap:false}).map(s=>({name:s.functionName,scriptId:s.scriptId,
          scriptName:s.scriptName,line:s.lineNumber,column:s.columnNumber}))
        let target
        if(configuration.describeOwnAccessors&&kind==='object') {
          const site=targetInspection==='adaptive'&&frames[5]?JSON.stringify([api,frames[5]]):null
          if(targetInspection==='watched'||site&&wideSites.has(site)) {
            target=scoped.inspect(argument.objectId);scopedInspections++
          }else {
            const accessors=properties(argument.objectId,{accessorsOnly:targetAccessorsOnly})
              .filter(p=>own(p,'isOwn')===true&&(own(p,'get')?.type==='function'||own(p,'set')?.type==='function'))
            target={inspectionScope:'all-own-accessors',ownAccessorCount:accessors.length,
              watchedAccessorKeys:accessors.map(p=>p.name).filter(k=>['seed','tmp','invalidate'].includes(k))}
            // Capacity only bounds this optional performance hint. Unknown
            // sites keep the complete protocol path, with no dropped calls.
            if(site&&accessors.length>=64&&wideSites.size<4096)wideSites.add(site)
          }
        }
        // Native call-site records avoid constructing every lexical scope.
        // Keep all bounded frames for now; no filename grants provenance.
        note({api,argumentCount,receiverBased:configuration.receiver,kind,...(target?{target}:{})},frames)
        return true
      }catch {fail('native entry capture failed');return owned}
      finally {
        if(owned&&payload&&canClear)try {
          // V8's console storage retains condition exceptions. Clear only
          // our fresh wrapper, never the synthetic arguments or target, so
          // stored messages do not keep application values alive.
          const result=post('Runtime.callFunctionOn',{objectId:payload,
            functionDeclaration:directArguments?'function(){this.argument=null;this.quodNativeEntry=null;this.api=null}':
              'function(){this.args=null;this.receiver=null;this.quodNativeEntry=null;this.api=null}',silent:true,returnByValue:true})
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
      handleMode:nativeCapture?'native-private-capture':handleGroups?'private-event-group':'individual',argumentMode:directArguments?'direct':'array',
      targetAccessorsOnly,targetInspection:nativeCapture?'native-watched':targetInspection,nativeCapture,scopedInspections,adaptiveSites:wideSites.size,errors:[...errors],
      replayEligible:false,limitation:'Diagnostic exception transport is visible to other Inspector sessions; capability exclusion and complete native coverage remain unproven'}),
    close(){if(closed)return;closed=true;try{scoped?.close();nativeInspector?.close()}catch{fail('private inspector cleanup failed')}},
  }
}
