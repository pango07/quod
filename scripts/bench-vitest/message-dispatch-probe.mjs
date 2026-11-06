// Diagnostic only. Snapshot the listener at actual emit entry, before a
// preceding listener can copy the message and remove itself. This does not
// establish ownership before deserialization, exclude earlier preloads, or
// distinguish genuine IPC from application-generated messages.
import {createHash,randomBytes} from 'node:crypto'

const eventsSource='113b0cd4b1c81c909022c572525ae844136a746dd26e2b7e4a3161841e744281'
const own=Object.hasOwn

// Call synchronously after installing the diagnostic as the first listener,
// before transferring control to the application. Capturing a reference here
// is an observation baseline, not proof that earlier code was trustworthy.
export function observeMessageDispatch(post,{pendingLimit=32}={}) {
  if(!Number.isSafeInteger(pendingLimit)||pendingLimit<1)throw Error('invalid pending message limit')
  const group='quod-message-dispatch-'+randomBytes(16).toString('hex')
  const breakpoints=new Map(),pending=[],errors=[],failureCounts=new Map()
  let receiver,observer,installed=false,closed=false,sequence=0
  let dispatches=0,callbacks=0,paired=0,ignoredReceivers=0,groupsCreated=0,groupsReleased=0
  let source=null
  function fail(reason) {
    failureCounts.set(reason,(failureCounts.get(reason)??0)+1)
    if(errors.length<20)errors.push(reason)
  }
  function release(objectGroup) {
    try{post('Runtime.releaseObjectGroup',{objectGroup});groupsReleased++}
    catch{fail('message handles could not be released')}
  }
  function fields(remote) {
    if(!remote?.objectId||remote.subtype==='proxy')throw Error('message registry object unavailable')
    const result=post('Runtime.getProperties',{objectId:remote.objectId,ownProperties:true,generatePreview:false})
    if(result.exceptionDetails||result.internalProperties?.some(p=>['[[Target]]','[[Handler]]'].includes(p.name)))
      throw Error('message registry descriptors unavailable')
    return result.result
  }
  function data(rows,name) {
    const field=rows.find(p=>p.name===name)
    if(!field||field.isOwn!==true||!own(field,'value')||field.get||field.set)
      throw Error('message registry field is not own data: '+name)
    return field.value
  }
  function same(a,b) {
    if(!a||!b||a.type!==b.type||a.subtype!==b.subtype)return false
    if(!a.objectId&&!b.objectId)return a.value===b.value&&a.unserializableValue===b.unserializableValue
    if(!a.objectId||!b.objectId)return false
    const r=post('Runtime.callFunctionOn',{objectId:a.objectId,
      functionDeclaration:'function(other){return this===other}',arguments:[{objectId:b.objectId}],
      returnByValue:true,silent:true})
    if(r.exceptionDetails)throw Error('message identity comparison unavailable')
    return r.result?.value===true
  }
  function firstListener() {
    const events=data(fields(receiver),'_events'),handler=data(fields(events),'message')
    if(handler.type==='function')return handler
    if(handler.subtype!=='array')throw Error('message listener list is not an array')
    const rows=fields(handler),length=data(rows,'length').value
    if(!Number.isSafeInteger(length)||length<1)throw Error('message listener list is empty')
    const first=data(rows,'0')
    if(first.type!=='function'||first.subtype==='proxy')throw Error('message listener is not an ordinary function')
    return first
  }
  function evaluate(frame,expression,objectGroup) {
    const r=post('Debugger.evaluateOnCallFrame',{callFrameId:frame.callFrameId,expression,
      objectGroup,silent:true,generatePreview:false})
    if(r.exceptionDetails||!r.result)throw Error('message entry arguments unavailable')
    return r.result
  }
  try {
    if(process.versions.node!=='24.18.0')throw Error('message dispatch requires pinned Node 24.18.0')
    const r=post('Runtime.evaluate',{expression:'process',objectGroup:group,silent:true})
    if(r.exceptionDetails||!r.result?.objectId)throw Error('process receiver unavailable')
    receiver=r.result
    // Resolve the actual function, including an inherited data property,
    // without invoking a replacement getter.
    let target=receiver,emit=null
    for(let depth=0;depth<8&&target?.objectId;depth++) {
      const props=post('Runtime.getProperties',{objectId:target.objectId,ownProperties:true,generatePreview:false})
      if(target.subtype==='proxy'||props.exceptionDetails)throw Error('process emit owner unavailable')
      if(props.result.some(p=>p.name==='emit')){emit=data(props.result,'emit');break}
      target=props.internalProperties?.find(p=>p.name==='[[Prototype]]')?.value
    }
    if(emit?.type!=='function'||emit.subtype==='proxy')throw Error('process emit function unavailable')
    const location=post('Runtime.getProperties',{objectId:emit.objectId,ownProperties:true}).internalProperties
      ?.find(p=>p.name==='[[FunctionLocation]]')?.value?.value
    if(!location?.scriptId)throw Error('process emit source unavailable')
    const script=post('Debugger.getScriptSource',{scriptId:location.scriptId}).scriptSource
    if(typeof script!=='string')throw Error('process emit source unavailable')
    source={sha256:createHash('sha256').update(script).digest('hex'),line:location.lineNumber,column:location.columnNumber}
    if(source.sha256!==eventsSource)throw Error('process emit source differs from pinned Node')
    observer=firstListener()
    // Function identity is retained. An identical function body is not enough
    // to pass the first-listener check at a later dispatch.
    for(const [kind,fn,condition] of [['emit',emit,'arguments[0] === "message"'],['observer',observer,null]]) {
      const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:fn.objectId,...(condition?{condition}:{})})
      breakpoints.set(breakpointId,kind)
    }
    installed=true
  }catch(e){fail(e.message)}
  return {
    handlePause(params) {
      const hits=(params.hitBreakpoints??[]).map(id=>breakpoints.get(id)).filter(Boolean)
      if(!hits.length)return false
      if(closed){fail('message entry after close');return true}
      const frame=params.callFrames?.[0],eventGroup=group+'-event-'+(++sequence)
      groupsCreated++
      let retain=false
      try {
        if(hits.length!==1||!frame)throw Error('ambiguous message entry')
        const actualReceiver=evaluate(frame,'this',eventGroup)
        if(hits[0]==='emit') {
          if(!same(receiver,actualReceiver)){ignoredReceivers++;return true}
          dispatches++
          if(evaluate(frame,'arguments[0]',eventGroup).value!=='message')throw Error('unexpected dispatch event')
          const message=evaluate(frame,'arguments[1]',eventGroup)
          // This snapshot occurs before the original emit body executes.
          // Failures stay recorded even if the listener disappears afterward.
          try{if(!same(observer,firstListener()))fail('observer was not first at message dispatch')}
          catch(e){fail(e.message)}
          if(pending.length>=pendingLimit)throw Error('pending message capacity exceeded')
          pending.push({message,group:eventGroup});retain=true
        } else {
          callbacks++
          const message=evaluate(frame,'arguments[0]',eventGroup),last=pending.at(-1)
          if(!same(receiver,actualReceiver)||!last||!same(last.message,message))
            throw Error('observer callback has no matching message dispatch')
          pending.pop();paired++;release(last.group)
        }
      }catch(e){fail(e.message)}
      finally{if(!retain)release(eventGroup)}
      return true
    },
    report() {
      return {available:installed&&!closed&&!errors.length&&!pending.length,
        installed,closed,source,dispatches,callbacks,paired,pending:pending.length,ignoredReceivers,
        groupsCreated,groupsReleased,errors:[...errors],failureCounts:Object.fromEntries(failureCounts),
        replayEligible:false,completeOwnership:false,
        limitation:'Observed first listener and paired callbacks only; earlier preloads, deserialization, alternative dispatch paths and genuine IPC origin are not certified'}
    },
    close() {
      if(closed)return
      closed=true
      for(const breakpointId of breakpoints.keys())try{post('Debugger.removeBreakpoint',{breakpointId})}catch{fail('message breakpoint removal failed')}
      for(const entry of pending)release(entry.group)
      pending.length=0
      try{post('Runtime.releaseObjectGroup',{objectGroup:group})}catch{fail('message baseline handles could not be released')}
    },
  }
}
