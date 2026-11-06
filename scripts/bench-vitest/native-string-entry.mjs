// Optional worker diagnostic transport. It changes collection, not replay
// authority. In particular, realm lexical bindings remain observable.
import {randomBytes} from 'node:crypto'
import {getCallSites} from 'node:util'
import {nativeStringTransport} from './native-string-transport.mjs'
import {decodeNativeCaptureSummary} from './native-capture-summary.mjs'

export function observeNativeStringEntries(post,{onEntry,ordinaryOwners=false,targetAccessorsOnly=false}={}) {
  const errors=[],failureCounts=new Map(),pendingContexts=new Map()
  let initialized=false,closed=false,calls=0,cleared=0,scopedInspections=0
  const fail=reason=>{failureCounts.set(reason,(failureCounts.get(reason)??0)+1);if(errors.length<20)errors.push(reason)}
  const transport=nativeStringTransport(post,randomBytes(32).toString('hex'),{ordinaryOwners,onSummary(packed) {
    const value=decodeNativeCaptureSummary(packed)
    calls++;if(value.cleared)cleared++
    const kind=['primitive','object','proxy'][value.kind]
    const frames=getCallSites(100,{sourceMap:false}).map(s=>({name:s.functionName,scriptId:s.scriptId,
      scriptName:s.scriptName,line:s.lineNumber,column:s.columnNumber}))
    const entry={api:value.api,argumentCount:value.argumentCount,receiverBased:value.receiverBased,kind,frames,replayEligible:false}
    if(!value.available) {
      const reason='native capture unavailable: '+['none','invalid-wrapper','api-wrapper','interceptor','callback-lookup','native-exception','cleanup'][value.failure]
      fail(reason);onEntry({...entry,available:false,reason});return
    }
    if(value.failure!==0||value.inspected!==(kind==='object'&&value.describeOwnAccessors&&!value.excludedOwner))
      throw Error('native capture summary unavailable')
    if(value.excludedOwner) {
      if(!ordinaryOwners||value.inspected||value.mask!==0||kind!=='object')throw Error('invalid excluded native owner')
      entry.target={inspectionScope:'excluded-owner-type',ownerPolicy:'native-ordinary',
        ownerType:value.excludedOwner===1?'api-wrapper':'interceptor',watchedAccessorKeys:[]}
    }else if(value.inspected) {
      scopedInspections++
      const keys=['seed','tmp','invalidate'].filter((_,index)=>value.mask&(1<<index))
      entry.target={inspectionScope:'watched-properties',callbackPropertyCheck:'v8-own-callback',
        watchedAccessorCount:keys.length,watchedAccessorKeys:keys}
    }
    onEntry(entry)
  }})
  return {
    contextCreated(context) {
      if(closed)return
      if(initialized)transport.contextCreated(context)
      else if(pendingContexts.size<32)pendingContexts.set(context.id,context)
      else fail('primitive pending context capacity exceeded')
    },
    contextDestroyed(id) {pendingContexts.delete(id);transport.contextDestroyed(id)},
    contextsCleared() {pendingContexts.clear();transport.contextsCleared()},
    condition(api,options) {
      if(closed)throw Error('native entry observer closed')
      // Deliberately wait until the first watched entry is installed: the
      // enclosing observer must see the helper's dlopen during setup.
      if(!initialized) {
        transport.initialize();initialized=true
        for(const context of pendingContexts.values())transport.contextCreated(context)
        pendingContexts.clear()
      }
      return transport.condition(api,{describeOwnAccessors:false,...options})
    },
    handleException:params=>transport.handleException(params),
    report() {
      const contextCapture=transport.report(),reportedErrors=[...errors]
      if(contextCapture.failures&&!reportedErrors.length)reportedErrors.push(contextCapture.failure)
      return {available:process.versions.node==='24.18.0'&&!closed&&!reportedErrors.length,
        calls,cleared,released:0,groupsReleased:0,ignored:contextCapture.ignored,ignoredReasons:[0,0,0,0,0,0],
        failureCounts:Object.fromEntries(failureCounts),failureSamples:[],contextCapture,
        ownerPolicy:ordinaryOwners?'native-ordinary':'unrestricted',handleMode:'native-primitive-string',argumentMode:'direct',
        targetAccessorsOnly,targetInspection:'native-watched',nativeCapture:true,scopedInspections,adaptiveSites:0,errors:reportedErrors,
        replayEligible:false,limitation:contextCapture.limitation}
    },
    close(){if(closed)return;closed=true;try{transport.close()}catch{fail('primitive transport cleanup failed')}},
  }
}
