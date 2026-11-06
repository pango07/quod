// Diagnostic only: observe decoded IPC messages before Node dispatches them.
// This is after deserialization. Native decoder integrity, earlier decoder
// copies and pre-installation activity remain unproven; never replay authority.
import {createHash,randomBytes} from 'node:crypto'

const childSource='0c2e2cc568bb1651aec25c8a9c7178380dfde9b4ac772b02c6286536ee4986b1'
const parserSource='59f0124b35fa9127200c10c5aa206b60ceaf0951d11947ce3e9a4afb1844f4f2'
const lineNumber=650
export function observeIpcDelivery(post,{callback,pendingLimit=128}={}) {
  if(!Number.isSafeInteger(pendingLimit)||pendingLimit<1)throw Error('invalid pending IPC limit')
  const group='quod-ipc-delivery-'+randomBytes(16).toString('hex'),pending=[],errors=[],sources=new Map()
  const parsers=new Map()
  let receiver,breakpointId=null,installed=false,closed=false,baselineReleased=false
  let decoded=0,prepared=0,paired=0,ignoredReceivers=0,failures=0,groupsCreated=0,groupsReleased=0
  function fail(reason){failures++;if(errors.length<20)errors.push(reason)}
  function release(objectGroup) {
    try{post('Runtime.releaseObjectGroup',{objectGroup});groupsReleased++}
    catch{fail('IPC delivery handles could not be released')}
  }
  function source(scriptId,expected) {
    if(!sources.has(scriptId)) {
      if(sources.size>=16)throw Error('IPC source capacity exceeded')
      const text=post('Debugger.getScriptSource',{scriptId}).scriptSource
      if(typeof text!=='string')throw Error('IPC source unavailable')
      sources.set(scriptId,createHash('sha256').update(text).digest('hex'))
    }
    if(sources.get(scriptId)!==expected)throw Error('IPC executable source differs from pinned Node')
  }
  function same(a,b) {
    if(!a?.objectId||!b?.objectId)return false
    const r=post('Runtime.callFunctionOn',{objectId:receiver.objectId,
      functionDeclaration:'function(a,b){return a===b}',arguments:[{objectId:a.objectId},{objectId:b.objectId}],
      returnByValue:true,silent:true})
    if(r.exceptionDetails)throw Error('IPC identity comparison unavailable')
    return r.result?.value===true
  }
  function evaluate(frame,expression,objectGroup) {
    const r=post('Debugger.evaluateOnCallFrame',{callFrameId:frame.callFrameId,expression,objectGroup,silent:true,generatePreview:false})
    if(r.exceptionDetails||!r.result)throw Error('IPC delivery local unavailable')
    return r.result
  }
  function parser(frame,objectGroup) {
    const fn=evaluate(frame,'parseChannelMessages',objectGroup)
    if(fn.type!=='function'||fn.subtype==='proxy'||!fn.objectId)throw Error('IPC parser is not an ordinary function')
    const r=post('Runtime.getProperties',{objectId:fn.objectId,ownProperties:true,generatePreview:false})
    if(r.exceptionDetails)throw Error('IPC parser location unavailable')
    const location=r.internalProperties?.find(p=>p.name==='[[FunctionLocation]]')?.value?.value
    if(!location?.scriptId)throw Error('IPC parser location unavailable')
    source(location.scriptId,parserSource)
    const mode=location.lineNumber===60?'advanced':location.lineNumber===141?'json':null
    if(!mode)throw Error('IPC parser function differs from pinned Node')
    parsers.set(mode,{mode,line:location.lineNumber,column:location.columnNumber,sha256:parserSource})
  }
  try {
    if(process.versions.node!=='24.18.0')throw Error('IPC delivery requires pinned Node 24.18.0')
    if(callback?.type!=='function'||callback.subtype==='proxy'||!callback.objectId)throw Error('IPC preparation callback unavailable')
    const r=post('Runtime.evaluate',{expression:'process',objectGroup:group,silent:true})
    if(r.exceptionDetails||!r.result?.objectId)throw Error('IPC process receiver unavailable')
    receiver=r.result
    const b=post('Debugger.setBreakpointByUrl',{url:'node:internal/child_process',lineNumber,columnNumber:0})
    breakpointId=b.breakpointId
    if(!breakpointId||b.locations?.length!==1||b.locations[0].lineNumber!==lineNumber)
      throw Error('IPC delivery location is not uniquely loaded')
    source(b.locations[0].scriptId,childSource);installed=true
  }catch(e){fail(e.message)}
  return {
    handlePause(params) {
      if(!breakpointId||!params.hitBreakpoints?.includes(breakpointId))return false
      const eventGroup=group+'-'+(++groupsCreated)
      let retained=false
      try {
        if(!installed||closed)throw Error('IPC delivery observer unavailable')
        const frame=params.callFrames?.[0]
        if(!frame||params.hitBreakpoints.length!==1||frame.location.lineNumber!==lineNumber)
          throw Error('ambiguous IPC delivery pause')
        source(frame.location.scriptId,childSource)
        if(!same(evaluate(frame,'target',eventGroup),receiver)){ignoredReceivers++;return true}
        decoded++
        parser(frame,eventGroup)
        const message=evaluate(frame,'message',eventGroup)
        if(message.type!=='object'||message.subtype==='proxy'||!message.objectId)
          throw Error('IPC decoded message is not an ordinary object')
        if(pending.length>=pendingLimit)throw Error('pending IPC delivery capacity exceeded')
        const entry={message,group:eventGroup,prepared:false}
        pending.push(entry);retained=true
        const r=post('Runtime.callFunctionOn',{objectId:callback.objectId,
          functionDeclaration:'function(value){return this(value)}',arguments:[{objectId:message.objectId}],
          objectGroup:eventGroup,returnByValue:true,silent:true})
        if(r.exceptionDetails||r.result?.value!==true)throw Error('IPC preparation callback did not accept the message')
        entry.prepared=true;prepared++
      }catch(e){fail(e.message)}
      finally{if(!retained)release(eventGroup)}
      return true
    },
    pair(message) {
      try {
        if(!installed||closed)throw Error('IPC callback arrived without a live delivery observer')
        const entry=pending[0]
        if(!entry||!same(entry.message,message))throw Error('message callback did not match the next decoded IPC message')
        pending.shift();release(entry.group);paired++
        if(!entry.prepared)throw Error('IPC callback arrived without successful preparation')
        return true
      }catch(e){fail(e.message);return false}
    },
    report:()=>({available:installed&&!closed&&!failures&&!pending.length,installed,closed,
      decoded,prepared,paired,pending:pending.length,ignoredReceivers,groupsCreated,groupsReleased,baselineReleased,
      parsers:[...parsers.values()],failures,errors:[...errors],replayEligible:false,completeOwnership:false,
      limitation:'Observed after deserialization and paired with the captured message observer; earlier decoder copies, decoder integrity, initial runtime integrity and pre-installation messages remain unproven'}),
    close() {
      if(closed)return
      closed=true
      if(breakpointId)try{post('Debugger.removeBreakpoint',{breakpointId})}catch{fail('IPC breakpoint removal failed')}
      for(const entry of pending)release(entry.group)
      pending.length=0
      try{post('Runtime.releaseObjectGroup',{objectGroup:group});baselineReleased=true}
      catch{fail('IPC baseline handles could not be released')}
    },
  }
}
