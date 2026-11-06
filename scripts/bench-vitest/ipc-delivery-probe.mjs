// Diagnostic only: observe decoded IPC messages before Node dispatches them.
// Check the decoder's call path before accepting the decoded message. Initial
// native integrity and pre-installation activity remain unproven; never replay
// authority. Host-object decoding stays explicitly unsupported.
import {createHash,randomBytes} from 'node:crypto'

const childSource='0c2e2cc568bb1651aec25c8a9c7178380dfde9b4ac772b02c6286536ee4986b1'
const parserSource='59f0124b35fa9127200c10c5aa206b60ceaf0951d11947ce3e9a4afb1844f4f2'
const lineNumber=650
export function observeIpcDelivery(post,{callback,pendingLimit=128}={}) {
  if(!Number.isSafeInteger(pendingLimit)||pendingLimit<1)throw Error('invalid pending IPC limit')
  const group='quod-ipc-delivery-'+randomBytes(16).toString('hex'),pending=[],errors=[],sources=new Map()
  const parsers=new Map()
  const breakpoints=new Map(),nativeMethods=new Map()
  const decoder={started:0,headerEntries:0,valueEntries:0,completed:0,hostEntries:0,
    unrelatedNativeEntries:0,ignoredReceivers:0,groupsCreated:0,groupsReleased:0,failureFrames:[]}
  let decoding=null,hostMethod=null
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
  function properties(remote) {
    if(!remote?.objectId||remote.subtype==='proxy')throw Error('IPC decoder object unavailable')
    const r=post('Runtime.getProperties',{objectId:remote.objectId,ownProperties:true,generatePreview:false})
    if(r.exceptionDetails)throw Error('IPC decoder descriptors unavailable')
    return r
  }
  function data(rows,name) {
    const p=rows.find(p=>p.name===name)
    if(!p||p.isOwn!==true||!Object.hasOwn(p,'value')||p.get||p.set)
      throw Error('IPC decoder field is not own data: '+name)
    return p.value
  }
  function inheritedData(object,name) {
    for(let depth=0;depth<8&&object?.objectId;depth++) {
      const p=properties(object)
      if(p.result.some(p=>p.name===name))return data(p.result,name)
      object=p.internalProperties?.find(p=>p.name==='[[Prototype]]')?.value
    }
    throw Error('IPC decoder method unavailable: '+name)
  }
  function decoderRelease() {
    if(!decoding)return
    try{post('Runtime.releaseObjectGroup',{objectGroup:decoding.group});decoder.groupsReleased++}
    catch{fail('IPC decoder handles could not be released')}
    decoding=null
  }
  function checkMethod(instance,name) {
    if(!same(inheritedData(instance,name),nativeMethods.get(name)))throw Error('IPC decoder method changed: '+name)
  }
  function checkHostMethod(instance) {
    const fn=inheritedData(instance,'_readHostObject'),props=properties(fn)
    const loc=props.internalProperties?.find(p=>p.name==='[[FunctionLocation]]')?.value?.value
    if(fn.type!=='function'||!loc?.scriptId||loc.lineNumber!==39)throw Error('IPC host decoder function differs from pinned Node')
    source(loc.scriptId,parserSource)
    if(!hostMethod) {
      const r=post('Runtime.callFunctionOn',{objectId:fn.objectId,functionDeclaration:'function(){return this}',objectGroup:group,silent:true})
      if(r.exceptionDetails||!r.result?.objectId)throw Error('IPC host decoder handle unavailable')
      hostMethod=r.result
      const b=post('Debugger.setBreakpointOnFunctionCall',{objectId:hostMethod.objectId})
      breakpoints.set(b.breakpointId,'host')
    }else if(!same(fn,hostMethod))throw Error('IPC host decoder identity changed')
  }
  function decoderFrame(params,line,objectGroup) {
    const frame=params.callFrames?.[0],parent=params.callFrames?.[1]
    if(!frame||frame.location.lineNumber!==line)throw Error('IPC decoder caller moved from pinned source')
    source(frame.location.scriptId,parserSource)
    // A replacement generator.next can copy the yielded result before the
    // delivery checkpoint. Require the direct Node caller too.
    if(!parent||parent.location.lineNumber!==646)throw Error('IPC parser has an unexpected caller')
    source(parent.location.scriptId,childSource)
    if(!same(evaluate(parent,'target',objectGroup),receiver))return null
    return frame
  }
  function decoderPause(params,kinds) {
    // Source and function-entry breakpoints may share a pause. Validate the
    // lookup before accepting the corresponding actual native entry.
    for(const kind of ['header','value','native:readHeader','native:readValue','host'])if(kinds.includes(kind)) {
      if(kind==='header') {
        if(decoding){fail('IPC decoder started before its previous result was delivered');decoderRelease()}
        const eventGroup=group+'-decoder-'+(++decoder.groupsCreated)
        decoding={group:eventGroup,instance:null,channel:null,stage:'header',header:0,value:0}
        const frame=decoderFrame(params,101,eventGroup)
        if(!frame){decoder.ignoredReceivers++;decoderRelease();continue}
        decoding.instance=evaluate(frame,'deserializer',eventGroup)
        decoding.channel=evaluate(frame,'channel',eventGroup)
        decoder.started++
        checkMethod(decoding.instance,'readHeader');checkMethod(decoding.instance,'readValue')
        checkHostMethod(decoding.instance)
      }else if(kind==='value') {
        if(!decoding)continue
        const frame=decoderFrame(params,102,decoding.group)
        if(!frame||!same(evaluate(frame,'deserializer',decoding.group),decoding.instance))throw Error('IPC decoder instance changed before value read')
        if(decoding.header!==1)throw Error('IPC decoder header entry was not observed exactly once')
        decoding.stage='value';checkMethod(decoding.instance,'readValue');checkHostMethod(decoding.instance)
      }else if(kind==='host') {
        if(decoding){decoder.hostEntries++;throw Error('IPC host-object decoder callbacks are unverified')}
      }else {
        if(!decoding){decoder.unrelatedNativeEntries++;continue}
        const name=kind.slice(7),isHeader=name==='readHeader',line=isHeader?101:102
        const frame=decoderFrame(params,line,decoding.group)
        if(!frame||!same(evaluate(frame,'deserializer',decoding.group),decoding.instance))throw Error('IPC native decoder entry has another instance')
        if(decoding.stage!==(isHeader?'header':'value'))throw Error('IPC native decoder entry is out of order')
        if(isHeader){decoding.header++;decoder.headerEntries++}
        else{decoding.value++;decoder.valueEntries++}
        if(decoding[isHeader?'header':'value']!==1)throw Error('IPC native decoder entry repeated')
      }
    }
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
    return mode
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
    source(b.locations[0].scriptId,childSource)
    breakpoints.set(breakpointId,'delivery')
    const namespace=post('Runtime.evaluate',{expression:'process.getBuiltinModule("v8")',objectGroup:group,silent:true})
    if(namespace.exceptionDetails)throw Error('IPC decoder namespace unavailable')
    const ctor=data(properties(namespace.result).result,'Deserializer'),proto=data(properties(ctor).result,'prototype')
    for(const name of ['readHeader','readValue']) {
      const fn=data(properties(proto).result,name),p=properties(fn)
      if(fn.type!=='function'||p.internalProperties?.some(p=>['[[FunctionLocation]]','[[TargetFunction]]','[[BoundTargetFunction]]'].includes(p.name)))
        throw Error('IPC decoder baseline is not an unbound native method')
      nativeMethods.set(name,fn)
      const b=post('Debugger.setBreakpointOnFunctionCall',{objectId:fn.objectId})
      breakpoints.set(b.breakpointId,'native:'+name)
    }
    for(const [kind,line] of [['header',101],['value',102]]) {
      const b=post('Debugger.setBreakpointByUrl',{url:'node:internal/child_process/serialization',lineNumber:line,columnNumber:0})
      breakpoints.set(b.breakpointId,kind)
      if(!b.breakpointId||b.locations?.length!==1||b.locations[0].lineNumber!==line)throw Error('IPC decoder location is not uniquely loaded')
      source(b.locations[0].scriptId,parserSource)
    }
    installed=true
  }catch(e){fail(e.message)}
  return {
    handlePause(params) {
      const kinds=(params.hitBreakpoints??[]).map(id=>breakpoints.get(id)).filter(Boolean)
      if(!kinds.length)return false
      if(!kinds.includes('delivery')) {
        try {
          if(!installed||closed||kinds.length!==(params.hitBreakpoints??[]).length)throw Error('ambiguous IPC decoder pause')
          decoderPause(params,kinds)
        }catch(e){
          fail(e.message)
          if(decoder.failureFrames.length<6)decoder.failureFrames.push({kinds,reason:e.message,
            frames:(params.callFrames??[]).slice(0,4).map(f=>({name:f.functionName,...f.location}))})
        }
        return true
      }
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
        const mode=parser(frame,eventGroup)
        if(mode==='advanced') {
          try {
            if(!decoding||decoding.header!==1||decoding.value!==1||decoding.stage!=='value')throw Error('IPC delivery lacks complete decoder entries')
            if(!same(evaluate(frame,'channel',eventGroup),decoding.channel))throw Error('IPC decoder delivered through another channel')
            decoder.completed++
          }catch(e){fail(e.message)}
          finally{decoderRelease()}
        }
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
      decoder:{...decoder,pending:decoding!==null,initialNativeIntegrity:false,completeOwnership:false},
      limitation:'Pinned advanced decoder lookup, native entries and delivery are checked; initial native integrity, raw-byte aliases, constructors, host-object callbacks, JSON decoder ownership and pre-installation messages remain unproven'}),
    close() {
      if(closed)return
      closed=true
      for(const breakpointId of breakpoints.keys())try{post('Debugger.removeBreakpoint',{breakpointId})}catch{fail('IPC breakpoint removal failed')}
      decoderRelease()
      for(const entry of pending)release(entry.group)
      pending.length=0
      try{post('Runtime.releaseObjectGroup',{objectGroup:group});baselineReleased=true}
      catch{fail('IPC baseline handles could not be released')}
    },
  }
}
