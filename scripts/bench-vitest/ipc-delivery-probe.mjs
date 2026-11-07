// Diagnostic only: observe decoded IPC messages before Node dispatches them.
// Check the decoder's call path before accepting the decoded message. Initial
// native integrity and pre-installation activity remain unproven; never replay
// authority. Host-object decoding stays explicitly unsupported.
import {createHash,randomBytes} from 'node:crypto'

const childSource='0c2e2cc568bb1651aec25c8a9c7178380dfde9b4ac772b02c6286536ee4986b1'
const parserSource='59f0124b35fa9127200c10c5aa206b60ceaf0951d11947ce3e9a4afb1844f4f2'
const v8Source='ed24392fbc14a67d065dedb839b975719c262f03a3b2bcf2c6aeb278a05e700d'
const asyncSource='08a5d05e584c48a6f51edd61e1e6cb117906d763e521e4b8dc2c7227462bbdca'
const bufferSource='85699dbf250b11767414f73c8912ef5b5578ae8f361730cdb2331763efa29ef7'
const internalBufferSource='ce1f2b80ecaf7f4d8ef3ff5d40e77e48c4413760e1a743c8c1b24cabfc1c25d8'
const lineNumber=650
export function observeIpcDelivery(post,{callback,pendingLimit=128}={}) {
  if(!Number.isSafeInteger(pendingLimit)||pendingLimit<1)throw Error('invalid pending IPC limit')
  const group='quod-ipc-delivery-'+randomBytes(16).toString('hex'),pending=[],errors=[],sources=new Map()
  const parsers=new Map()
  const breakpoints=new Map(),nativeMethods=new Map()
  const decoder={started:0,constructorEntries:0,headerEntries:0,valueEntries:0,completed:0,hostEntries:0,
    unrelatedConstructorEntries:0,unrelatedNativeEntries:0,ignoredReceivers:0,groupsCreated:0,groupsReleased:0,failureFrames:[]}
  const reads={entries:0,directEntries:0,trampolineEntries:0,completed:0,delivered:0,ignoredReceivers:0,groupsCreated:0,groupsReleased:0,failureFrames:[]}
  const slices={entries:0,checked:0,uint8:0,buffer:0,failures:0}
  let uint8Constructor,uint8Prototype,bufferConstructor,bufferPrototype,fastBuffer,speciesSymbol,typedSpeciesGetter,bufferSpeciesGetter,ownDescriptor
  let reading=null,readReturn=null
  let decoding=null,hostMethod=null,nativeConstructor=null,defaultConstructor=null,objectPrototype=null
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
  function properties(remote,nonIndexedPropertiesOnly=false) {
    if(!remote?.objectId||remote.subtype==='proxy')throw Error('IPC decoder object unavailable')
    const r=post('Runtime.getProperties',{objectId:remote.objectId,ownProperties:true,generatePreview:false,nonIndexedPropertiesOnly})
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
  function prototype(object) {
    const p=properties(object).internalProperties?.find(p=>p.name==='[[Prototype]]')?.value
    if(!p?.objectId||p.subtype==='proxy')throw Error('IPC decoder prototype unavailable')
    return p
  }
  function descriptor(object,name,objectGroup=group) {
    for(let depth=0;depth<8&&object?.objectId;depth++) {
      if(object.subtype==='proxy')throw Error('IPC slice descriptor receiver is a proxy')
      // Inspector can expose native accessors as computed values. Use the
      // captured native descriptor function to retain the actual getter.
      const r=post('Runtime.callFunctionOn',{objectId:ownDescriptor.objectId,
        functionDeclaration:'function(object,key){return this(object,key)}',
        arguments:[{objectId:object.objectId},typeof name==='string'?{value:name}:{objectId:name.objectId}],
        objectGroup,silent:true})
      if(r.exceptionDetails)throw Error('IPC slice descriptor inspection failed')
      if(r.result?.objectId) {
        const row={}
        for(const field of properties(r.result).result)row[field.name]=data([field],field.name)
        return row
      }
      const p=properties(object,true)
      object=p.internalProperties?.find(p=>p.name==='[[Prototype]]')?.value
    }
    throw Error('IPC slice descriptor unavailable')
  }
  function checkSlice(frame,objectGroup) {
    slices.entries++
    try {
      // Subarray reads constructor and @@species before creating a view.
      // Inspect descriptors without running accessors. A self-restoring
      // species constructor can retain bytes before Deserializer is entered.
      const view=evaluate(frame,'concatenatedBuffer',objectGroup)
      const p=properties(view,true),proto=p.internalProperties?.find(p=>p.name==='[[Prototype]]')?.value
      const ctorField=descriptor(view,'constructor',objectGroup)
      if(!Object.hasOwn(ctorField,'value')||ctorField.get||ctorField.set)throw Error('constructor is observable')
      const ctor=ctorField.value,isUint8=same(ctor,uint8Constructor),isBuffer=same(ctor,bufferConstructor)
      if(!(isUint8&&same(proto,uint8Prototype))&&!(isBuffer&&same(proto,bufferPrototype)))throw Error('constructor or view prototype changed')
      const species=descriptor(ctor,speciesSymbol,objectGroup),getter=isUint8?typedSpeciesGetter:bufferSpeciesGetter
      if(Object.hasOwn(species,'value')||!same(species.get,getter)||(species.set&&species.set.type!=='undefined'))
        throw Error('species lookup changed')
      if(isBuffer&&(!same(prototype(fastBuffer),uint8Constructor)||!same(data(properties(fastBuffer).result,'prototype'),bufferPrototype)))
        throw Error('FastBuffer constructor changed')
      slices[isUint8?'uint8':'buffer']++;slices.checked++
    }catch(e){slices.failures++;fail('IPC slice '+e.message)}
  }
  function pinnedFunction(fn,expected,line) {
    const p=properties(fn),loc=p.internalProperties?.find(p=>p.name==='[[FunctionLocation]]')?.value?.value
    if(fn.type!=='function'||!loc?.scriptId||loc.lineNumber!==line)throw Error('IPC constructor function differs from pinned Node: expected line '+line+', observed '+loc?.lineNumber)
    source(loc.scriptId,expected)
  }
  function checkBufferInitialization(object) {
    // Node's native Deserializer constructor uses ordinary Set for buffer.
    // An inherited setter can retain its bytes and restore itself before
    // readHeader. Inspect descriptors without invoking that setter, both
    // before argument evaluation and at the actual native constructor entry.
    for(let depth=0;depth<8;depth++) {
      const p=properties(object)
      if(p.result.some(p=>p.name==='buffer'))throw Error('IPC decoder buffer initialization is observable')
      const parent=p.internalProperties?.find(p=>p.name==='[[Prototype]]')?.value
      if(parent?.subtype==='null')return
      // Inspector omits a null prototype. Accept that omission only for the
      // intrinsic immutable Object.prototype captured from an object literal.
      if(!parent&&same(object,objectPrototype))return
      if(!parent?.objectId||parent.subtype==='proxy')throw Error('IPC decoder buffer prototype chain unavailable')
      object=parent
    }
    throw Error('IPC decoder buffer prototype chain exceeds limit')
  }
  function constructorChain(frame,objectGroup) {
    const ctor=evaluate(frame,'ChildProcessDeserializer',objectGroup)
    pinnedFunction(ctor,parserSource,38)
    if(!same(prototype(ctor),defaultConstructor)||!same(prototype(defaultConstructor),nativeConstructor))
      throw Error('IPC decoder constructor chain changed')
    const proto=data(properties(ctor).result,'prototype')
    checkBufferInitialization(proto)
    return proto
  }
  function decoderRelease() {
    if(!decoding)return
    try{post('Runtime.releaseObjectGroup',{objectGroup:decoding.group});decoder.groupsReleased++}
    catch{fail('IPC decoder handles could not be released')}
    decoding=null
  }
  function readRelease() {
    if(!reading)return
    try{post('Runtime.releaseObjectGroup',{objectGroup:reading.group});reads.groupsReleased++}
    catch{fail('IPC read handles could not be released')}
    reading=null
  }
  function readPause(params,kind) {
    const frame=params.callFrames?.[0]
    if(kind==='read:entry') {
      if(reading){fail('IPC read entered before its previous callback returned');readRelease()}
      const eventGroup=group+'-read-'+(++reads.groupsCreated)
      reading={group:eventGroup,channel:null,checked:false}
      if(!frame||frame.location.lineNumber!==637)throw Error('IPC read entry moved from pinned source')
      source(frame.location.scriptId,childSource)
      if(!same(evaluate(frame,'target',eventGroup),receiver)){reads.ignoredReceivers++;readRelease();return}
      reads.entries++
      reading.channel=evaluate(frame,'channel',eventGroup)
      if(!same(frame.this,reading.channel))throw Error('IPC read callback has another receiver')
      const args=properties(evaluate(frame,'arguments',eventGroup)).result
      if(data(args,'length').value!==1)throw Error('IPC read callback has unexpected arguments')
      // StreamBase dispatches its native internal callback slot. A public
      // onread getter can shadow the JS accessor without changing that slot.
      // Validate the executing frame, not a later public property lookup.
      if(frame.functionLocation?.scriptId!==frame.location.scriptId||frame.functionLocation?.lineNumber!==636)
        throw Error('IPC read callback differs from pinned Node')
      if(params.callFrames.length===1)reads.directEntries++
      else {
        const parent=params.callFrames[1]
        if(params.callFrames.length!==2||parent.location.lineNumber!==129)throw Error('IPC read callback has an unexpected caller')
        source(parent.location.scriptId,asyncSource)
        pinnedFunction(evaluate(parent,'cb',eventGroup),childSource,636)
        if(!same(parent.this,reading.channel)||!same(evaluate(parent,'resource',eventGroup),reading.channel))
          throw Error('IPC async callback has another resource')
        const forwarded=properties(evaluate(parent,'args',eventGroup)).result
        if(data(forwarded,'length').value!==1||!same(data(forwarded,'0'),data(args,'0')))
          throw Error('IPC async callback arguments differ')
        reads.trampolineEntries++
      }
      reading.checked=true
    }else {
      if(!reading)return
      try {
        if(!frame||frame.location.lineNumber!==readReturn.lineNumber||frame.location.columnNumber!==readReturn.columnNumber)
          throw Error('IPC read return moved from pinned source')
        source(frame.location.scriptId,childSource)
        if(!same(evaluate(frame,'channel',reading.group),reading.channel))throw Error('IPC read returned through another channel')
        if(decoding){fail('IPC read returned before decoder delivery');decoderRelease()}
        reads.completed++
      }finally{readRelease()}
    }
  }
  function requireRead(channel) {
    if(!reading?.checked||!same(channel,reading.channel))throw Error('IPC decoder lacks a validated native read callback')
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
    for(const kind of ['construct','native:construct','slice:remainder','header','value','native:readHeader','native:readValue','host'])if(kinds.includes(kind)) {
      if(kind==='construct') {
        if(decoding){fail('IPC decoder started before its previous result was delivered');decoderRelease()}
        const eventGroup=group+'-decoder-'+(++decoder.groupsCreated)
        decoding={group:eventGroup,instance:null,channel:null,prototype:null,stage:'construct',constructor:0,header:0,value:0}
        const frame=decoderFrame(params,92,eventGroup)
        if(!frame){decoder.ignoredReceivers++;decoderRelease();continue}
        decoding.channel=evaluate(frame,'channel',eventGroup)
        requireRead(decoding.channel)
        decoder.started++
        decoding.prototype=constructorChain(frame,eventGroup)
        checkSlice(frame,eventGroup)
      }else if(kind==='native:construct') {
        if(!decoding){decoder.unrelatedConstructorEntries++;continue}
        const frames=params.callFrames??[]
        // Both classes have implicit constructors. The actual native entry
        // must come directly through those pinned classes, not a wrapper
        // that restored its constructor chain before delegating.
        for(const [index,line,hash] of [[0,418,v8Source],[1,38,parserSource]]) {
          const frame=frames[index]
          if(!frame||frame.location.lineNumber!==line)throw Error('IPC native constructor has an unexpected caller')
          source(frame.location.scriptId,hash)
        }
        const frame=decoderFrame({...params,callFrames:frames.slice(2)},92,decoding.group)
        if(!frame||!same(evaluate(frame,'channel',decoding.group),decoding.channel))throw Error('IPC native constructor has another channel')
        constructorChain(frame,decoding.group)
        if(decoding.stage!=='construct'||++decoding.constructor!==1)throw Error('IPC native constructor entry is out of order')
        decoder.constructorEntries++
      }else if(kind==='slice:remainder') {
        if(!decoding)throw Error('IPC remainder slice lacks construction evidence')
        const frame=decoderFrame(params,96,decoding.group)
        if(!frame||!same(evaluate(frame,'channel',decoding.group),decoding.channel))throw Error('IPC remainder slice has another channel')
        checkSlice(frame,decoding.group)
      }else if(kind==='header') {
        if(!decoding)throw Error('IPC decoder header lacks construction evidence')
        const frame=decoderFrame(params,101,decoding.group)
        if(!frame||!same(evaluate(frame,'channel',decoding.group),decoding.channel))throw Error('IPC decoder channel changed after construction')
        if(decoding.constructor!==1)throw Error('IPC native constructor entry was not observed exactly once')
        decoding.instance=evaluate(frame,'deserializer',decoding.group)
        if(!same(prototype(decoding.instance),decoding.prototype))throw Error('IPC decoder instance prototype differs from constructor')
        decoding.stage='header'
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
    const literal=post('Runtime.evaluate',{expression:'({})',objectGroup:group,silent:true})
    if(literal.exceptionDetails)throw Error('IPC intrinsic object prototype unavailable')
    objectPrototype=prototype(literal.result)
    const byteNamespace=post('Runtime.evaluate',{expression:'({Uint8Array,Symbol,ownDescriptor:Object.getOwnPropertyDescriptor,buffer:process.getBuiltinModule("buffer")})',objectGroup:group,silent:true})
    if(byteNamespace.exceptionDetails)throw Error('IPC slice namespace unavailable')
    const byteFields=properties(byteNamespace.result).result
    ownDescriptor=data(byteFields,'ownDescriptor')
    const descriptorProperties=properties(ownDescriptor)
    if(ownDescriptor.type!=='function'||descriptorProperties.internalProperties?.some(p=>['[[FunctionLocation]]','[[TargetFunction]]','[[BoundTargetFunction]]'].includes(p.name)))
      throw Error('IPC descriptor baseline is not an unbound native function')
    uint8Constructor=data(byteFields,'Uint8Array');uint8Prototype=data(properties(uint8Constructor).result,'prototype')
    const nativeByteProperties=properties(uint8Constructor)
    if(uint8Constructor.type!=='function'||nativeByteProperties.internalProperties?.some(p=>['[[FunctionLocation]]','[[TargetFunction]]','[[BoundTargetFunction]]'].includes(p.name)))
      throw Error('IPC byte constructor baseline is not an unbound native function')
    speciesSymbol=data(properties(data(byteFields,'Symbol')).result,'species')
    typedSpeciesGetter=descriptor(uint8Constructor,speciesSymbol).get
    const getterProperties=properties(typedSpeciesGetter)
    if(typedSpeciesGetter.type!=='function'||getterProperties.internalProperties?.some(p=>['[[FunctionLocation]]','[[TargetFunction]]','[[BoundTargetFunction]]'].includes(p.name)))
      throw Error('IPC species baseline is not an unbound native getter')
    bufferConstructor=data(properties(data(byteFields,'buffer')).result,'Buffer')
    bufferPrototype=data(properties(bufferConstructor).result,'prototype')
    bufferSpeciesGetter=descriptor(bufferConstructor,speciesSymbol).get
    pinnedFunction(bufferSpeciesGetter,bufferSource,319)
    const fast=post('Runtime.callFunctionOn',{objectId:bufferSpeciesGetter.objectId,functionDeclaration:'function(){return this()}',objectGroup:group,silent:true})
    if(fast.exceptionDetails)throw Error('IPC FastBuffer baseline unavailable')
    fastBuffer=fast.result;pinnedFunction(fastBuffer,internalBufferSource,959)
    if(!same(prototype(fastBuffer),uint8Constructor))throw Error('IPC FastBuffer baseline constructor changed')
    const b=post('Debugger.setBreakpointByUrl',{url:'node:internal/child_process',lineNumber,columnNumber:0})
    breakpointId=b.breakpointId
    if(!breakpointId||b.locations?.length!==1||b.locations[0].lineNumber!==lineNumber)
      throw Error('IPC delivery location is not uniquely loaded')
    source(b.locations[0].scriptId,childSource)
    breakpoints.set(breakpointId,'delivery')
    const readEntry=post('Debugger.setBreakpointByUrl',{url:'node:internal/child_process',lineNumber:637,columnNumber:0})
    breakpoints.set(readEntry.breakpointId,'read:entry')
    if(!readEntry.breakpointId||readEntry.locations?.length!==1||readEntry.locations[0].lineNumber!==637)
      throw Error('IPC read entry is not uniquely loaded')
    source(readEntry.locations[0].scriptId,childSource)
    const possible=post('Debugger.getPossibleBreakpoints',{
      start:{scriptId:b.locations[0].scriptId,lineNumber:637,columnNumber:0},
      end:{scriptId:b.locations[0].scriptId,lineNumber:670,columnNumber:0},restrictToFunction:true})
    const returns=possible.locations.filter(p=>p.type==='return')
    if(returns.length!==1||returns[0].lineNumber!==669)throw Error('IPC read return is not uniquely pinned')
    readReturn=returns[0]
    const readExit=post('Debugger.setBreakpoint',{location:{scriptId:readReturn.scriptId,lineNumber:readReturn.lineNumber,columnNumber:readReturn.columnNumber}})
    breakpoints.set(readExit.breakpointId,'read:return')
    if(!readExit.breakpointId||readExit.actualLocation?.lineNumber!==readReturn.lineNumber||readExit.actualLocation?.columnNumber!==readReturn.columnNumber)
      throw Error('IPC read return breakpoint moved')
    const namespace=post('Runtime.evaluate',{expression:'process.getBuiltinModule("v8")',objectGroup:group,silent:true})
    if(namespace.exceptionDetails)throw Error('IPC decoder namespace unavailable')
    const ctor=data(properties(namespace.result).result,'Deserializer'),proto=data(properties(ctor).result,'prototype')
    nativeConstructor=ctor
    defaultConstructor=data(properties(namespace.result).result,'DefaultDeserializer')
    pinnedFunction(defaultConstructor,v8Source,418)
    const constructorProps=properties(nativeConstructor)
    if(nativeConstructor.type!=='function'||constructorProps.internalProperties?.some(p=>['[[FunctionLocation]]','[[TargetFunction]]','[[BoundTargetFunction]]'].includes(p.name)))
      throw Error('IPC decoder constructor baseline is not an unbound native function')
    const constructorBreakpoint=post('Debugger.setBreakpointOnFunctionCall',{objectId:nativeConstructor.objectId})
    breakpoints.set(constructorBreakpoint.breakpointId,'native:construct')
    for(const name of ['readHeader','readValue']) {
      const fn=data(properties(proto).result,name),p=properties(fn)
      if(fn.type!=='function'||p.internalProperties?.some(p=>['[[FunctionLocation]]','[[TargetFunction]]','[[BoundTargetFunction]]'].includes(p.name)))
        throw Error('IPC decoder baseline is not an unbound native method')
      nativeMethods.set(name,fn)
      const b=post('Debugger.setBreakpointOnFunctionCall',{objectId:fn.objectId})
      breakpoints.set(b.breakpointId,'native:'+name)
    }
    for(const [kind,line] of [['construct',92],['slice:remainder',96],['header',101],['value',102]]) {
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
          const readKind=kinds.find(k=>k.startsWith('read:'))
          if(readKind) {
            if(kinds.length!==1)throw Error('ambiguous IPC read pause')
            readPause(params,readKind)
          }else decoderPause(params,kinds)
        }catch(e){
          fail(e.message)
          const failures=kinds.some(k=>k.startsWith('read:'))?reads.failureFrames:decoder.failureFrames
          if(failures.length<6)failures.push({kinds,reason:e.message,
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
        requireRead(evaluate(frame,'channel',eventGroup));reads.delivered++
        decoded++
        const mode=parser(frame,eventGroup)
        if(mode==='advanced') {
          try {
            if(!decoding||decoding.constructor!==1||decoding.header!==1||decoding.value!==1||decoding.stage!=='value')throw Error('IPC delivery lacks complete decoder entries')
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
    report:()=>({available:installed&&!closed&&!failures&&!pending.length&&!reading,installed,closed,
      decoded,prepared,paired,pending:pending.length,ignoredReceivers,groupsCreated,groupsReleased,baselineReleased,
      parsers:[...parsers.values()],failures,errors:[...errors],replayEligible:false,completeOwnership:false,
      decoder:{...decoder,pending:decoding!==null,initialNativeIntegrity:false,completeOwnership:false},
      reads:{...reads,pending:reading!==null,returnLocation:readReturn,completeOwnership:false},
      slices:{...slices,completeOwnership:false},
      limitation:'Pinned native read callback entry/return, advanced constructor call path, decoder lookup and delivery are checked; initial native integrity, raw-buffer aliases outside that call path, host-object callbacks, JSON decoder ownership and pre-installation activity remain unproven'}),
    close() {
      if(closed)return
      closed=true
      for(const breakpointId of breakpoints.keys())try{post('Debugger.removeBreakpoint',{breakpointId})}catch{fail('IPC breakpoint removal failed')}
      decoderRelease()
      readRelease()
      for(const entry of pending)release(entry.group)
      pending.length=0
      try{post('Runtime.releaseObjectGroup',{objectGroup:group});baselineReleased=true}
      catch{fail('IPC baseline handles could not be released')}
    },
  }
}
