// Isolated experiment: a private native Runtime session shares Node's
// existing Inspector, but unwraps its own handles without protocol calls.
import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const probe=new URL('./native-entry-probe.mjs',import.meta.url).href
const codec=new URL('./native-capture-summary.mjs',import.meta.url).href
const strings=new URL('./native-string-transport.mjs',import.meta.url).href
const compilation=new URL('./worker-module-probe.mjs',import.meta.url).href
const directMode=process.env.QUOD_NATIVE_CAPTURE_TRANSPORT==='string'?'string':'native'
function fixture(mode,body,{timeout=20000}={}) {
  const source=`import {Session} from 'node:inspector';
import {getCallSites,types} from 'node:util';
import {createRequire} from 'node:module';
import {randomBytes} from 'node:crypto';
import vm from 'node:vm';
import {observeNativeEntryArguments} from ${JSON.stringify(probe)};
import {decodeNativeCaptureSummary} from ${JSON.stringify(codec)};
import {nativeStringTransport} from ${JSON.stringify(strings)};
import {createCompiledScriptRecorder} from ${JSON.stringify(compilation)};
const require=createRequire(import.meta.url),session=new Session(),mode=${JSON.stringify(mode)};
const events=[],extra={},commands={},nonce=randomBytes(32).toString('hex');
let summaries=0,cleared=0,pauses=0,compiled=0,observer=null,bridge=null,sourceRecorder=null;
function post(method,params={}) {
 commands[method]=(commands[method]??0)+1;let done=false,error,result;
 session.post(method,params,(e,r)=>{done=true;error=e;result=r});
 if(!done||error)throw error??Error('asynchronous Inspector response');return result;
}
session.connect();session.on('Debugger.scriptParsed',({params})=>{compiled++;sourceRecorder?.record(params)});post('Debugger.enable');
const globalProperties=Reflect.ownKeys(globalThis);
session.on('Debugger.paused',()=>{pauses++;post('Debugger.resume')});
function nativeSummary(packed) {
 const value=decodeNativeCaptureSummary(packed);
 if(!value.owned)return;
 if(!value.available||!value.cleared||!value.metadataValid||value.failure!==0)throw Error('incomplete native summary');
 summaries++;cleared++;
 events.push({api:value.api,kind:['primitive','object','proxy'][value.kind],argumentCount:value.argumentCount,
  receiverBased:value.receiverBased,keys:['seed','tmp','invalidate'].filter((_,i)=>value.mask&(1<<i)),
  excludedOwner:value.excludedOwner,
  frames:getCallSites(100,{sourceMap:false}).map(s=>({name:s.functionName,scriptId:s.scriptId,line:s.lineNumber,column:s.columnNumber}))});
 if(extra.closeInSink)bridge.close();
 if(extra.throwInSink)throw Error('intentional sink failure');
}
if(mode==='string') {
 bridge=nativeStringTransport(post,nonce,{onSummary:nativeSummary});bridge.initialize();
 session.on('Runtime.executionContextCreated',({params})=>bridge.contextCreated(params.context));
 session.on('Runtime.exceptionThrown',({params})=>bridge.handleException(params));
 post('Runtime.enable');
} else if(mode==='native') {
 const addon=require(process.env.QUOD_NATIVE_INSPECTOR_ADDON);
 bridge=addon.createInspectorCapture(nonce,nativeSummary);
 if(typeof bridge.report!=='function')throw Error('native bridge unavailable: '+JSON.stringify(bridge));
} else {
 post('Runtime.enable');
 observer=observeNativeEntryArguments(post,{nativeCapture:true,directArguments:true,ordinaryOwners:true,
  onEntry:e=>{summaries++;if(e.available===false)throw Error('incomplete protocol entry');events.push({api:e.api,kind:e.kind,
   argumentCount:e.argumentCount,receiverBased:e.receiverBased,keys:e.target?.watchedAccessorKeys??[],
   excludedOwner:e.target?.ownerType==='api-wrapper'?1:e.target?.ownerType==='interceptor'?2:0,frames:e.frames});}});
 session.on('Runtime.exceptionThrown',({params})=>observer.handleException(params));
}
const breakpoints=[];
function watch(api,expression,{receiver=false,describeOwnAccessors=true}={}) {
 const fn=post('Runtime.evaluate',{expression,objectGroup:'quod-native-session-fixture'}).result.objectId;
 const condition=mode==='string'?bridge.condition(api,{receiver,describeOwnAccessors}):mode==='native'?
  '(()=>{throw({__proto__:null,quodNativeEntry:'+JSON.stringify(nonce)+',api:'+JSON.stringify(api)+
  ',argument:'+(receiver?'this':'arguments.length?arguments[0]:void(0)')+',argumentCount:arguments.length,receiverBased:'+receiver+',describeOwnAccessors:'+describeOwnAccessors+'})})()':
  observer.condition(api,{receiver,describeOwnAccessors});
 new vm.Script(condition);
 breakpoints.push(post('Debugger.setBreakpointOnFunctionCall',{objectId:fn,condition}).breakpointId);
}
function stop() {
 for(const breakpointId of breakpoints)post('Debugger.removeBreakpoint',{breakpointId});
 breakpoints.length=0;
 if(bridge){extra.bridge=bridge.report();bridge.close();extra.closedBridge=bridge.report();}
 else {extra.capture=observer.report();cleared=extra.capture.cleared;observer.close();}
}
${body}
if(breakpoints.length)stop();
const finalGlobalProperties=Reflect.ownKeys(globalThis);
extra.globalPropertiesUnchanged=globalProperties.length===finalGlobalProperties.length&&globalProperties.every((key,i)=>key===finalGlobalProperties[i]);
post('Runtime.releaseObjectGroup',{objectGroup:'quod-native-session-fixture'});session.disconnect();
console.log(JSON.stringify({mode,events,summaries,cleared,pauses,commands,...extra}));`
  const r=spawnSync(process.execPath,['--expose-gc','--input-type=module','-e',source],{encoding:'utf8',timeout,maxBuffer:16*1024*1024})
  assert.equal(r.status,0,r.stderr||String(r.error))
  return JSON.parse(r.stdout)
}

