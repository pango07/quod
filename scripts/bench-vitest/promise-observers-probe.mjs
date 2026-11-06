// Diagnostic only: read Node's observer registries through Inspector.
// Never modifies a registry, exposes a reference, or authorizes replay.
// Private runtime shapes are not a supported product contract.
import {Session} from 'node:inspector'
import {createHash} from 'node:crypto'
import {observeNativeCallArguments} from './native-call-argument.mjs'
import {compiledScriptRecord,createCompiledScriptRecorder} from './worker-module-probe.mjs'
import {observeHeapNativeIdentity} from './heap-native-identity-probe.mjs'
import {compiledScriptLog} from './compiled-script-log.mjs'
import {blockCompiledScriptLog} from './compiled-block-log.mjs'
import {observeNativeEntryArguments} from './native-entry-probe.mjs'
import {nativeEntryLog,nativeEntryFrames} from './native-entry-log.mjs'
import {scriptMetadataIndex} from './script-metadata-index.mjs'
const monotonicNow=process.hrtime.bigint
const memoryUsage=process.memoryUsage

// Exact primary Node 24.18.0 source, previously compared with the V8
// scripts in this benchmark. Unknown versions remain unknown. This
// certifies constructor source, not earlier preload effects or native
// addon integrity; it must not authorize reuse on its own.
const nativeSources={
  promises:'6a3d853657112c197bab3b06d1d179840310eb1ec10205ab84d2a2c7eced2dd1',
  utils:'a47d9b1f38bd21771eb4a93b4e1730b8f158ec0fb07097d638fb5436fb945770',
}
const constructedFields=['dev','mode','nlink','uid','gid','rdev','blksize','ino','size','blocks',
  'atimeMs','mtimeMs','ctimeMs','birthtimeMs']

