// Diagnostic experiment only. No production key or replay decision uses this.
import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
const probe=new URL('./native-entry-probe.mjs',import.meta.url).href

function fixture(body,{ballast=0,gc=false,handleGroups=false,targetAccessorsOnly=false,inProcessInspection=false}={}) {
  const source=`import {Session} from 'node:inspector';
import vm from 'node:vm';
import {types} from 'node:util';
import {observeNativeEntryArguments} from ${JSON.stringify(probe)};
${Array.from({length:ballast},(_,i)=>`export const unused${i}=${i};`).join('\n')}
const session=new Session(),errors=[],pauses=[],events=[],extra={},commands=Object.create(null);
function post(method,params={}){
 commands[method]=(commands[method]??0)+1;
 let done=false,error,result;
 session.post(method,params,(e,r)=>{done=true;error=e;result=r});
 if(!done||error)throw error??Error('asynchronous inspector response');return result;
}
session.connect();post('Runtime.enable');post('Debugger.enable');
session.on('Debugger.paused',({params})=>{pauses.push(params.hitBreakpoints);post('Debugger.resume')});
let gets=0,memberGets=0;
const target={get tmp(){gets++;return 'not-read'}},plain={tmp:7},key='tmp',native=Object.getOwnPropertyDescriptor;
globalThis.__quodEntryFixture={target,plain};
const targetId=post('Runtime.evaluate',{expression:'globalThis.__quodEntryFixture.target',objectGroup:'quod-entry-fixture'}).result.objectId;
const watched=post('Runtime.evaluate',{expression:'Object.getOwnPropertyDescriptor',objectGroup:'quod-entry-fixture'}).result.objectId;
function same(id,other){
 if(!id)return false;
 const r=post('Runtime.callFunctionOn',{objectId:id,functionDeclaration:'function(other){return this===other}',arguments:[{objectId:other}],returnByValue:true,silent:true});
 if(r.exceptionDetails)throw Error('identity comparison failed');return r.result.value;
}
function props(id){return post('Runtime.getProperties',{objectId:id,ownProperties:true,generatePreview:false}).result}
function captureEntries(registrations,onEntry=entry=>events.push(entry)) {
 const capture=observeNativeEntryArguments(post,{onEntry,handleGroups:${JSON.stringify(handleGroups)},targetAccessorsOnly:${JSON.stringify(targetAccessorsOnly)},inProcessInspection:${JSON.stringify(inProcessInspection)}});
 const listener=({params})=>capture.handleException(params);session.on('Runtime.exceptionThrown',listener);
 const breakpoints=registrations.map(([api,expression,options])=>{
  const fn=post('Runtime.evaluate',{expression,objectGroup:'quod-entry-fixture'}).result.objectId;
  return post('Debugger.setBreakpointOnFunctionCall',{objectId:fn,condition:capture.condition(api,options)}).breakpointId;
 });
 return ()=>{for(const breakpointId of breakpoints)post('Debugger.removeBreakpoint',{breakpointId});
   session.removeListener('Runtime.exceptionThrown',listener);extra.capture=capture.report();capture.close()};
}
${body}
post('Runtime.releaseObjectGroup',{objectGroup:'quod-entry-fixture'});session.disconnect();delete globalThis.__quodEntryFixture;
console.log(JSON.stringify({gets,memberGets,pauses:pauses.length,events,errors,...extra}));`
  const r=spawnSync(process.execPath,[...(gc?['--expose-gc']:[]),'--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error))
  const data=JSON.parse(r.stdout)
  assert.equal(data.gets,0);assert.deepEqual(data.errors,[])
  return data
}

function complete(data,calls) {
  assert.equal(data.pauses,0);assert.equal(data.capture.available,true,JSON.stringify(data.capture))
  assert.equal(data.capture.calls,calls);assert.equal(data.capture.cleared,calls)
  assert.deepEqual(data.capture.errors,[]);assert.equal(data.capture.replayEligible,false)
}

test('native entry conditions see actual arguments through wrappers, binding and replacing getters',t=>{
  const data=fixture(`const condition="arguments.length===2 && arguments[0]===globalThis.__quodEntryFixture.target && arguments[1]==='tmp'";
const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:watched,condition});
function invoke(decoy,key,value){return native(value,key)}
invoke('not-the-target',key,target);invoke('not-the-target',key,plain);
native.bind(null,target)(key);
const owner={get call(){memberGets++;Object.defineProperty(this,'call',{value:native});return native.bind(null,target)}};
owner.call(key);native.apply(null,[target,key]);
post('Debugger.removeBreakpoint',{breakpointId});`)
  assert.equal(data.memberGets,1)
  assert.equal(data.pauses,4,'only calls whose actual first argument is target should pause')
  t.diagnostic(JSON.stringify({case:'actual-native-arguments',...data,replayEligible:false}))
})

test('condition exceptions expose native arguments without pausing or reaching application exception handlers',t=>{
  const data=fixture(`let applicationExceptions=0;
process.on('uncaughtException',()=>applicationExceptions++);
session.on('Runtime.exceptionThrown',({params})=>{
 try {
  const exception=params.exceptionDetails.exception;
  const fields=props(exception.objectId),kind=fields.find(p=>p.name==='kind')?.value?.value;
  if(kind!=='quod-native-entry-fixture')throw Error('unexpected condition exception');
  const args=fields.find(p=>p.name==='args')?.value?.objectId,values=props(args);
  const first=values.find(p=>p.name==='0')?.value,second=values.find(p=>p.name==='1')?.value;
  events.push({length:values.find(p=>p.name==='length')?.value?.value,target:same(first?.objectId,targetId),key:second?.value,
    exceptionScript:params.exceptionDetails.scriptId??null,stackFrames:params.exceptionDetails.stackTrace?.callFrames?.length??0});
  // Release handles from the event; never print or retain application values.
  const ids=new Set([exception.objectId,args,...fields.map(p=>p.value?.objectId),...values.map(p=>p.value?.objectId)]);
  for(const objectId of ids)if(objectId)post('Runtime.releaseObject',{objectId});
 }catch(e){errors.push(e.message)}
});
const condition='(()=>{throw {__proto__:null,kind:"quod-native-entry-fixture",args:arguments,receiver:this}})()';
const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:watched,condition});
function invoke(decoy,key,value){return native(value,key)}
const results=[invoke('not-the-target',key,target),invoke('not-the-target',key,plain),native.bind(null,target)(key)];
const owner={get call(){memberGets++;Object.defineProperty(this,'call',{value:native});return native.bind(null,target)}};
results.push(owner.call(key),native.apply(null,[target,key]));
results.push(vm.runInNewContext('native(value,key)',{native,value:target,key}));
post('Debugger.removeBreakpoint',{breakpointId});
if(applicationExceptions!==0||results.filter(r=>typeof r.get==='function').length!==5||results[1].value!==7)throw Error('application behavior changed');`)
  assert.equal(data.memberGets,1);assert.equal(data.pauses,0)
  assert.equal(data.events.length,6)
  assert.deepEqual(data.events.map(e=>e.target),[true,false,true,true,true,true])
  assert(data.events.every(e=>e.length===2&&e.key==='tmp'))
  t.diagnostic(JSON.stringify({case:'non-pausing-native-arguments',...data,replayEligible:false}))
})

for(const inProcessInspection of [false,true])test('actual native capture handles C++ proxy checks and receiver-based descriptor APIs: '+inProcessInspection,()=>{
  const data=fixture(`let traps=0;const proxy=new Proxy({}, {get(){traps++;throw Error('trap')},getOwnPropertyDescriptor(){traps++;throw Error('trap')}});
const isProxy=types.isProxy,lookup=Object.prototype.__lookupGetter__;
const stop=captureEntries([
 ['util.types.isProxy','process.getBuiltinModule("util").types.isProxy',{}],
 ['Object.prototype.__lookupGetter__','Object.prototype.__lookupGetter__',{receiver:true,describeOwnAccessors:true}]
]);
extra.values=[isProxy(proxy),isProxy(plain),typeof lookup.call(target,key),typeof lookup.bind(target)(key)];
stop();extra.traps=traps;`,{inProcessInspection})
  complete(data,4);assert.equal(data.traps,0)
  assert.deepEqual(data.values,[true,false,'function','function'])
  assert.deepEqual(data.events.map(e=>e.kind),['proxy','object','object','object'])
  assert(data.events.slice(2).every(e=>e.receiverBased&&e.target.watchedAccessorKeys.includes('tmp')))
})

for(const inProcessInspection of [false,true])test('native capture preserves arguments and queued microtasks: '+inProcessInspection,()=>{
  const data=fixture(`let keyConversions=0,microtasks=0;const objectKey={toString(){keyConversions++;return key}};
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
let current=target;queueMicrotask(()=>{current=plain;microtasks++});
function originalCaller(value,k){return native(value,k)}
const a=originalCaller(current,objectKey);current=plain;const b=originalCaller(current,key);
extra.during=microtasks;extra.values=[typeof a.get,b.value];extra.keyConversions=keyConversions;
stop();await Promise.resolve();extra.after=microtasks;`,{inProcessInspection})
  complete(data,2);assert.equal(data.during,0);assert.equal(data.after,1)
  assert.equal(data.keyConversions,1);assert.deepEqual(data.values,['function',7])
  assert.deepEqual(data.events.map(e=>e.target.watchedAccessorKeys),[['tmp'],[]])
  assert(data.events.every(e=>e.frames.some(f=>f.name==='originalCaller'&&typeof f.scriptId==='string')),
    'native call-site capture must preserve the actual caller without materializing lexical scopes')
})

for(const inProcessInspection of [false,true])test('clearing owned exception wrappers does not retain application arguments: '+inProcessInspection,()=>{
  const data=fixture(`const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
function temporary(){const value={tmp:1};const weak=new WeakRef(value);native(value,key);return weak}
const weak=temporary();stop();
await new Promise(resolve=>setImmediate(resolve));
for(let i=0;i<4;i++)globalThis.gc();extra.collected=weak.deref()===undefined;`,{gc:true,inProcessInspection})
  complete(data,1);assert.equal(data.collected,true,'the Inspector console must retain only a cleared diagnostic wrapper')
})

test('private event groups preserve actual arguments, receivers, errors and queued work',()=>{
  const data=fixture(`let microtasks=0,traps=0,failed=false;
const proxy=new Proxy({}, {getOwnPropertyDescriptor(){traps++;return undefined}});
const lookup=Object.prototype.__lookupGetter__,isProxy=types.isProxy;
const stop=captureEntries([
 ['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}],
 ['Object.prototype.__lookupGetter__','Object.prototype.__lookupGetter__',{receiver:true,describeOwnAccessors:true}],
 ['util.types.isProxy','process.getBuiltinModule("util").types.isProxy',{}]
]);
queueMicrotask(()=>microtasks++);
const owner={get call(){memberGets++;Object.defineProperty(this,'call',{value:native});return native.bind(null,target)}};
extra.values=[typeof owner.call(key).get,typeof lookup.bind(target)(key),isProxy(proxy)];
native(proxy,key);try{native()}catch(e){failed=e instanceof TypeError}
extra.during=microtasks;stop();await Promise.resolve();extra.after=microtasks;
extra.traps=traps;extra.failed=failed;extra.originalHandleSurvives=same(targetId,targetId);`,{handleGroups:true})
  complete(data,5);assert.equal(data.capture.groupsReleased,5)
  assert.equal(data.capture.handleMode,'private-event-group')
  assert.equal(data.memberGets,1);assert.equal(data.gets,0);assert.equal(data.traps,1)
  assert.equal(data.during,0);assert.equal(data.after,1);assert(data.failed&&data.originalHandleSurvives)
  assert.deepEqual(data.values,['function','function',true])
  assert.deepEqual(data.events.map(e=>e.kind),['object','object','proxy','proxy','primitive'])
  assert(data.events.slice(0,2).every(e=>e.target.watchedAccessorKeys.includes('tmp')))
})

test('group cleanup releases descriptors and internal properties without releasing unrelated handles',()=>{
  const data=fixture(`const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
function temporary(){
 const captured={tmp:1};const weak=new WeakRef(captured);
 const value={get tmp(){return captured}};
 native(value,key);return [new WeakRef(value),weak];
}
const weak=temporary();stop();extra.originalHandleSurvives=same(targetId,targetId);
await new Promise(resolve=>setImmediate(resolve));for(let i=0;i<4;i++)globalThis.gc();
extra.collected=weak.every(w=>w.deref()===undefined);`,{gc:true,handleGroups:true})
  complete(data,1);assert.equal(data.capture.groupsReleased,1)
  assert(data.collected&&data.originalHandleSurvives)
})

for(const inProcessInspection of [false,true])test('accessor-only target inspection preserves own accessor evidence: '+inProcessInspection,()=>{
  const data=fixture(`let reads=0;
const proto={get seed(){reads++;throw Error('inherited accessor')}};
const value=Object.create(proto);
Object.defineProperties(value,{tmp:{get(){reads++;throw Error('own getter')}},invalidate:{set(v){reads++;}},
 seed:{value:123},data:{value:()=>42}});
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
extra.returned=typeof native(value,key).get;stop();extra.reads=reads;`,{targetAccessorsOnly:true,inProcessInspection})
  complete(data,1);assert.equal(data.reads,0);assert.equal(data.returned,'function')
  assert.equal(data.events[0].target.ownAccessorCount,2)
  assert.deepEqual(data.events[0].target.watchedAccessorKeys.sort(),['invalidate','tmp'])
})

test('private summaries preserve replacing bound callees and add no public global bridge',()=>{
  const data=fixture(`const before=Reflect.ownKeys(globalThis);
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
let owner={get call(){memberGets++;owner={call:native};return native.bind(null,target)}};
const a=owner.call(key);let current=target;const b=native(current,(current=plain,key));
const c=vm.runInNewContext('native(target,key)',{native,target,key});
extra.values=[typeof a.get,typeof b.get,typeof c.get];stop();
const after=Reflect.ownKeys(globalThis);extra.globalsUnchanged=before.length===after.length&&before.every((v,i)=>v===after[i]);
extra.originalHandleSurvives=same(targetId,targetId);`,{inProcessInspection:true})
  complete(data,3);assert.equal(data.memberGets,1);assert(data.globalsUnchanged&&data.originalHandleSurvives)
  assert.equal(data.capture.handleMode,'private-inspector-summary')
  assert.deepEqual(data.values,['function','function','function'])
  assert(data.events.every(e=>e.target.watchedAccessorKeys.includes('tmp')))
})

test('private summaries ignore unrelated exception getters and leave their fields intact',()=>{
  const data=fixture(`let reads=0;
const foreign={get quodNativeEntry(){reads++;throw Error('foreign getter')},args:target};
globalThis.__quodEntryFixture.foreign=foreign;
const capture=observeNativeEntryArguments(post,{inProcessInspection:true});
capture.condition('Object.getOwnPropertyDescriptor',{describeOwnAccessors:true});
const remote=post('Runtime.evaluate',{expression:'globalThis.__quodEntryFixture.foreign',objectGroup:'quod-entry-fixture'}).result;
extra.owned=capture.handleException({exceptionDetails:{exception:remote}});
extra.capture=capture.report();capture.close();extra.reads=reads;
extra.intact=foreign.args===target&&typeof Object.getOwnPropertyDescriptor(foreign,'quodNativeEntry').get==='function';`)
  complete(data,0);assert.equal(data.owned,false);assert.equal(data.reads,0);assert(data.intact)
  assert.equal(data.capture.ignored,1)
})

test('private summaries use captured intrinsics without touching poisoned descriptor prototypes',()=>{
  const data=fixture(`const originals={descriptor:Object.getOwnPropertyDescriptor,keys:Reflect.ownKeys,has:Object.hasOwn,
 proxy:types.isProxy,define:Reflect.defineProperty,setPrototype:Object.setPrototypeOf,integer:Number.isSafeInteger};
const define=Object.defineProperty;
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
let poisoned=0;const poison=()=>{poisoned++;throw Error('poisoned public method')};
Object.getOwnPropertyDescriptor=poison;Reflect.ownKeys=poison;Object.hasOwn=poison;types.isProxy=poison;
Reflect.defineProperty=poison;Object.setPrototypeOf=poison;Number.isSafeInteger=poison;
define(Object.prototype,'get',{get:poison,configurable:true});define(Object.prototype,'value',{get:poison,configurable:true});
const answer=native(target,key);
delete Object.prototype.get;delete Object.prototype.value;
Object.getOwnPropertyDescriptor=originals.descriptor;Reflect.ownKeys=originals.keys;Object.hasOwn=originals.has;types.isProxy=originals.proxy;
Reflect.defineProperty=originals.define;Object.setPrototypeOf=originals.setPrototype;Number.isSafeInteger=originals.integer;
extra.answer=typeof answer.get;extra.poisoned=poisoned;stop();`,{inProcessInspection:true})
  complete(data,1);assert.equal(data.poisoned,0);assert.equal(data.answer,'function')
  assert.equal(data.events[0].target.ownAccessorCount,1)
})

test('private summaries clear accessor closures as well as argument objects',()=>{
  const data=fixture(`const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
function temporary(){const captured={tmp:1},value={get tmp(){return captured}};
 const refs=[new WeakRef(captured),new WeakRef(value)];native(value,key);return refs}
const refs=temporary();stop();await new Promise(resolve=>setImmediate(resolve));
for(let i=0;i<4;i++)globalThis.gc();extra.collected=refs.every(r=>r.deref()===undefined);`,{inProcessInspection:true,gc:true})
  complete(data,1);assert(data.collected)
})

test('private summaries preserve native errors and observe revoked proxies without touching their traps',()=>{
  const data=fixture(`let traps=0,failed=0;
const proxy=new Proxy({}, {ownKeys(){traps++;throw Error('unexpected inspection')},getOwnPropertyDescriptor(){traps++;return undefined}});
const revocable=Proxy.revocable({},{});revocable.revoke();
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
native(proxy,key);
for(const value of [undefined,revocable.proxy])try{native(value,key)}catch(e){if(e instanceof TypeError)failed++}
stop();extra.traps=traps;extra.failed=failed;`,{inProcessInspection:true})
  complete(data,3);assert.equal(data.traps,1);assert.equal(data.failed,2)
  assert.deepEqual(data.events.map(e=>e.kind),['proxy','primitive','proxy'])
})

test('compare complete and accessor-only target descriptors with both cleanup modes and compilation logging',t=>{
  const rows=[],compiled=new URL('./worker-module-probe.mjs',import.meta.url).href,
    logs=new URL('./compiled-script-log.mjs',import.meta.url).href
  for(const targetAccessorsOnly of [false,true])for(const handleGroups of [false,true]) {
    const data=fixture(`const {compiledScriptRecord}=await import(${JSON.stringify(compiled)});
const {dictionaryCompiledScriptLog,validateCompilationCounts}=await import(${JSON.stringify(logs)});
const log=dictionaryCompiledScriptLog();
session.on('Debugger.scriptParsed',({params})=>log.append(compiledScriptRecord(params,post('Debugger.getScriptSource',{scriptId:params.scriptId}))));
const wide={get tmp(){gets++;return 'not-read'}};
for(let i=0;i<80;i++)wide['field'+i]=()=>i;
const now=performance.now.bind(performance);now();let observed=0;
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]],e=>{
 if(e.kind!=='object'||e.target.ownAccessorCount!==1)throw Error('incorrect native target');observed++;
});
const start=now();let values=0;
for(let i=0;i<300;i++)if(typeof native(wide,key).get==='function')values++;
extra.elapsedMs=now()-start;stop();extra.observed=observed;extra.values=values;
const report=log.report();validateCompilationCounts(report);extra.compiledEvents=report.events;
extra.commands={...commands};`,{ballast:2000,handleGroups,targetAccessorsOnly})
    complete(data,300);assert.equal(data.gets,0);assert.equal(data.values,300);assert.equal(data.observed,300)
    if(handleGroups){assert.equal(data.capture.groupsReleased,300);assert.equal(data.commands['Runtime.releaseObject'],300)}
    else if(targetAccessorsOnly)assert(data.commands['Runtime.releaseObject']<3000)
    else assert(data.commands['Runtime.releaseObject']>24000)
    assert(data.compiledEvents>=600)
    rows.push({handleGroups,targetAccessorsOnly,calls:300,elapsedMs:data.elapsedMs,commands:data.commands,compiledEvents:data.compiledEvents,
      capture:data.capture,replayEligible:false})
  }
  t.diagnostic(JSON.stringify({nativeHandleGroupMeasurements:rows}))
})

test('measure private native summaries against accessor-only protocol inspection with full compilation logging',t=>{
  const rows=[],compiled=new URL('./worker-module-probe.mjs',import.meta.url).href,
    logs=new URL('./compiled-script-log.mjs',import.meta.url).href
  for(const width of [0,80])for(const inProcessInspection of [false,true]) {
    const data=fixture(`const {compiledScriptRecord}=await import(${JSON.stringify(compiled)});
const {dictionaryCompiledScriptLog,validateCompilationCounts}=await import(${JSON.stringify(logs)});
const log=dictionaryCompiledScriptLog();
session.on('Debugger.scriptParsed',({params})=>log.append(compiledScriptRecord(params,post('Debugger.getScriptSource',{scriptId:params.scriptId}))));
const value={get tmp(){gets++;return 'not-read'}};
for(let i=0;i<${width};i++)value['field'+i]=()=>i;
const now=performance.now.bind(performance);now();let observed=0;
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]],entry=>{
 if(entry.kind!=='object'||entry.target.ownAccessorCount!==1||entry.target.watchedAccessorKeys.join(',')!=='tmp')throw Error('incorrect summary');observed++;
});
const start=now();let values=0;
for(let i=0;i<300;i++)if(typeof native(value,key).get==='function')values++;
extra.elapsedMs=now()-start;stop();extra.observed=observed;extra.values=values;
const report=log.report();validateCompilationCounts(report);extra.compiledEvents=report.events;extra.commands={...commands};`,
      {ballast:2000,targetAccessorsOnly:true,inProcessInspection})
    complete(data,300);assert.equal(data.observed,300);assert.equal(data.values,300)
    if(inProcessInspection){assert.equal(data.commands['Runtime.getProperties']??0,0);assert.equal(data.commands['Runtime.releaseObject'],300)}
    else {assert.equal(data.commands['Runtime.getProperties'],900);assert.equal(data.commands['Runtime.releaseObject'],2400)}
    assert(data.compiledEvents>=600)
    rows.push({width,inProcessInspection,calls:300,elapsedMs:data.elapsedMs,commands:data.commands,
      compiledEvents:data.compiledEvents,capture:data.capture,replayEligible:false})
  }
  t.diagnostic(JSON.stringify({privateNativeSummaryMeasurements:rows}))
})

test('compare native argument capture against an empty full-pause handler',t=>{
  const rows=[]
  for(const mode of ['pause','capture']) {
    const data=fixture(`const now=performance.now.bind(performance);now();
let observed=0,withGetter=0,callerSeen=false;
const stop=${mode==='capture'?`captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]],entry=>{
 observed++;if(entry.target.watchedAccessorKeys.includes('tmp'))withGetter++;callerSeen||=entry.frames.some(f=>f.name==='invoke');
})`:`(()=>{const {breakpointId}=post('Debugger.setBreakpointOnFunctionCall',{objectId:watched});return ()=>post('Debugger.removeBreakpoint',{breakpointId})})()`};
function invoke(value,k){return native(value,k)}
let accessorCount=0,valueCount=0;const start=now();
for(let i=0;i<300;i++){const r=invoke(i%2?target:plain,key);if(r.get)accessorCount++;else valueCount+=r.value}
extra.elapsedMs=now()-start;stop();Object.assign(extra,{observed,withGetter,callerSeen,accessorCount,valueCount});`,{ballast:2000})
    assert.equal(data.accessorCount,150);assert.equal(data.valueCount,1050)
    if(mode==='capture'){complete(data,300);assert.equal(data.observed,300);assert.equal(data.withGetter,150);assert(data.callerSeen)}
    else assert.equal(data.pauses,300)
    rows.push({mode,calls:300,elapsedMs:data.elapsedMs,pauses:data.pauses,argumentsCaptured:data.observed,replayEligible:false})
  }
  t.diagnostic(JSON.stringify({nativeEntryMeasurements:rows}))
})
