// Diagnostic only. Snapshot the listener at actual emit entry, before a
// preceding listener can copy the message and remove itself. This does not
// establish ownership before deserialization, exclude earlier preloads, or
// distinguish genuine IPC from application-generated messages.
import {createHash,randomBytes} from 'node:crypto'

const eventsSource='113b0cd4b1c81c909022c572525ae844136a746dd26e2b7e4a3161841e744281'
const warningWrapperSource='a5012da3d2027007781baecda154d343d8c7ad2327b79bec65b1b60a41f42dd1'
const own=Object.hasOwn

// Call synchronously after installing the diagnostic as the first listener,
// before transferring control to the application. Capturing a reference here
// is an observation baseline, not proof that earlier code was trustworthy.
export function observeMessageDispatch(post,{pendingLimit=32,onObserverMessage=null}={}) {
  if(!Number.isSafeInteger(pendingLimit)||pendingLimit<1)throw Error('invalid pending message limit')
  const group='quod-message-dispatch-'+randomBytes(16).toString('hex')
  const breakpoints=new Map(),pending=[],wrapperPending=[],errors=[],failureCounts=new Map()
  let receiver,observer,nativeEmit,reflectNamespace,reflectApply,ownDescriptor,listenerApply,emittingSymbol,installed=false,closed=false,sequence=0
  let dispatches=0,callbacks=0,paired=0,ignoredReceivers=0,groupsCreated=0,groupsReleased=0
  let wrapperEntries=0,wrapperForwarded=0,wrapperChecks=0,singleListenerChecks=0,arrayListenerChecks=0
  let source=null,wrapperSource=null
  function fail(reason) {
    failureCounts.set(reason,(failureCounts.get(reason)??0)+1)
    if(errors.length<20)errors.push(reason)
  }
  function release(objectGroup) {
    try{post('Runtime.releaseObjectGroup',{objectGroup});groupsReleased++}
    catch{fail('message handles could not be released')}
  }
  function inspect(remote) {
    if(!remote?.objectId||remote.subtype==='proxy')throw Error('message registry object unavailable')
    const result=post('Runtime.getProperties',{objectId:remote.objectId,ownProperties:true,generatePreview:false})
    if(result.exceptionDetails||result.internalProperties?.some(p=>['[[Target]]','[[Handler]]'].includes(p.name)))
      throw Error('message registry descriptors unavailable')
    return result
  }
  function fields(remote){return inspect(remote).result}
  function data(rows,name) {
    const field=rows.find(p=>p.name===name)
    if(!field||field.isOwn!==true||!own(field,'value')||field.get||field.set)
      throw Error('message registry field is not own data: '+name)
    return field.value
  }
  function same(a,b) {
    if(!a||!b||a.type!==b.type||a.subtype!==b.subtype)return false
    if(['object','function','symbol'].includes(a.type)&&a.subtype!=='null'&&(!a.objectId||!b.objectId))
      throw Error('message identity handle unavailable')
    if(!a.objectId&&!b.objectId)return a.value===b.value&&a.unserializableValue===b.unserializableValue
    if(!a.objectId||!b.objectId)return false
    // Passing both values as arguments also preserves primitive Symbol
    // identity; using a Symbol as `this` would box it in a non-strict helper.
    const r=post('Runtime.callFunctionOn',{objectId:receiver.objectId,
      functionDeclaration:'function(a,b){return a===b}',arguments:[{objectId:a.objectId},{objectId:b.objectId}],
      returnByValue:true,silent:true})
    if(r.exceptionDetails)throw Error('message identity comparison unavailable')
    return r.result?.value===true
  }
  function inheritedData(object,name) {
    for(let depth=0;depth<8&&object?.objectId;depth++) {
      const props=inspect(object)
      if(props.result.some(p=>p.name===name))return data(props.result,name)
      object=props.internalProperties?.find(p=>p.name==='[[Prototype]]')?.value
    }
    throw Error('message invocation property unavailable: '+name)
  }
  function listenerState(target=receiver) {
    const events=data(fields(target),'_events'),handler=data(fields(events),'message')
    if(handler.type==='function')return {first:handler,kind:'single'}
    if(handler.subtype!=='array')throw Error('message listener list is not an array')
    const rows=fields(handler),length=data(rows,'length').value
    if(!Number.isSafeInteger(length)||length<1)throw Error('message listener list is empty')
    const first=data(rows,'0')
    if(first.type!=='function'||first.subtype==='proxy')throw Error('message listener is not an ordinary function')
    return {first,kind:'array',rows}
  }
  function checkListenerInvocation(state) {
    if(state.kind==='single') {
      if(!same(listenerApply,inheritedData(state.first,'apply')))throw Error('single message listener apply changed')
      singleListenerChecks++
    } else {
      const field=state.rows.find(p=>p.symbol&&same(p.symbol,emittingSymbol))
      if(!field||!own(field,'value')||field.get||field.set||field.writable!==true||
        !Number.isSafeInteger(field.value.value)||field.value.value<0)
        throw Error('message array emission counter is not ordinary writable data')
      arrayListenerChecks++
    }
  }
  function evaluate(frame,expression,objectGroup) {
    const r=post('Debugger.evaluateOnCallFrame',{callFrameId:frame.callFrameId,expression,
      objectGroup,silent:true,generatePreview:false})
    if(r.exceptionDetails||!r.result)throw Error('message entry arguments unavailable')
    return r.result
  }
  function functionSource(fn) {
    if(fn?.type!=='function'||fn.subtype==='proxy')throw Error('process emit function unavailable')
    const location=post('Runtime.getProperties',{objectId:fn.objectId,ownProperties:true}).internalProperties
      ?.find(p=>p.name==='[[FunctionLocation]]')?.value?.value
    if(!location?.scriptId)throw Error('process emit source unavailable')
    const script=post('Debugger.getScriptSource',{scriptId:location.scriptId}).scriptSource
    if(typeof script!=='string')throw Error('process emit source unavailable')
    return {sha256:createHash('sha256').update(script).digest('hex'),line:location.lineNumber,column:location.columnNumber}
  }
  function binding(fn,name) {
    const scopes=post('Runtime.getProperties',{objectId:fn.objectId,ownProperties:true}).internalProperties
      ?.find(p=>p.name==='[[Scopes]]')?.value
    for(const scope of fields(scopes)) {
      if(!/^\d+$/.test(scope.name)||scope.value?.description==='Global')continue
      const bindings=fields(scope.value)
      if(bindings.some(p=>p.name===name))return data(bindings,name)
    }
    throw Error('message dispatch binding unavailable: '+name)
  }
  function checkWrapperInvocation(frame,eventGroup) {
    if(frame.scopeChain?.some(s=>s.type==='with'))throw Error('warning wrapper has an unexpected with scope')
    const globals=frame.scopeChain?.filter(s=>s.type==='global')??[]
    if(globals.length!==1)throw Error('warning wrapper global scope unavailable')
    // Enumerating the entire global with Runtime.getProperties initializes
    // Node's lazy web globals (including Undici). Query only this descriptor
    // through the captured builtin, without invoking its getter. The returned
    // descriptor and its descendants belong to the transient event group.
    const described=post('Runtime.callFunctionOn',{objectId:ownDescriptor.objectId,
      functionDeclaration:'function(object){return this(object,"Reflect")}',
      arguments:[{objectId:globals[0].object.objectId}],objectGroup:eventGroup,silent:true})
    if(described.exceptionDetails||!described.result?.objectId)throw Error('warning wrapper global descriptor unavailable')
    const globalReflect=data(fields(described.result),'value')
    if(!same(globalReflect,reflectNamespace))throw Error('warning wrapper global Reflect changed')
    // An own data property excludes a global getter. The real lexical lookup
    // also catches a later Script-scope `let Reflect`, without invoking
    // application code through a with scope or a global accessor.
    const actual=evaluate(frame,'Reflect',eventGroup)
    if(!same(actual,reflectNamespace))throw Error('warning wrapper lexical Reflect changed')
    if(!same(data(fields(actual),'apply'),reflectApply))throw Error('warning wrapper Reflect.apply changed')
    wrapperChecks++
  }
  try {
    if(process.versions.node!=='24.18.0')throw Error('message dispatch requires pinned Node 24.18.0')
    const r=post('Runtime.evaluate',{expression:'process',objectGroup:group,silent:true})
    if(r.exceptionDetails||!r.result?.objectId)throw Error('process receiver unavailable')
    receiver=r.result
    // Resolve the actual function, including an inherited data property,
    // without invoking a replacement getter.
    let emit=inheritedData(receiver,'emit'),wrapper=null
    source=functionSource(emit)
    if(source.sha256===warningWrapperSource&&source.line===14&&source.column===24) {
      // Vitest requires suppress-warnings.cjs before ESM imports. Observe
      // the actual captured Node function, not a guessed public prototype.
      wrapperSource=source;wrapper=emit;emit=binding(emit,'emit');source=functionSource(emit)
    }
    // The file digest alone also matches removeListener/addListener and every
    // other function in node:events. Pin emit's actual function location too.
    if(source.sha256!==eventsSource||source.line!==455||source.column!==43)
      throw Error('process emit function differs from pinned Node')
    nativeEmit=emit
    observer=listenerState().first
    listenerApply=inheritedData(observer,'apply')
    reflectApply=binding(emit,'ReflectApply')
    emittingSymbol=binding(emit,'kEmitting')
    if(reflectApply.type!=='function'||emittingSymbol.type!=='symbol')throw Error('Node dispatch invocation bindings unavailable')
    {
      // Capture the invocation baseline even before Vitest installs its
      // warning wrapper. A later handoff must compare with this early state.
      const d=post('Runtime.evaluate',{expression:'Object.getOwnPropertyDescriptor',objectGroup:group,silent:true})
      if(d.exceptionDetails||d.result?.type!=='function'||!d.result.objectId)throw Error('global descriptor builtin unavailable')
      ownDescriptor=d.result
      const r=post('Runtime.evaluate',{expression:'Reflect',objectGroup:group,silent:true})
      if(r.exceptionDetails||!r.result?.objectId)throw Error('warning wrapper Reflect unavailable')
      reflectNamespace=r.result
      if(!same(data(fields(reflectNamespace),'apply'),reflectApply))throw Error('initial warning wrapper Reflect.apply differs from Node primordial')
    }
    // Function identity is retained. An identical function body is not enough
    // to pass the first-listener check at a later dispatch.
    for(const [kind,fn,condition] of [['emit',emit,'arguments[0] === "message"'],['observer',observer,null],
      ...(wrapper?[['wrapper',wrapper,'arguments[0] === "message"']]:[])]) {
      const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:fn.objectId,...(condition?{condition}:{})})
      breakpoints.set(breakpointId,kind)
    }
    installed=true
  }catch(e){fail(e.message)}
  return {
    captureCurrentWrapper() {
      try {
        if(!installed||closed||wrapperSource||dispatches||callbacks||pending.length||wrapperPending.length)
          throw Error('warning wrapper handoff is not at the initial boundary')
        const wrapper=inheritedData(receiver,'emit')
        if(same(wrapper,nativeEmit))return true
        const current=functionSource(wrapper)
        if(current.sha256!==warningWrapperSource||current.line!==14||current.column!==24||!same(binding(wrapper,'emit'),nativeEmit))
          throw Error('late warning wrapper differs from pinned Node forwarding')
        // No observer or breakpoint is replaced. Earlier failures and the
        // original native function baseline survive this additional guard.
        const b=post('Debugger.setBreakpointOnFunctionCall',{objectId:wrapper.objectId,condition:'arguments[0] === "message"'})
        breakpoints.set(b.breakpointId,'wrapper');wrapperSource=current
        return true
      }catch(e){fail(e.message);return false}
    },
    ipcDeliveryCallback() {
      if(!installed||closed)throw Error('message observer unavailable for IPC registration')
      const callback=binding(observer,'prepareWorkerMessage')
      if(callback.type!=='function'||callback.subtype==='proxy')throw Error('IPC preparation callback unavailable')
      return callback
    },
    transportDeliveryCallback() {
      if(!installed||closed)throw Error('message observer unavailable for transport registration')
      // The bootstrap's captured observer closes over its diagnostic transport.
      // Resolve that exact callback without publishing a global bridge or
      // transferring application values through the Inspector protocol.
      const callback=data(fields(binding(observer,'transport')),'delivered')
      if(callback.type!=='function'||callback.subtype==='proxy')throw Error('transport delivery callback unavailable')
      return callback
    },
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
        if(hits[0]==='wrapper') {
          if(!same(receiver,actualReceiver)){ignoredReceivers++;return true}
          wrapperEntries++
          const message=evaluate(frame,'arguments[1]',eventGroup)
          try{checkWrapperInvocation(frame,eventGroup)}catch(e){fail(e.message)}
          if(wrapperPending.length>=pendingLimit)throw Error('pending warning wrapper capacity exceeded')
          wrapperPending.push({message,group:eventGroup});retain=true
        } else if(hits[0]==='emit') {
          if(!same(receiver,actualReceiver)){ignoredReceivers++;return true}
          dispatches++
          if(evaluate(frame,'arguments[0]',eventGroup).value!=='message')throw Error('unexpected dispatch event')
          const message=evaluate(frame,'arguments[1]',eventGroup)
          if(wrapperSource) {
            const last=wrapperPending.at(-1)
            if(!last||!same(last.message,message))fail('Node dispatch bypassed or changed the warning wrapper message')
            else{wrapperPending.pop();wrapperForwarded++;release(last.group)}
          }
          // This snapshot occurs before the original emit body executes.
          // Failures stay recorded even if the listener disappears afterward.
          try{
            // Use the current event's receiver handle, so registry objects
            // and removed listener arrays are released with that event.
            const state=listenerState(actualReceiver)
            if(!same(observer,state.first))fail('observer was not first at message dispatch')
            checkListenerInvocation(state)
          }
          catch(e){fail(e.message)}
          if(pending.length>=pendingLimit)throw Error('pending message capacity exceeded')
          pending.push({message,group:eventGroup});retain=true
        } else {
          callbacks++
          const message=evaluate(frame,'arguments[0]',eventGroup),last=pending.at(-1)
          if(onObserverMessage&&onObserverMessage(message)!==true)fail('observer callback has no prepared IPC delivery')
          if(!same(receiver,actualReceiver)||!last||!same(last.message,message))
            throw Error('observer callback has no matching message dispatch')
          pending.pop();paired++;release(last.group)
        }
      }catch(e){fail(e.message)}
      finally{if(!retain)release(eventGroup)}
      return true
    },
    report() {
      return {available:installed&&!closed&&!errors.length&&!pending.length&&!wrapperPending.length,
        installed,closed,source,wrapperSource,dispatches,callbacks,paired,pending:pending.length,ignoredReceivers,
        invocation:{wrapperEntries,wrapperForwarded,wrapperChecks,wrapperPending:wrapperPending.length,singleListenerChecks,arrayListenerChecks},
        groupsCreated,groupsReleased,errors:[...errors],failureCounts:Object.fromEntries(failureCounts),
        replayEligible:false,completeOwnership:false,
        limitation:'Observed wrapper and Node invocation state plus paired callbacks only; earlier preloads, initial runtime integrity, deserialization, alternative dispatch paths and genuine IPC origin are not certified'}
    },
    close() {
      if(closed)return
      closed=true
      for(const breakpointId of breakpoints.keys())try{post('Debugger.removeBreakpoint',{breakpointId})}catch{fail('message breakpoint removal failed')}
      for(const entry of pending)release(entry.group)
      for(const entry of wrapperPending)release(entry.group)
      pending.length=0
      wrapperPending.length=0
      try{post('Runtime.releaseObjectGroup',{objectGroup:group})}catch{fail('message baseline handles could not be released')}
    },
  }
}