export function inspectPromiseObservers({proxyAccess=false,hookAccess=false,compiledScripts=false,descriptorAccess=false,
  onDescriptorCheckpoint=null,descriptorCheckpointEvery=10000,heapIdentity=false,moduleBindingEvaluation=false,moduleBindingLookup=false,inspectorTiming=false,nativeEntries=false,ordinaryOwners=false}={}) {
  if(ordinaryOwners&&!nativeEntries)throw Error('ordinary owner scope requires native entry observation')
  if(onDescriptorCheckpoint&&(!Number.isSafeInteger(descriptorCheckpointEvery)||descriptorCheckpointEvery<1))throw Error('invalid descriptor checkpoint interval')
  const session=new Session()
  const scripts=nativeEntries?scriptMetadataIndex():new Map(),scriptEvidence=new Map()
  // Repeated condition/capture pairs exceeded four million ordered runs.
  // Intern exact blocks while retaining every event and its position; the
  // four-million-pair bound now applies to the stored representation.
  const compilationLog=nativeEntries?blockCompiledScriptLog():compiledScriptLog(),compilationErrors=[]
  const compilationRecorder=nativeEntries?createCompiledScriptRecorder(scriptId=>post('Debugger.getScriptSource',{scriptId})):null
  let failedToParse=0
  function compiledEvent(params,failedParse) {
    if(!compiledScripts)return
    if(failedParse)failedToParse++
    try{compilationLog.append(compilationRecorder?compilationRecorder.record(params,failedParse):
      compiledScriptRecord(params,post('Debugger.getScriptSource',{scriptId:params.scriptId}),failedParse))}
    catch(e){compilationErrors.push(e.message)}
  }
  session.on('Debugger.scriptParsed',({params})=>{
    try {
      if(scripts.set(params.scriptId,{url:params.url,isModule:params.isModule===true,
        hasSourceURL:params.hasSourceURL===true,language:params.scriptLanguage??'JavaScript'})===false)
        throw Error('script metadata capacity exceeded')
    }catch {if(compilationErrors.length<20)compilationErrors.push('script metadata indexing failed')}
    compiledEvent(params,false)
  })
  session.on('Debugger.scriptFailedToParse',({params})=>compiledEvent(params,true))
  let failure=null, closed=false
  const inspectorBreakpoints=new Map(), inspectorEntries=[]
  let inspectorInitial=null, inspectorFailure=null
  const opaqueBreakpoints=new Map(),opaqueEntries=[]
  const proxyBreakpoints=new Map(),proxyEntries=new Map()
  const nativeProxyLog=nativeEntries?nativeEntryLog():null,nativeDescriptorLog=nativeEntries?nativeEntryLog():null
  let heapIdentityObserver=null
  const proxyFunctions=new Map(),nativeArgument=observeNativeCallArguments(post,
    {moduleBindingEvaluation,moduleBindingLookup,
      identityComparison:heapIdentity?(a,b)=>heapIdentityObserver?.compare(a,b)??{available:false,reason:'native heap identity not ready'}:null})
  let proxyFailure=null,proxyOverflow=false
  const descriptorBreakpoints=new Map(),descriptorFunctions=new Map(),descriptorEntries=new Map()
  let descriptorFailure=null,descriptorOverflow=false
  let descriptorCalls=0
  const descriptorCounts=new Map(),descriptorArgumentCounts=new Map()
  const descriptorNames=['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptors','Reflect.getOwnPropertyDescriptor',
    'Object.defineProperty','Object.defineProperties','Reflect.defineProperty','Reflect.deleteProperty',
    'Object.freeze','Object.seal','Object.preventExtensions','Reflect.preventExtensions',
    'Object.isFrozen','Object.isSealed','Object.setPrototypeOf','Reflect.setPrototypeOf',
    'Object.prototype.__defineGetter__','Object.prototype.__defineSetter__',
    'Object.prototype.__lookupGetter__','Object.prototype.__lookupSetter__']
  const hookBreakpoints=new Map(),hookEntries=new Map(),hookSources={}
  let hookInitial=null,hookFailure=null,hookOverflow=false
  const opaqueNames=['Worker','Worker.startThread','dlopen','linkedBinding','Process.spawn','spawnSync',
    'v8.queryObjects','v8.getHeapSnapshot','v8.writeHeapSnapshot','v8.setFlagsFromString','v8.setHeapSnapshotNearHeapLimit']
  let opaqueInitial=null,opaqueFailure=null
  const commandTiming=new Map()
  const pauseTiming={calls:0,elapsedMs:0}
  const nativeEntryObserver=nativeEntries?observeNativeEntryArguments(post,{targetAccessorsOnly:true,directArguments:true,nativeCapture:true,ordinaryOwners,onEntry(entry) {
    // Resolve every watched frame while it is still live. Retired Inspector
    // source caches may then be bounded without losing deferred report proof.
    // Missing source is an observation failure, never an ownership exemption.
    if(entry.target?.watchedAccessorKeys.length)for(const frame of entry.frames) {
      const evidence=sourceForScript(frame.scriptId)
      if(!evidence.available)descriptorFailure='watched caller source unavailable at native entry'
    }
    const frames=entry.frames.map(f=>({name:f.name,
      location:{scriptId:f.scriptId,lineNumber:f.line-1,columnNumber:f.column-1},script:scripts.get(f.scriptId)??null}))
    const argument={available:entry.available!==false,kind:entry.kind,...(entry.reason?{reason:entry.reason}:{}),...(entry.target?{target:entry.target}:{}),
      actualNativeEntry:true,argumentCount:entry.argumentCount,receiverBased:entry.receiverBased,replayEligible:false}
    if(proxyFunctions.has(entry.api))recordProxy([entry.api],frames,argument)
    else if(descriptorFunctions.has(entry.api))recordDescriptor([entry.api],frames,argument)
    else throw Error('unexpected native argument event')
  }}):null
  function argumentDiagnostics() {
    return nativeEntryObserver?{mode:'native-entry-arguments',...nativeEntryObserver.report()}:nativeArgument.diagnostics()
  }
  function recordProxy(entries,frames,argument) {
    if(nativeProxyLog){nativeProxyLog.append(entries,frames,argument);return}
    const key=JSON.stringify([entries,frames,argument])
    if(proxyEntries.has(key))proxyEntries.get(key).count++
    else if(proxyEntries.size<10000)proxyEntries.set(key,{entries,frames,argument,count:1})
    else proxyOverflow=true
  }
  function recordDescriptor(entries,frames,argument) {
    if(nativeDescriptorLog)nativeDescriptorLog.append(entries,frames,argument)
    else {
      const key=JSON.stringify([entries,frames,argument])
      if(descriptorEntries.has(key))descriptorEntries.get(key).count++
      else if(descriptorEntries.size<10000)descriptorEntries.set(key,{entries,frames,argument,count:1})
      else descriptorOverflow=true
    }
    descriptorCalls++
    for(const name of entries)descriptorCounts.set(name,(descriptorCounts.get(name)??0)+1)
    const kind=argument.available?argument.kind:'unknown: '+argument.reason
    descriptorArgumentCounts.set(kind,(descriptorArgumentCounts.get(kind)??0)+1)
    if(onDescriptorCheckpoint&&descriptorCalls%descriptorCheckpointEvery===0)try {
      onDescriptorCheckpoint({calls:descriptorCalls,apis:Object.fromEntries(descriptorCounts),
        memory:memoryUsage(),
        arguments:Object.fromEntries(descriptorArgumentCounts),last:frames[0],
        overflow:nativeDescriptorLog?.counts().overflow??descriptorOverflow,
        ...(nativeDescriptorLog?{nativeLog:nativeDescriptorLog.counts()}:{}),
        ...(nativeEntryObserver?{stack:frames}:{}),
        compilation:{...compilationLog.counts(),failedToParse,scriptMetadataIndex:scripts.counts?.()??null,
          ...(compilationRecorder?{sourceCache:compilationRecorder.counts()}: {})},argumentDiagnostics:argumentDiagnostics(),
        identity:nativeEntryObserver?{mode:'native-entry-arguments',snapshotsRequested:0}:
          heapIdentityObserver?.report()??{mode:'compiled-comparison'},replayEligible:false})
    }catch{descriptorFailure='descriptor progress checkpoint failed'}
  }
  function post(method,params={}) {
    const started=inspectorTiming?monotonicNow():null
    try {
      let done=false,result,error
      session.post(method,params,(e,r)=>{done=true;error=e;result=r})
      if (!done || error) throw Error(!done ? 'asynchronous inspector response' : 'inspector request failed')
      return result
    }finally {
      if(started!==null) {
        const elapsed=Number(monotonicNow()-started)/1e6
        const row=commandTiming.get(method)??{calls:0,elapsedMs:0}
        row.calls++;row.elapsedMs+=elapsed;commandTiming.set(method,row)
      }
    }
  }
  function properties(id) {
    if (!id) throw Error('missing runtime object')
    return post('Runtime.getProperties',{objectId:id,ownProperties:true,generatePreview:false})
  }
  function prop(id,name) {return properties(id).result.find(p=>p.name===name)?.value}
  function builtin(expression) {
    const r=post('Runtime.evaluate',{expression,objectGroup:'quod-observer-diagnostic',silent:true})
    if (r.exceptionDetails) throw Error('builtin unavailable')
    return r.result.objectId
  }
  function binding(fn,name) {
    const scopes=properties(fn).internalProperties?.find(p=>p.name==='[[Scopes]]')?.value?.objectId
    if (!scopes) throw Error('function scopes unavailable')
    for(const p of properties(scopes).result) {
      if (!/^\d+$/.test(p.name) || p.value?.description==='Global') continue
      const value=prop(p.value?.objectId,name)
      if(value) return value
    }
    throw Error('runtime binding unavailable: '+name)
  }
  function same(a,b) {
    if(!a || !b) return false
    const r=post('Runtime.callFunctionOn',{objectId:a,
      functionDeclaration:'function(other){return this===other}',
      arguments:[{objectId:b}],returnByValue:true,silent:true})
    return !r.exceptionDetails && r.result?.value===true
  }
  function functionSource(fn) {
    const location=properties(fn).internalProperties?.find(p=>p.name==='[[FunctionLocation]]')?.value?.value
    if(!location?.scriptId) return {location:null}
    const source=post('Debugger.getScriptSource',{scriptId:location.scriptId}).scriptSource
    if(typeof source!=='string') throw Error('function source unavailable')
    return {location,script:scripts.get(location.scriptId)??null,
      sourceSha256:createHash('sha256').update(source).digest('hex')}
  }
  function constructionFieldsAbsent(prototype) {
    // A setter inherited before the diagnostic starts can publish the
    // newly created Stats even if the local consumer only reads size.
    // Inspector descriptors do not invoke those setters or other getters.
    const seen=new Set()
    for(let p=prototype;p;) {
      if(seen.has(p) || seen.size>=8) return false
      seen.add(p)
      const props=properties(p)
      if(props.result.some(v=>constructedFields.includes(v.name))) return false
      if(props.internalProperties?.some(v=>['[[Target]]','[[Handler]]'].includes(v.name))) return false
      const next=props.internalProperties?.find(v=>v.name==='[[Prototype]]')?.value
      // V8 omits [[Prototype]] for a null prototype (runtime-debug.cc).
      // Accept that omission only at the realm's intrinsic Object.prototype,
      // obtained from a fresh literal without reading the global Object.
      if(!next) return !!objectPrototype && same(p,objectPrototype)
      if(next.subtype==='null') return true
      if(!next.objectId) return false
      p=next.objectId
    }
    return false
  }
  const kinds=['init','before','after','settled']
  let lists={}, fields=null, indices={}, active=null, owned=null, nativeRefs=null, objectPrototype=null
  function hookSnapshot() {
    if(failure || closed) return {available:false,reason:failure??'closed',replayEligible:false}
    try {
      const promiseHooks={}, identities={}
      for(const kind of kinds) {
        const length=prop(lists[kind],'length')?.value
        if(!Number.isInteger(length) || length<0) throw Error('unknown promise registry shape')
        promiseHooks[kind]=length
        if(owned) {
          const fn=length===1 ? prop(lists[kind],'0') : null
          identities[kind]=false
          if(fn?.type==='function' && fn.objectId) identities[kind]=same(owned[kind],fn.objectId)
        }
      }
      const values=properties(fields).result, asyncHooks={}
      for(const [name,index] of Object.entries(indices)) {
        const value=values.find(p=>p.name===String(index))?.value?.value
        if(!Number.isInteger(value) || value<0) throw Error('unknown async registry shape')
        asyncHooks[name]=value
      }
      const pending=prop(active,'tmp_array')
      return {available:true,promiseHooks,asyncHooks,pendingAsyncHookEdit:pending?.subtype!=='null',
        onlyOwnedPromiseHooks:owned ? kinds.every(k=>identities[k]) : null,replayEligible:false}
    } catch(e) {failure=e.message;return {available:false,reason:failure,replayEligible:false}}
  }
  try {
    session.connect()
    if(nativeEntryObserver) {
      session.on('Runtime.exceptionThrown',({params})=>nativeEntryObserver.handleException(params))
      post('Runtime.enable')
    }
    post('Debugger.enable',nativeEntries&&compiledScripts?{maxScriptsCacheSize:0}:{})
    if(heapIdentity) {
      heapIdentityObserver=observeHeapNativeIdentity(post)
      if(!heapIdentityObserver.report().available)throw Error('native heap identity setup failed')
    }
    session.on('Debugger.paused',({params})=>{
      const started=inspectorTiming?monotonicNow():null
      try {
      const inspector=(params.hitBreakpoints??[]).map(id=>inspectorBreakpoints.get(id)).filter(Boolean)
      const opaque=(params.hitBreakpoints??[]).map(id=>opaqueBreakpoints.get(id)).filter(Boolean)
      const proxy=(params.hitBreakpoints??[]).map(id=>proxyBreakpoints.get(id)).filter(Boolean)
      const hooks=(params.hitBreakpoints??[]).map(id=>hookBreakpoints.get(id)).filter(Boolean)
      const descriptors=(params.hitBreakpoints??[]).map(id=>descriptorBreakpoints.get(id)).filter(Boolean)
      if(inspector.length||opaque.length||proxy.length||hooks.length||descriptors.length) {
        const frames=params.callFrames.slice(0,8).map(f=>({
          name:f.functionName,location:f.location,script:scripts.get(f.location.scriptId)??null}))
        if(inspector.length)inspectorEntries.push({entries:inspector,frames})
        if(opaque.length)opaqueEntries.push({entries:opaque,frames})
        if(hooks.length) {
          const key=JSON.stringify([hooks,frames])
          if(hookEntries.has(key))hookEntries.get(key).count++
          else if(hookEntries.size<10000)hookEntries.set(key,{entries:hooks,frames,count:1})
          else hookOverflow=true
        }
        if(proxy.length) {
          // Count all calls, including the config probe's own isProxy checks.
          // Do not retain arguments, targets or raw values in the diagnostic.
          const argument=nativeArgument(params.callFrames[0],proxyFunctions.get(proxy[0]))
          recordProxy(proxy,frames,argument)
        }
        if(descriptors.length) {
          const name=descriptors[0]
          const argument=name.startsWith('Object.prototype.')?
            {available:false,reason:'receiver-based descriptor API',replayEligible:false}:
            nativeArgument(params.callFrames[0],descriptorFunctions.get(name),{pureAdditionalArguments:true,describeOwnAccessors:true})
          recordDescriptor(descriptors,frames,argument)
        }
        try {post('Debugger.resume')} catch {inspectorFailure=opaqueFailure=proxyFailure=hookFailure=descriptorFailure='could not resume watched entry'}
      } else inspectorFailure=opaqueFailure=proxyFailure=hookFailure=descriptorFailure='unrelated debugger pause'
      }finally {
        if(started!==null) {pauseTiming.calls++;pauseTiming.elapsedMs+=Number(monotonicNow()-started)/1e6}
      }
    })
    // An Inspector connection can inspect a fulfilled promise or heap
    // object without appearing in either JavaScript hook registry. This
    // diagnostic watches the actual native entry functions, including
    // references cached before observation. A public Session wrapper is
    // not enough: process.binding('inspector').Connection is accessible.
    // Existing native connection objects are counted once at startup;
    // queryObjects may collect garbage, so never use it per stat call.
    try {
      const inspector=builtin("process.binding('inspector')")
      const connection=prop(inspector,'Connection')?.objectId
      const prototype=prop(connection,'prototype')?.objectId
      const mainThreadPrototype=prop(prop(inspector,'MainThreadConnection')?.objectId,'prototype')?.objectId
      const sessionPrototype=builtin("process.getBuiltinModule('inspector').Session.prototype")
      const capturedConnection=binding(prop(sessionPrototype,'connect')?.objectId,'Connection')?.objectId
      const capturedPrototype=prop(capturedConnection,'prototype')?.objectId
      if(!prototype||!mainThreadPrototype)throw Error('native inspector prototypes unavailable')
      const objects=post('Runtime.queryObjects',{prototypeObjectId:prototype,
        objectGroup:'quod-observer-diagnostic'}).objects.objectId
      const objectsList=properties(objects).result.filter(p=>/^\d+$/.test(p.name))
      // Exclude the prototype only if its exact identity appears. The
      // census does not establish which objects are live connected
      // sessions, so a raw count must not be used as such a count.
      // Check both prototypes by exact identity. The pinned fixture contains
      // neither; its extra connection-shaped object is not a prototype.
      // No description or assumed baseline count can certify a live session.
      const isPrototype=p=>same(p.value?.objectId,prototype)||same(p.value?.objectId,mainThreadPrototype)
      const prototypeObjects=objectsList.filter(isPrototype).length
      const sessionObjects=post('Runtime.queryObjects',{prototypeObjectId:sessionPrototype,
        objectGroup:'quod-observer-diagnostic'}).objects.objectId
      const sessionConnections=properties(sessionObjects).result.filter(p=>/^\d+$/.test(p.name)).map(p=>
        properties(p.value?.objectId).privateProperties?.find(v=>v.name==='#connection')?.value?.objectId).filter(Boolean)
      inspectorInitial={localConnectionObjects:objectsList.length-prototypeObjects,prototypeObjects,
        sessionConnectionObjects:sessionConnections.length,
        capturedConstructorMatches:same(connection,capturedConnection),
        objects:objectsList.map(p=>({description:p.value?.description,subtype:p.value?.subtype??null,
          properties:properties(p.value?.objectId).result.map(v=>v.name),
          internalProperties:(properties(p.value?.objectId).internalProperties??[]).map(v=>v.name),
          sameAsObjects:objectsList.map(other=>same(p.value?.objectId,other.value?.objectId)),
          prototypeMatchesCaptured:same(properties(p.value?.objectId).internalProperties?.find(v=>v.name==='[[Prototype]]')?.value?.objectId,capturedPrototype),
          isPrototype:isPrototype(p),matchesCapturedPrototype:same(p.value?.objectId,capturedPrototype),
          matchesSessionConnections:sessionConnections.map(c=>same(p.value?.objectId,c))})),
        listenerOpen:post('Runtime.callFunctionOn',{objectId:prop(inspector,'isEnabled').objectId,
          functionDeclaration:'function(){return this()}',returnByValue:true,silent:true}).result?.value}
      if(!Number.isInteger(inspectorInitial.localConnectionObjects) ||
         typeof inspectorInitial.listenerOpen!=='boolean') throw Error('unknown inspector startup state')
      for(const name of ['Connection','MainThreadConnection','open']) {
        const fn=prop(inspector,name)
        if(fn?.type!=='function') throw Error('missing inspector entry: '+name)
        const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:fn.objectId})
        inspectorBreakpoints.set(breakpointId,name)
      }
    } catch(e) {inspectorFailure=e.message}
    try {
      // Observe the actual closed-over native constructor and thread
      // start method, not only the replaceable public Worker export.
      const worker=builtin("process.getBuiltinModule('worker_threads').Worker")
      const nativeWorker=binding(worker,'WorkerImpl').objectId
      const prototype=prop(nativeWorker,'prototype')?.objectId
      const objects=post('Runtime.queryObjects',{prototypeObjectId:prototype,
        objectGroup:'quod-observer-diagnostic'}).objects.objectId
      const rows=properties(objects).result.filter(p=>/^\d+$/.test(p.name))
      const cache=builtin("process.getBuiltinModule('module')._cache")
      const processPrototype=prop(prop(builtin("process.binding('process_wrap')"),'Process')?.objectId,'prototype')?.objectId
      opaqueInitial={nativeWorkerObjects:rows.filter(p=>!same(p.value?.objectId,prototype)).length,
        cachedNativeModules:properties(cache).result.filter(p=>p.name.endsWith('.node')).map(p=>p.name)}
      for(const [name,fn] of [
        ['Worker',nativeWorker],['Worker.startThread',prop(prototype,'startThread')?.objectId],
        ['dlopen',builtin('process.dlopen')],['linkedBinding',builtin('process._linkedBinding')],
        ['Process.spawn',prop(processPrototype,'spawn')?.objectId],
        ['spawnSync',prop(builtin("process.binding('spawn_sync')"),'spawn')?.objectId],
        // Public V8 heap APIs do not open Inspector connections. Watch
        // their actual native bindings so a cached wrapper remains visible.
        // Resolving these private names is version-specific; missing shapes
        // make the entire opaque observation unavailable, never clean.
        ...[
          ['queryObjects','_queryObjects'],['getHeapSnapshot','createHeapSnapshotStream'],
          ['writeHeapSnapshot','triggerHeapSnapshot'],['setFlagsFromString','_setFlagsFromString'],
          ['setHeapSnapshotNearHeapLimit','_setHeapSnapshotNearHeapLimit'],
        ].map(([name,native])=>['v8.'+name,binding(builtin(`process.getBuiltinModule('v8').${name}`),native).objectId]),
      ]) {
        if(!fn)throw Error('missing opaque entry: '+name)
        const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:fn})
        opaqueBreakpoints.set(breakpointId,name)
      }
    } catch(e) {opaqueFailure=e.message}
    objectPrototype=properties(builtin('({})')).internalProperties?.find(p=>p.name==='[[Prototype]]')?.value?.objectId
    if(!objectPrototype) throw Error('intrinsic object prototype unavailable')
    for(const kind of kinds) {
      const name='on'+kind[0].toUpperCase()+kind.slice(1)
      lists[kind]=binding(builtin(`process.getBuiltinModule('v8').promiseHooks.${name}`),'list').objectId
    }
    const publicCreate=builtin("process.getBuiltinModule('async_hooks').createHook")
    const getHooks=binding(publicCreate,'getHookArrays').objectId
    fields=binding(getHooks,'async_hook_fields').objectId
    active=binding(getHooks,'active_hooks').objectId
    for(const name of ['kInit','kBefore','kAfter','kDestroy','kPromiseResolve','kTotals']) {
      // kTotals is used by the public AsyncHook enable/disable methods,
      // and V8 may omit it from the internal function's lexical context.
      // Read the actual binding; never assume a numeric field offset.
      const index=binding(publicCreate,name).value
      if(!Number.isInteger(index)) throw Error('unknown hook field index')
      indices[name]=index
    }
    if(proxyAccess)try {
      // Node's native getProxyDetails unwraps a Proxy without invoking its
      // traps; isProxy exposes the instrumentation itself. Observe actual
      // function identities so references cached before observation count.
      // This covers these two entries only, not every native observation.
      for(const [name,fn] of [
        ['util.types.isProxy',builtin("process.getBuiltinModule('util').types.isProxy")],
        ['util.getProxyDetails',binding(builtin("process.getBuiltinModule('util').inspect"),'getProxyDetails').objectId],
      ]) {
        if(!fn)throw Error('missing proxy observation entry: '+name)
        proxyFunctions.set(name,fn)
        const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:fn,
          ...(nativeEntryObserver?{condition:nativeEntryObserver.condition(name)}:{})})
        proxyBreakpoints.set(breakpointId,name)
      }
    }catch(e){proxyFailure=e.message}
    if(hookAccess)try {
      // A hook can capture a raw Promise result and disappear before a final
      // registry snapshot. Watch its registration path and native installation
      // for the entire diagnostic lifetime, including cached references.
      hookInitial=hookSnapshot()
      if(!hookInitial.available)throw Error('initial hook registry unavailable')
      const onInit=builtin("process.getBuiltinModule('v8').promiseHooks.onInit")
      const update=binding(onInit,'update').objectId
      const asyncHook=binding(publicCreate,'AsyncHook').objectId
      const asyncPrototype=prop(asyncHook,'prototype')?.objectId
      const asyncWrap=binding(getHooks,'async_wrap').objectId
      const resource=builtin("process.getBuiltinModule('async_hooks').executionAsyncResource")
      // V8 installs the function-call breakpoint at their shared code
      // location: the four registration closures cannot get separate ones.
      // Keep this a grouped observation, rather than mislabeling every call
      // as onInit. Fixtures exercise each closure and cached references.
      const registrationSource=functionSource(onInit)
      if(!registrationSource.location)throw Error('promise registration source unavailable')
      for(const kind of kinds) {
        const name='on'+kind[0].toUpperCase()+kind.slice(1)
        const source=functionSource(builtin(`process.getBuiltinModule('v8').promiseHooks.${name}`))
        if(JSON.stringify(source)!==JSON.stringify(registrationSource))throw Error('promise registration source differs')
      }
      for(const [name,fn] of [
        ['promiseHooks.register',onInit],
        ['async_wrap.setPromiseHooks',binding(update,'setPromiseHooks').objectId],
        ['AsyncHook.enable',prop(asyncPrototype,'enable')?.objectId],
        ['AsyncHook.disable',prop(asyncPrototype,'disable')?.objectId],
        ['async_hooks.executionAsyncResource',resource],
        ['async_wrap.executionAsyncResource',binding(resource,'executionAsyncResource_').objectId],
        ['async_wrap.setupHooks',prop(asyncWrap,'setupHooks')?.objectId],
        ['async_wrap.setCallbackTrampoline',prop(asyncWrap,'setCallbackTrampoline')?.objectId],
        ['process.binding',builtin('process.binding')],
      ]) {
        if(!fn)throw Error('missing hook observation entry: '+name)
        hookSources[name]=functionSource(fn)
        const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:fn})
        hookBreakpoints.set(breakpointId,name)
      }
    }catch(e){hookFailure=e.message}
    if(descriptorAccess)try {
      // Native function breakpoints also observe aliases captured before
      // installation. No replacement of Object/Reflect exports is required.
      // This remains a finite API census, not complete descriptor coverage.
      for(const name of descriptorNames) {
        const fn=builtin(name)
        if(!fn)throw Error('missing descriptor entry: '+name)
        descriptorFunctions.set(name,fn)
        const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:fn,
          ...(nativeEntryObserver?{condition:nativeEntryObserver.condition(name,
            {receiver:name.startsWith('Object.prototype.'),describeOwnAccessors:true})}:{})})
        descriptorBreakpoints.set(breakpointId,name)
      }
    }catch(e){descriptorFailure=e.message}
  } catch(e) {failure=e.message}
  function sourceForScript(scriptId) {
    // A filename or a Function#toString result can be forged. Resolve the
    // actual V8 script identity from a native call site in this session.
    if(failure||closed)return {available:false,reason:failure??'closed',replayEligible:false}
    try {
      if(typeof scriptId!=='string'||!scripts.has(scriptId))throw Error('unknown script identity')
      if(scriptEvidence.has(scriptId))return scriptEvidence.get(scriptId)
      if(scriptEvidence.size>=20000)throw Error('script evidence capacity exceeded')
      const script=scripts.get(scriptId)
      if(script.language!=='JavaScript')throw Error('non-JavaScript read site')
      const source=post('Debugger.getScriptSource',{scriptId}).scriptSource
      if(typeof source!=='string')throw Error('script source unavailable')
      const evidence={available:true,scriptId,script:{...script},code:{encoding:'utf16le',
        bytes:Buffer.byteLength(source,'utf16le'),sha256:createHash('sha256').update(source,'utf16le').digest('hex')},
        replayEligible:false}
      scriptEvidence.set(scriptId,evidence)
      return evidence
    }catch(e){return {available:false,reason:e.message,replayEligible:false}}
  }
  return {
    compiledScriptReport() {
      return {enabled:compiledScripts,replayEligible:false,completeCompilationCoverage:false,
        ...compilationLog.report(),errors:[...compilationErrors],failedToParse,
        debuggerScriptCacheBytes:nativeEntries&&compiledScripts?0:null,
        ...(compilationRecorder?{sourceCache:compilationRecorder.counts()}:{}),
        ...(nativeEntries?{scriptMetadataIndex:scripts.counts()}:{}),
        limitation:'Debugger-reported scripts can omit unbound compilations and do not certify complete worker inputs'}
    },
    sourceForScript,
    hookAccess() {
      const current=hookSnapshot()
      return {available:hookAccess&&!failure&&!hookFailure&&current.available&&hookBreakpoints.size===9,
        initial:hookInitial,current,entries:[...hookEntries.values()],sources:hookSources,
        overflow:hookOverflow,reason:failure??hookFailure,replayEligible:false,
        limitation:'Observed hook registrations, async resource access and private binding entry; not complete raw-object ownership or native capability exclusion'}
    },
    proxyAccess() {
      return {available:proxyAccess&&!failure&&!proxyFailure&&proxyBreakpoints.size===2&&(!nativeEntryObserver||nativeEntryObserver.report().available),
        ...(nativeProxyLog?.report()??{entries:[...proxyEntries.values()].map(e=>({...e})),overflow:proxyOverflow}),
        reason:failure??proxyFailure,argumentDiagnostics:argumentDiagnostics(),replayEligible:false,
        limitation:'Observes isProxy and getProxyDetails with bounded caller argument diagnostics; other native observers are not excluded'}
    },
    descriptorAccess() {
      const log=nativeDescriptorLog?.report()??{entries:[...descriptorEntries.values()],calls:descriptorCalls,overflow:descriptorOverflow}
      let sourceEvidence
      if(nativeDescriptorLog) {
        const ids=new Set()
        for(const entry of log.entries)if(entry.argument.target?.watchedAccessorKeys.length)
          for(const frame of nativeEntryFrames(log,entry))ids.add(frame.location?.scriptId)
        const records=[...ids].map(scriptId=>sourceForScript(scriptId))
        sourceEvidence={scope:'frames-of-watched-accessor-targets',collection:'at-native-entry',
          available:records.every(r=>r.available),records,replayEligible:false}
      }
      return {available:descriptorAccess&&!failure&&!descriptorFailure&&descriptorBreakpoints.size===descriptorNames.length&&(!nativeEntryObserver||nativeEntryObserver.report().available),
        ...log,...(sourceEvidence?{sourceEvidence}:{}),reason:failure??descriptorFailure,
        argumentDiagnostics:argumentDiagnostics(),
        replayEligible:false,completeDescriptorCoverage:false,
        limitation:'Listed native APIs only; syntax deletion, cross-realm intrinsics, native inspection and earlier copies are not excluded'}
    },
    nativeIdentity() {
      return nativeEntryObserver?{mode:'native-entry-arguments',snapshotsRequested:0,...nativeEntryObserver.report()}:
        heapIdentityObserver?.report()??{mode:'compiled-comparison',replayEligible:false}
    },
    inspectorTiming() {
      return {enabled:inspectorTiming,pauses:{...pauseTiming},commands:Object.fromEntries([...commandTiming].map(([method,row])=>[method,{...row}])),
        replayEligible:false,limitation:'Inclusive command timings can overlap through synchronous notifications'}
    },
    opaqueSnapshot() {
      return {available:!opaqueFailure&&!!opaqueInitial,initial:opaqueInitial,
        counts:Object.fromEntries(opaqueNames.map(name=>
          [name,opaqueEntries.filter(e=>e.entries.includes(name)).length])),
        reason:opaqueFailure,replayEligible:false}
    },
    opaqueAccess() {
      let cachedNativeModules=[]
      try {cachedNativeModules=properties(builtin("process.getBuiltinModule('module')._cache")).result
        .filter(p=>p.name.endsWith('.node')).map(p=>p.name)}catch{}
      return {available:!opaqueFailure&&!!opaqueInitial,initial:opaqueInitial,
        entries:opaqueEntries.map(e=>({...e})),cachedNativeModules,reason:opaqueFailure,replayEligible:false}
    },
    inspectorAccess() {
      // These are observations, not an exhaustive exclusion certificate:
      // worker/native-addon access and initial runtime provenance still
      // require independent checks. Even a zero-entry result grants no key.
      return {available:!inspectorFailure && !!inspectorInitial,initial:inspectorInitial,
        entries:inspectorEntries.map(e=>({...e})),reason:inspectorFailure,replayEligible:false}
    },
    nativeStatProvenance() {
      // Capture before wrapping fs. Do not equate matching names/source
      // with native provenance: these are diagnostic facts for review.
      try {
        const stat=builtin("process.getBuiltinModule('fs').promises.stat")
        const convert=binding(stat,'getStatsFromBinding').objectId
        const ctor=binding(convert,'Stats').objectId
        const base=binding(ctor,'StatsBase').objectId
        const native=binding(stat,'binding').objectId
        const nativeStat=prop(native,'stat')
        const publicCtor=builtin("process.getBuiltinModule('fs').Stats")
        const prototype=prop(ctor,'prototype')?.objectId
        if(!prototype || nativeStat?.type!=='function') throw Error('native stat shape unavailable')
        nativeRefs={stat,convert,ctor,base,native,fn:nativeStat.objectId,prototype}
        const statSource=functionSource(stat),convertSource=functionSource(convert),ctorSource=functionSource(ctor),baseSource=functionSource(base)
        const knownSources=statSource.sourceSha256===nativeSources.promises &&
          [convertSource,ctorSource,baseSource].every(s=>s.sourceSha256===nativeSources.utils &&
            s.location?.scriptId===convertSource.location?.scriptId && s.script?.hasSourceURL===false)
        return {available:true,stat:statSource,convert:convertSource,constructor:ctorSource,baseConstructor:baseSource,
          construction:{knownSources,fieldsAbsent:constructionFieldsAbsent(prototype),replayEligible:false},
          publicConstructor:functionSource(publicCtor),publicConstructorMatches:same(ctor,publicCtor),
          publicPrototypeMatches:same(prop(ctor,'prototype')?.objectId,prop(publicCtor,'prototype')?.objectId),
          bindingStat:functionSource(nativeStat?.objectId),
          bindingStatDescriptionSha256:createHash('sha256').update(nativeStat?.description??'').digest('hex'),
          replayEligible:false}
      } catch(e) {return {available:false,reason:e.message,replayEligible:false}}
    },
    nativeStatSnapshot() {
      if(failure || closed || !nativeRefs) return {available:false,unchanged:false,replayEligible:false}
      try {
        const n=nativeRefs
        // The public promise function can remain unchanged while its
        // closed-over binding's stat method is replaced. Compare actual
        // identities; names and native-looking text are insufficient.
        const unchanged=same(binding(n.stat,'getStatsFromBinding').objectId,n.convert) &&
          same(binding(n.convert,'Stats').objectId,n.ctor) &&
          same(binding(n.ctor,'StatsBase').objectId,n.base) &&
          same(binding(n.stat,'binding').objectId,n.native) &&
          same(prop(n.native,'stat')?.objectId,n.fn) &&
          same(prop(n.ctor,'prototype')?.objectId,n.prototype)
        return {available:true,unchanged,constructionFieldsAbsent:constructionFieldsAbsent(n.prototype),replayEligible:false}
      } catch(e) {return {available:false,unchanged:false,reason:e.message,replayEligible:false}}
    },
    promiseOutcome(kind) {
      // The owned hook keeps this short-lived reference in its private
      // lexical context. No global bridge, replacement Stats or promise
      // reaction is needed to inspect the native result before its await
      // consumer starts. Merely resolving a promise is too early: the
      // settled hook may still see a pending internal state.
      if(failure || closed || !owned || !nativeRefs) return {available:false,replayEligible:false}
      const transient=new Set()
      try {
        const context=binding(owned[kind],'checkpoint').objectId
        const promise=prop(context,'promise')?.objectId
        if(!promise) throw Error('owned promise reference unavailable')
        transient.add(promise)
        const internal=properties(promise).internalProperties
        const result=internal?.find(p=>p.name==='[[PromiseResult]]')?.value
        if(result?.objectId)transient.add(result.objectId)
        const state=internal?.find(p=>p.name==='[[PromiseState]]')?.value?.value
        if(state==='rejected') return {available:true,state,projection:'ignored-error',replayEligible:false}
        if(state!=='fulfilled') return {available:false,state:state??'unknown',replayEligible:false}
        if(result?.type!=='object' || !result.objectId) throw Error('non-object stat result')
        const p=properties(result.objectId)
        const prototype=p.internalProperties?.find(p=>p.name==='[[Prototype]]')?.value?.objectId
        const size=p.result.find(p=>p.name==='size')
        if(!same(prototype,nativeRefs.prototype) || size?.get || size?.set || size?.value?.type!=='number' ||
          !Number.isSafeInteger(size.value.value) || size.value.value<0) throw Error('unknown stat result shape')
        return {available:true,state,projection:'size',size:size.value.value,replayEligible:false}
      } catch(e) {return {available:false,reason:e.message,replayEligible:false}}
      finally {
        // Runtime.getProperties creates strong remote handles. Releasing
        // only the diagnostic session at exit would keep the native
        // promise and its Stats/Error observable after user code drops it.
        for(const objectId of transient)try {post('Runtime.releaseObject',{objectId})}catch{}
      }
    },
    captureOwned() {
      if(failure || closed || owned) return false
      try {
        const refs={}
        for(const kind of kinds) {
          if(prop(lists[kind],'length')?.value!==1) return false
          const fn=prop(lists[kind],'0')
          if(fn?.type!=='function' || !fn.objectId) return false
          refs[kind]=fn.objectId
        }
        owned=refs
        return true
      } catch(e) {failure=e.message;return false}
    },
    snapshot:hookSnapshot,
    close() {
      closed=true
      heapIdentityObserver?.close()
      for(const breakpointId of inspectorBreakpoints.keys()) {
        try {post('Debugger.removeBreakpoint',{breakpointId})} catch {}
      }
      for(const breakpointId of opaqueBreakpoints.keys()) {
        try {post('Debugger.removeBreakpoint',{breakpointId})} catch {}
      }
      for(const breakpointId of proxyBreakpoints.keys()) {
        try {post('Debugger.removeBreakpoint',{breakpointId})} catch {}
      }
      for(const breakpointId of hookBreakpoints.keys()) {
        try {post('Debugger.removeBreakpoint',{breakpointId})} catch {}
      }
      for(const breakpointId of descriptorBreakpoints.keys()) {
        try {post('Debugger.removeBreakpoint',{breakpointId})} catch {}
      }
      nativeEntryObserver?.close()
      try {post('Runtime.releaseObjectGroup',{objectGroup:'quod-observer-diagnostic'})} catch {}
      try {session.disconnect()} catch {}
    },
  }
}