function complete(data,count) {
  assert.equal(data.summaries,count);assert.equal(data.cleared,count);assert.equal(data.pauses,0)
  assert.equal(data.globalPropertiesUnchanged,true)
  if(data.bridge) {
    assert.equal(data.bridge.enabled,true);assert.equal(data.bridge.failures,0,JSON.stringify(data.bridge))
    assert.equal(data.bridge.failure,null);assert.equal(data.bridge.delivered,count)
    assert.equal(data.bridge.notifications,count);assert.equal(data.closedBridge.closed,true)
    if(data.bridge.transport==='primitive-string') {
      assert.equal(data.bridge.perEntryObjectHandles,0);assert.equal(data.bridge.perEntryProtocolRequests,0)
      assert(data.bridge.contexts>=1)
    }else {
      assert.equal(data.bridge.unwrapped,count);assert.equal(data.bridge.groupsReleased,count)
      assert.equal(data.bridge.verifiedHandleReleases,count)
    }
    assert.equal(data.bridge.replayEligible,false)
  }else {
    assert.equal(data.capture.available,true,JSON.stringify(data.capture));assert.deepEqual(data.capture.errors,[])
  }
}
const withoutFrames=data=>data.events.map(({frames,...row})=>row)

test('native session preserves actual arguments, foreign contexts, proxies and queued microtasks',t=>{
  const rows=[]
  for(const mode of ['protocol',directMode]) {
    const data=fixture(mode,`let getters=0,traps=0,microtasks=0,conversions=0;
const target={get tmp(){getters++;return 7}},plain={tmp:7},key='tmp';
const native=Object.getOwnPropertyDescriptor,lookup=Object.prototype.__lookupGetter__;
const proxy=new Proxy({}, {get(){traps++;throw Error('trap')},getOwnPropertyDescriptor(){traps++;throw Error('trap')}});
watch('Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor');
watch('Object.prototype.__lookupGetter__','Object.prototype.__lookupGetter__',{receiver:true});
watch('util.types.isProxy','process.getBuiltinModule("util").types.isProxy',{describeOwnAccessors:false});
queueMicrotask(()=>microtasks++);
function invoke(decoy,value,k){return native(value,k)}
extra.values=[typeof invoke(null,target,key).get,invoke(target,plain,key).value,
 typeof native.bind(null,target)(key).get,typeof native.apply(null,[target,key]).get,
 typeof lookup.call(target,key),types.isProxy(proxy),types.isProxy(plain),
 typeof vm.runInNewContext('native(value,key)',{native,value:target,key}).get,
 typeof native(target,{toString(){conversions++;return key}}).get];
extra.during=microtasks;stop();await Promise.resolve();extra.after=microtasks;
extra.getters=getters;extra.traps=traps;extra.conversions=conversions;`)
    complete(data,9);assert.equal(data.getters,0);assert.equal(data.traps,0);assert.equal(data.conversions,1)
    assert.equal(data.during,0);assert.equal(data.after,1)
    assert.deepEqual(data.values,['function',7,'function','function','function',true,false,'function','function'])
    assert(data.events.slice(0,2).every(e=>e.frames.some(f=>f.name==='invoke'&&typeof f.scriptId==='string')))
    rows.push(data)
  }
  assert.deepEqual(withoutFrames(rows[1]),withoutFrames(rows[0]))
  t.diagnostic(JSON.stringify({case:'native-session-equivalence',calls:rows[0].summaries,
    nativeBridge:rows[1].bridge,nativeCommands:rows[1].commands,protocolCommands:rows[0].commands,replayEligible:false}))
})

test('native private session leaves another Inspector session usable after close',()=>{
  const data=fixture(directMode,`const native=Object.getOwnPropertyDescriptor,target={get tmp(){throw Error('getter')}};
watch('Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor');native(target,'tmp');stop();
extra.afterClose=post('Runtime.evaluate',{expression:'21*2',returnByValue:true}).result.value;
extra.sourceCount=post('Debugger.getScriptSource',{scriptId:events[0].frames.find(f=>f.name==='nativeSummary').scriptId}).scriptSource.length;
native(target,'tmp');`)
  complete(data,1);assert.equal(data.afterClose,42);assert(data.sourceCount>0)
})

test('native summary callback failure and reentrant close remain explicit observation failures',()=>{
  for(const flag of ['closeInSink','throwInSink']) {
    const data=fixture(directMode,`const native=Object.getOwnPropertyDescriptor,target={tmp:7};
watch('Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor');
extra.${flag}=true;extra.value=native(target,'tmp').value;stop();`)
    assert.equal(data.value,7);assert.equal(data.pauses,0)
    assert(data.bridge.failures>0)
    if(directMode==='native')assert.equal(data.bridge.verifiedHandleReleases,1)
    assert.equal(data.closedBridge.closed,true);assert.equal(data.bridge.replayEligible,false)
  }
})

test('primitive condition exceptions do not retain application argument objects',()=>{
  if(directMode!=='string')return
  const data=fixture('string',`const native=Object.getOwnPropertyDescriptor;
watch('Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor');
function invoke(){const target={get tmp(){return 7}};native(target,'tmp');return new WeakRef(target)}
const reference=invoke();await new Promise(resolve=>setImmediate(resolve));
for(let i=0;i<5;i++)globalThis.gc();extra.collected=reference.deref()===undefined;stop();`)
  complete(data,1);assert.equal(data.collected,true)
})

test('primitive context installation failure remains unavailable after a native call',()=>{
  if(directMode!=='string')return
  const data=fixture('string',`const native=Object.getOwnPropertyDescriptor;
watch('Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor');
bridge.contextCreated({id:99999999,auxData:{isDefault:false}});
extra.value=native({tmp:7},'tmp').value;stop();`)
  assert.equal(data.value,7);assert.equal(data.summaries,1);assert.equal(data.bridge.failures,1)
  assert.equal(data.bridge.failure,'primitive transport context installation failed')
  assert.equal(data.bridge.replayEligible,false);assert.equal(data.closedBridge.closed,true)
})

test('primitive conditions use the exact-source cache without weakening its source restriction',()=>{
  if(directMode!=='string')return
  const data=fixture('string',`const native=Object.getOwnPropertyDescriptor,target={get tmp(){return 7}};
watch('Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor');
sourceRecorder=createCompiledScriptRecorder(scriptId=>post('Debugger.getScriptSource',{scriptId}));
for(let i=0;i<1000;i++)native(target,'tmp');
extra.sourceCache=sourceRecorder.counts();sourceRecorder=null;stop();`)
  complete(data,1000)
  assert.equal(data.sourceCache.policy,'pinned-injective-ascii');assert.equal(data.sourceCache.events,1000)
  assert.equal(data.sourceCache.sourceReads,1);assert.equal(data.sourceCache.cacheHits,999)
  assert.equal(data.sourceCache.entries,1)
})

test('direct native session and protocol transport retain every repeated observation',t=>{
  const rows=[]
  for(const mode of ['protocol',directMode,directMode,'protocol']) {
    const data=fixture(mode,`const native=Object.getOwnPropertyDescriptor,target={get tmp(){throw Error('getter')}},key='tmp',calls=5000;
watch('Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor');
const now=performance.now.bind(performance);now();let values=0;
const commandStart={...commands},compileStart=compiled,started=now();for(let i=0;i<calls;i++)if(typeof native(target,key).get==='function')values++;
extra.elapsedMs=now()-started;extra.compiledEvents=compiled-compileStart;extra.values=values;
extra.entryCommands=Object.fromEntries(Object.entries(commands).map(([name,count])=>[name,count-(commandStart[name]??0)]).filter(([,count])=>count));stop();
extra.frameCounts=events.map(e=>e.frames.length);extra.observations=events.length;
extra.uniform=events.every(e=>e.api==='Object.getOwnPropertyDescriptor'&&e.kind==='object'&&e.argumentCount===2&&e.keys.length===1&&e.keys[0]==='tmp');
events.length=0;`)
    complete(data,5000);assert.equal(data.values,5000);assert.equal(data.observations,5000);assert(data.uniform)
    assert.equal(data.compiledEvents,mode==='protocol'?10000:5000)
    if(mode==='string')assert.deepEqual(data.entryCommands,{})
    assert(data.frameCounts.every(n=>n>0&&n<=100))
    rows.push({mode,elapsedMs:data.elapsedMs,calls:data.summaries,cleared:data.cleared,commands:data.commands,
      bridge:data.bridge??null,compiledEvents:data.compiledEvents,entryCommands:data.entryCommands,
      minimumFrames:Math.min(...data.frameCounts),maximumFrames:Math.max(...data.frameCounts)})
  }
  t.diagnostic(JSON.stringify({nativeInspectorTransportMeasurements:rows,
    scope:'isolated transport, summaries and bounded stacks; not workload or CI savings',replayEligible:false}))
})
