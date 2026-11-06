// Diagnostic experiment only. No production key or replay decision uses this.
import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {createRequire} from 'node:module'
import vm from 'node:vm'
import {randomBytes} from 'node:crypto'
import {nativeOwnerGuard} from './native-owner-guard.mjs'
import {observeScalarProperty} from './scalar-property-probe.mjs'
import {observeModuleMetadata} from './worker-module-metadata-probe.mjs'
const probe=new URL('./native-entry-probe.mjs',import.meta.url).href

function fixture(body,{ballast=0,gc=false,handleGroups=false,targetAccessorsOnly=false,directArguments=false,targetInspection='protocol',nativeCapture=false,ordinaryOwners=false}={}) {
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
 const capture=observeNativeEntryArguments(post,{onEntry,handleGroups:${JSON.stringify(handleGroups)},targetAccessorsOnly:${JSON.stringify(targetAccessorsOnly)},directArguments:${JSON.stringify(directArguments)},targetInspection:${JSON.stringify(targetInspection)},nativeCapture:${JSON.stringify(nativeCapture)},ordinaryOwners:${JSON.stringify(ordinaryOwners)}});
 const listener=({params})=>capture.handleException(params);session.on('Runtime.exceptionThrown',listener);
 const breakpoints=registrations.map(([api,expression,options])=>{
  const fn=post('Runtime.evaluate',{expression,objectGroup:'quod-entry-fixture'}).result.objectId;
  const condition=capture.condition(api,options);
  // Inspector accepts malformed breakpoint conditions without a setup error.
  // Parse every generated variant here so invalid syntax cannot look like zero calls.
  new vm.Script(condition);
  return post('Debugger.setBreakpointOnFunctionCall',{objectId:fn,condition}).breakpointId;
 });
 const stop=()=>{for(const breakpointId of breakpoints)post('Debugger.removeBreakpoint',{breakpointId});
   session.removeListener('Runtime.exceptionThrown',listener);extra.capture=capture.report();capture.close()};
 stop.inspectException=params=>capture.handleException(params);
 return stop;
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
  assert.equal(data.capture.calls,calls,JSON.stringify(data.capture));assert.equal(data.capture.cleared,calls)
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

for(const {directArguments,targetInspection='protocol',nativeCapture=false} of [{directArguments:false},{directArguments:true},{directArguments:true,targetInspection:'watched'},{directArguments:true,nativeCapture:true}])test('actual native capture handles C++ proxy checks and receiver-based descriptor APIs: '+directArguments+'/'+targetInspection+'/native:'+nativeCapture,()=>{
  const data=fixture(`let traps=0;const proxy=new Proxy({}, {get(){traps++;throw Error('trap')},getOwnPropertyDescriptor(){traps++;throw Error('trap')}});
const isProxy=types.isProxy,lookup=Object.prototype.__lookupGetter__;
const stop=captureEntries([
 ['util.types.isProxy','process.getBuiltinModule("util").types.isProxy',{}],
 ['Object.prototype.__lookupGetter__','Object.prototype.__lookupGetter__',{receiver:true,describeOwnAccessors:true}]
]);
extra.values=[isProxy(proxy),isProxy(plain),typeof lookup.call(target,key),typeof lookup.bind(target)(key)];
stop();extra.traps=traps;`,{directArguments,targetInspection,nativeCapture})
  complete(data,4);assert.equal(data.traps,0)
  assert.deepEqual(data.values,[true,false,'function','function'])
  assert.deepEqual(data.events.map(e=>e.kind),['proxy','object','object','object'])
  assert(data.events.slice(2).every(e=>e.receiverBased&&e.target.watchedAccessorKeys.includes('tmp')))
})

for(const {directArguments,targetInspection='protocol',nativeCapture=false} of [{directArguments:false},{directArguments:true},{directArguments:true,targetInspection:'watched'},{directArguments:true,nativeCapture:true}])test('native capture preserves arguments and queued microtasks: '+directArguments+'/'+targetInspection+'/native:'+nativeCapture,()=>{
  const data=fixture(`let keyConversions=0,microtasks=0;const objectKey={toString(){keyConversions++;return key}};
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
let current=target;queueMicrotask(()=>{current=plain;microtasks++});
function originalCaller(value,k){return native(value,k)}
const a=originalCaller(current,objectKey);current=plain;const b=originalCaller(current,key);
extra.during=microtasks;extra.values=[typeof a.get,b.value];extra.keyConversions=keyConversions;
stop();await Promise.resolve();extra.after=microtasks;`,{directArguments,targetInspection,nativeCapture})
  complete(data,2);assert.equal(data.during,0);assert.equal(data.after,1)
  assert.equal(data.keyConversions,1);assert.deepEqual(data.values,['function',7])
  assert.deepEqual(data.events.map(e=>e.target.watchedAccessorKeys),[['tmp'],[]])
  assert(data.events.every(e=>e.frames.some(f=>f.name==='originalCaller'&&typeof f.scriptId==='string')),
    'native call-site capture must preserve the actual caller without materializing lexical scopes')
})

for(const {directArguments,targetInspection='protocol',nativeCapture=false} of [{directArguments:false},{directArguments:true},{directArguments:true,targetInspection:'watched'},{directArguments:true,nativeCapture:true}])test('clearing owned exception wrappers does not retain application arguments: '+directArguments+'/'+targetInspection+'/native:'+nativeCapture,()=>{
  const data=fixture(`const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
function temporary(){const value={tmp:1};const weak=new WeakRef(value);native(value,key);return weak}
const weak=temporary();stop();
await new Promise(resolve=>setImmediate(resolve));
for(let i=0;i<4;i++)globalThis.gc();extra.collected=weak.deref()===undefined;`,{gc:true,directArguments,targetInspection,nativeCapture})
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

for(const {directArguments,targetInspection='protocol',nativeCapture=false} of [{directArguments:false},{directArguments:true},{directArguments:true,targetInspection:'watched'},{directArguments:true,nativeCapture:true}])test('accessor-only target inspection preserves own accessor evidence: '+directArguments+'/'+targetInspection+'/native:'+nativeCapture,()=>{
  const data=fixture(`let reads=0;
const proto={get seed(){reads++;throw Error('inherited accessor')}};
const value=Object.create(proto);
Object.defineProperties(value,{tmp:{get(){reads++;throw Error('own getter')}},invalidate:{set(v){reads++;}},
 seed:{value:123},data:{value:()=>42}});
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
extra.returned=typeof native(value,key).get;stop();extra.reads=reads;`,{targetAccessorsOnly:true,directArguments,targetInspection,nativeCapture})
  complete(data,1);assert.equal(data.reads,0);assert.equal(data.returned,'function')
  if(targetInspection==='watched'||nativeCapture){assert.equal(data.events[0].target.watchedAccessorCount,2);assert(!Object.hasOwn(data.events[0].target,'ownAccessorCount'))}
  else assert.equal(data.events[0].target.ownAccessorCount,2)
  assert.deepEqual(data.events[0].target.watchedAccessorKeys.sort(),['invalidate','tmp'])
})

for(const {targetInspection='protocol',nativeCapture=false} of [{},{targetInspection:'watched'},{nativeCapture:true}])test('direct native arguments preserve replacing bound callees and add no public global bridge: '+targetInspection+'/native:'+nativeCapture,()=>{
  const data=fixture(`const before=Reflect.ownKeys(globalThis);
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
let owner={get call(){memberGets++;owner={call:native};return native.bind(null,target)}};
const a=owner.call(key);let current=target;const b=native(current,(current=plain,key));
const c=vm.runInNewContext('native(target,key)',{native,target,key});
extra.values=[typeof a.get,typeof b.get,typeof c.get];stop();
const after=Reflect.ownKeys(globalThis);extra.globalsUnchanged=before.length===after.length&&before.every((v,i)=>v===after[i]);
extra.originalHandleSurvives=same(targetId,targetId);`,{directArguments:true,targetInspection,nativeCapture})
  complete(data,3);assert.equal(data.memberGets,1);assert(data.globalsUnchanged&&data.originalHandleSurvives)
  assert.equal(data.capture.argumentMode,'direct')
  assert.deepEqual(data.values,['function','function','function'])
  assert(data.events.every(e=>e.target.watchedAccessorKeys.includes('tmp')))
})

for(const {targetInspection='protocol',nativeCapture=false} of [{},{targetInspection:'watched'},{nativeCapture:true}])test('direct native arguments ignore unrelated exception getters and leave their fields intact: '+targetInspection+'/native:'+nativeCapture,()=>{
  const data=fixture(`let reads=0;
const foreign={get quodNativeEntry(){reads++;throw Error('foreign getter')},args:target};
globalThis.__quodEntryFixture.foreign=foreign;
const capture=observeNativeEntryArguments(post,{directArguments:true,targetInspection:${JSON.stringify(targetInspection)},nativeCapture:${JSON.stringify(nativeCapture)}});
capture.condition('Object.getOwnPropertyDescriptor',{describeOwnAccessors:true});
const remote=post('Runtime.evaluate',{expression:'globalThis.__quodEntryFixture.foreign',objectGroup:'quod-entry-fixture'}).result;
extra.owned=capture.handleException({exceptionDetails:{exception:remote}});
extra.capture=capture.report();capture.close();extra.reads=reads;
extra.intact=foreign.args===target&&typeof Object.getOwnPropertyDescriptor(foreign,'quodNativeEntry').get==='function';`)
  complete(data,0);assert.equal(data.owned,false);assert.equal(data.reads,0);assert(data.intact)
  assert.equal(data.capture.ignored,1)
})

test('foreign error-shaped objects remain unknown instead of inheriting native Error exclusion',()=>{
  const data=fixture(`let contextId;
session.on('Runtime.executionContextCreated',({params})=>{if(params.context.name==='quod-foreign-lookalike')contextId=params.context.id});
const context=vm.createContext({reads:0},{name:'quod-foreign-lookalike'});
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
const result=post('Runtime.evaluate',{contextId,expression:'({__proto__:null,[Symbol.toStringTag]:"Error",name:"Error",get quodNativeEntry(){reads++;throw Error("must not read")}})',generatePreview:false});
extra.subtype=result.result.subtype??null;
stop.inspectException({exceptionDetails:{exception:result.result}});
extra.foreignReads=post('Runtime.evaluate',{contextId,expression:'reads',returnByValue:true}).result.value;
native(target,key);stop();`,{directArguments:true,nativeCapture:true})
  assert.equal(data.subtype,null);assert.equal(data.foreignReads,0)
  assert.equal(data.capture.available,false);assert.equal(data.capture.ignored,0)
  assert.equal(data.capture.calls,1);assert.equal(data.capture.cleared,1)
  assert.deepEqual(data.capture.errors,['native capture protocol request failed'])
  assert.equal(data.capture.failureCounts['native capture protocol request failed'],1)
  assert.equal(data.capture.failureSamples.length,1);assert(data.capture.failureSamples[0].frames.length>0)
})

test('native entry conditions from foreign intrinsics use one private native bridge per context',()=>{
  const data=fixture(`const foreignNative=vm.runInNewContext('Object.getOwnPropertyDescriptor');
globalThis.__quodEntryFixture.foreignNative=foreignNative;
const stop=captureEntries([['foreign.descriptor','globalThis.__quodEntryFixture.foreignNative',{describeOwnAccessors:true}]]);
extra.returned=typeof foreignNative(target,key).get;
for(let i=0;i<5;i++)foreignNative(target,key);stop();`,{directArguments:true,nativeCapture:true})
  assert.equal(data.returned,'function');complete(data,6)
  assert.equal(data.capture.ignored,0);assert.equal(data.capture.contextCapture.contexts,1)
  assert.equal(data.capture.contextCapture.contextQueries,1)
  assert.equal(data.events.length,6);assert(data.events.every(e=>e.target.watchedAccessorKeys.join(',')==='tmp'))
})

test('foreign plain exceptions with context IDs are inspected without invoking getters or trusting Error labels',()=>{
  const data=fixture(`let contextId;
session.on('Runtime.executionContextCreated',({params})=>{if(params.context.name==='quod-private-context')contextId=params.context.id});
const context=vm.createContext({reads:0},{name:'quod-private-context'});
const before=post('Runtime.evaluate',{contextId,expression:'Reflect.ownKeys(globalThis).map(String)',returnByValue:true}).result.value;
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
const result=post('Runtime.evaluate',{contextId,expression:'({__proto__:null,name:"Error",get quodNativeEntry(){reads++;throw Error("must not read")}})',generatePreview:false});
stop.inspectException({exceptionDetails:{executionContextId:contextId,exception:result.result}});
extra.foreignReads=post('Runtime.evaluate',{contextId,expression:'reads',returnByValue:true}).result.value;
extra.globalsUnchanged=JSON.stringify(before)===JSON.stringify(post('Runtime.evaluate',{contextId,expression:'Reflect.ownKeys(globalThis).map(String)',returnByValue:true}).result.value);
native(target,key);stop();`,{directArguments:true,nativeCapture:true})
  complete(data,1);assert.equal(data.foreignReads,0);assert(data.globalsUnchanged)
  assert.equal(data.capture.ignored,1);assert.equal(data.capture.ignoredReasons[4],1)
  assert.equal(data.capture.contextCapture.contexts,1)
})

for(const {targetInspection='protocol',nativeCapture=false} of [{},{targetInspection:'watched'},{nativeCapture:true}])test('direct native arguments use captured intrinsics without touching poisoned descriptor prototypes: '+targetInspection+'/native:'+nativeCapture,()=>{
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
extra.answer=typeof answer.get;extra.poisoned=poisoned;stop();`,{directArguments:true,targetInspection,nativeCapture})
  complete(data,1);assert.equal(data.poisoned,0);assert.equal(data.answer,'function')
  assert.equal(targetInspection==='watched'||nativeCapture?data.events[0].target.watchedAccessorCount:data.events[0].target.ownAccessorCount,1)
})

for(const {targetInspection='protocol',nativeCapture=false} of [{},{targetInspection:'watched'},{nativeCapture:true}])test('direct native arguments clear accessor closures as well as argument objects: '+targetInspection+'/native:'+nativeCapture,()=>{
  const data=fixture(`const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
function temporary(){const captured={tmp:1},value={get tmp(){return captured}};
 const refs=[new WeakRef(captured),new WeakRef(value)];native(value,key);return refs}
const refs=temporary();stop();await new Promise(resolve=>setImmediate(resolve));
for(let i=0;i<4;i++)globalThis.gc();extra.collected=refs.every(r=>r.deref()===undefined);`,{directArguments:true,gc:true,targetInspection,nativeCapture})
  complete(data,1);assert(data.collected)
})

for(const {targetInspection='protocol',nativeCapture=false} of [{},{targetInspection:'watched'},{nativeCapture:true}])test('direct native arguments preserve native errors and observe revoked proxies without touching their traps: '+targetInspection+'/native:'+nativeCapture,()=>{
  const data=fixture(`let traps=0,failed=0;
const proxy=new Proxy({}, {ownKeys(){traps++;throw Error('unexpected inspection')},getOwnPropertyDescriptor(){traps++;return undefined}});
const revocable=Proxy.revocable({},{});revocable.revoke();
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
native(proxy,key);
for(const value of [undefined,revocable.proxy])try{native(value,key)}catch(e){if(e instanceof TypeError)failed++}
stop();extra.traps=traps;extra.failed=failed;`,{directArguments:true,targetInspection,nativeCapture})
  complete(data,3);assert.equal(data.traps,1);assert.equal(data.failed,2)
  assert.deepEqual(data.events.map(e=>e.kind),['proxy','primitive','proxy'])
})

for(const {targetInspection='protocol',nativeCapture=false} of [{},{targetInspection:'watched'},{nativeCapture:true}])test('direct native arguments distinguish absent and explicit undefined arguments: '+targetInspection+'/native:'+nativeCapture,()=>{
  const data=fixture(`let failed=0;
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
try{native()}catch(e){if(e instanceof TypeError)failed++}
try{native(undefined,key)}catch(e){if(e instanceof TypeError)failed++}
extra.absent=native(12,key)===undefined;stop();extra.failed=failed;`,{directArguments:true,targetInspection,nativeCapture})
  complete(data,3);assert.equal(data.failed,2);assert(data.absent)
  assert.deepEqual(data.events.map(e=>e.argumentCount),[0,2,2])
  assert(data.events.every(e=>e.kind==='primitive'))
})

test('adaptive inspection rechecks watched descriptors after target and property changes',()=>{
  const data=fixture(`const wide={get tmp(){gets++;throw Error('getter')}};
for(let i=0;i<128;i++)Object.defineProperty(wide,'field'+i,{get(){gets++;throw Error('getter')}});
const next=Object.create({get seed(){gets++;throw Error('inherited getter')}});
Object.defineProperty(next,'tmp',{get(){gets++;throw Error('getter')},configurable:true});
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
function invoke(value){return native(value,key)}
invoke(wide);invoke(next);
Object.defineProperty(next,'tmp',{value:3,configurable:true});invoke(next);
Object.defineProperty(next,'tmp',{get(){gets++;throw Error('new getter')},configurable:true});invoke(next);
stop();`,{directArguments:true,targetAccessorsOnly:true,targetInspection:'adaptive'})
  complete(data,4);assert.equal(data.gets,0)
  assert.equal(data.capture.adaptiveSites,1);assert.equal(data.capture.scopedInspections,3)
  assert.deepEqual(data.events.map(e=>e.target.watchedAccessorKeys),[['tmp'],['tmp'],[],['tmp']])
  assert.deepEqual(data.events.map(e=>e.target.inspectionScope),['all-own-accessors','watched-properties','watched-properties','watched-properties'])
  assert(data.events.slice(1).every(e=>!Object.hasOwn(e.target,'ownAccessorCount')))
})

for(const nativeCapture of [false,true])test('scoped inspection does not retain targets while its private helper remains active: native '+nativeCapture,()=>{
  const data=fixture(`const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
function temporary(){const captured={tmp:1},value={get tmp(){return captured}};
 const refs=[new WeakRef(captured),new WeakRef(value)];native(value,key);return refs}
const refs=temporary();await new Promise(resolve=>setImmediate(resolve));
for(let i=0;i<4;i++)globalThis.gc();extra.collectedWhileActive=refs.every(r=>r.deref()===undefined);
native(target,key);stop();`,{directArguments:true,gc:true,targetInspection:nativeCapture?'protocol':'watched',nativeCapture})
  complete(data,2);assert(data.collectedWhileActive)
  assert.equal(data.capture.scopedInspections,2)
})

test('native capture rejects malformed owned wrappers and foreign getters without leaking target values',()=>{
  const {createCapture}=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
  const nonce='a'.repeat(64),capture=createCapture(nonce)
  const target={get tmp(){throw Error('getter must not run')}}
  const payload=()=>({__proto__:null,quodNativeEntry:nonce,api:'test',argument:target,
    argumentCount:1,receiverBased:false,describeOwnAccessors:true})
  let gets=0
  const foreign={__proto__:null,get quodNativeEntry(){gets++;throw Error('foreign getter')},argument:target}
  const ignored=capture(foreign)
  assert.equal(ignored.owned,false);assert.equal(gets,0);assert.equal(foreign.argument,target)
  const invalid=payload();invalid.argumentCount=NaN
  const rejected=capture(invalid)
  assert(rejected.owned&&!rejected.available&&rejected.cleared)
  assert.equal(invalid.argument,null);assert.equal(invalid.quodNativeEntry,null);assert.equal(invalid.api,null)
  const frozen=Object.freeze(payload()),untouched=capture(frozen)
  assert(untouched.owned&&!untouched.available&&!untouched.cleared);assert.equal(frozen.argument,target)
  const valid=payload(),summary=capture(valid)
  assert(summary.owned&&summary.available&&summary.cleared&&summary.inspected)
  assert.equal(summary.mask,2);assert.equal(summary.kind,1)
  assert(!Object.hasOwn(summary,'argument'));assert.equal(valid.argument,null)
})

test('native data-field lookup agrees with descriptor baseline and never invokes malformed fields',()=>{
  const require=createRequire(import.meta.url),nonce='f'.repeat(64)
  const captures=[process.env.QUOD_NATIVE_CAPTURE_DESCRIPTOR_ADDON,process.env.QUOD_NATIVE_CAPTURE_ADDON]
    .map(file=>require(file).createCapture(nonce))
  const fields=['quodNativeEntry','api','argument','argumentCount','receiverBased','describeOwnAccessors']
  let gets=0,traps=0
  const target={get tmp(){gets++;throw Error('target getter')}}
  const fresh=()=>({__proto__:null,quodNativeEntry:nonce,api:'fixture',argument:target,
    argumentCount:1,receiverBased:false,describeOwnAccessors:true})
  const cases=[()=>fresh(),()=>Object.freeze(fresh()),()=>Object.create(fresh()),
    ()=>new Proxy(fresh(),{getPrototypeOf(){traps++;throw Error('proxy')},getOwnPropertyDescriptor(){traps++;throw Error('proxy')}}),
    ()=>{const p=fresh();for(const name of fields)Object.defineProperty(p,name,{enumerable:false,configurable:false});return p}]
  for(const name of fields) {
    cases.push(()=>{const p=fresh();delete p[name];return p})
    cases.push(()=>{const p=fresh();Object.defineProperty(p,name,{writable:false});return p})
    cases.push(()=>{const p=fresh();Object.defineProperty(p,name,{get(){gets++;throw Error('payload getter')}});return p})
    cases.push(()=>{const p=fresh();Object.defineProperty(p,name,{get:undefined,set:undefined});return p})
  }
  for(const make of cases) {
    const results=captures.map(capture=>capture(make()))
    assert.deepEqual(results[1],results[0])
  }
  assert.equal(gets,0);assert.equal(traps,0)
})

test('measure native payload validation without changing capture summaries',t=>{
  const measurements=[]
  for(const mode of ['descriptors','real-data','real-data','descriptors']) {
    const addon=mode==='descriptors'?process.env.QUOD_NATIVE_CAPTURE_DESCRIPTOR_ADDON:process.env.QUOD_NATIVE_CAPTURE_ADDON
    const source=`const {createCapture}=require(${JSON.stringify(addon)}),nonce='f'.repeat(64),capture=createCapture(nonce);
let gets=0;const target={get tmp(){gets++;throw Error('getter')}};
const payload=()=>({__proto__:null,quodNativeEntry:nonce,api:'fixture',argument:target,argumentCount:1,receiverBased:false,describeOwnAccessors:true});
for(let i=0;i<2000;i++)capture(payload());
const calls=100000,started=performance.now();let matches=0;
for(let i=0;i<calls;i++){const p=payload(),r=capture(p);if(r.owned&&r.available&&r.cleared&&r.metadataValid&&r.failure===0&&r.mask===2&&r.inspected&&r.kind===1&&p.argument===null&&p.api===null&&p.quodNativeEntry===null)matches++}
console.log(JSON.stringify({mode:${JSON.stringify(mode)},calls,matches,gets,elapsedMs:performance.now()-started}));`
    const result=spawnSync(process.execPath,['-e',source],{encoding:'utf8',timeout:20000})
    assert.equal(result.status,0,result.stderr||String(result.error))
    const row=JSON.parse(result.stdout);assert.equal(row.calls,100000);assert.equal(row.matches,row.calls);assert.equal(row.gets,0)
    measurements.push(row)
  }
  t.diagnostic(JSON.stringify({nativePayloadValidationMeasurements:measurements,scope:'native helper only; not Inspector transport, workload or CI savings'}))
})

test('native context installation requires one exact writable private holder',()=>{
  const {createContextCapture}=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
  const nonce=randomBytes(32).toString('hex'),key=randomBytes(32).toString('hex')
  assert.equal(createContextCapture(nonce,key),false,'missing holder')
  const first={__proto__:null,[key]:true,capture:undefined},second={__proto__:null,[key]:true,capture:undefined}
  assert.equal(createContextCapture(nonce,key),false,'duplicate holders')
  assert.equal(first.capture,undefined);assert.equal(second.capture,undefined)
  delete second[key]
  assert.equal(createContextCapture(nonce,key),true)
  assert.equal(typeof first.capture,'function');assert.equal(first[key],null)
  assert.equal(createContextCapture(nonce,key),false,'consumed holder cannot be reused')
})

test('native context holder discovery does not execute getters or proxy traps',()=>{
  const {createContextCapture}=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
  const nonce=randomBytes(32).toString('hex'),key=randomBytes(32).toString('hex')
  let reads=0,traps=0
  const holder={__proto__:null,get [key](){reads++;throw Error('marker getter')},capture:undefined}
  const proxy=new Proxy(holder,{getPrototypeOf(){traps++;throw Error('proxy trap')},getOwnPropertyDescriptor(){traps++;throw Error('proxy trap')}})
  assert.equal(createContextCapture(nonce,key),false)
  assert.equal(reads,0);assert.equal(traps,0);assert(proxy)
  Object.defineProperty(holder,key,{value:true,writable:true})
  Object.defineProperty(holder,'capture',{get(){reads++;throw Error('capture getter')}})
  assert.equal(createContextCapture(nonce,key),false);assert.equal(reads,0)
})

test('native context installation creates the callback in the holder realm and releases target references',()=>{
  const {createContextCapture}=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
  const nonce=randomBytes(32).toString('hex'),key=randomBytes(32).toString('hex')
  const context=vm.createContext({})
  const holder=vm.runInContext(`({__proto__:null,${JSON.stringify(key)}:true,capture:void 0})`,context)
  assert.equal(createContextCapture(nonce,key),true)
  assert.equal(Object.getPrototypeOf(holder.capture),vm.runInContext('Function.prototype',context))
  let reads=0
  const target={get tmp(){reads++;return 1}}
  const payload={__proto__:null,quodNativeEntry:nonce,api:'fixture',argument:target,argumentCount:1,receiverBased:false,describeOwnAccessors:true}
  const result=holder.capture(payload)
  assert(result.owned&&result.available&&result.cleared);assert.equal(result.mask,2)
  assert.equal(reads,0);assert.equal(payload.argument,null);assert.equal(payload.quodNativeEntry,null)
})

test('native capture rejects interceptor targets but clears the owned wrapper',()=>{
  const {createCapture}=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
  const nonce='b'.repeat(64),capture=createCapture(nonce)
  const payload={__proto__:null,quodNativeEntry:nonce,api:'test',argument:process.env,
    argumentCount:1,receiverBased:false,describeOwnAccessors:true}
  const result=capture(payload)
  assert(result.owned&&!result.available&&result.cleared)
  assert.equal(payload.argument,null)
})

test('native capture observes host callback presence without executing its descriptor getter',()=>{
  const addon=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
  const nonce='c'.repeat(64),capture=addon.createCapture(nonce)
  const {target,state,apiWrapper}=addon.createNativeDataFixture()
  assert.equal(apiWrapper,false)
  const payload={__proto__:null,quodNativeEntry:nonce,api:'test',argument:target,
    argumentCount:1,receiverBased:false,describeOwnAccessors:true}
  const result=capture(payload)
  assert(result.owned&&result.available&&result.cleared&&result.inspected)
  assert.equal(result.mask,2);assert.equal(state.calls,0)
  assert.equal(Object.getOwnPropertyDescriptor(target,'tmp').value,'host callback value')
  assert.equal(state.calls,1,'ordinary descriptor inspection must exercise the fixture callback')
  const foreign=addon.createNativeDataFixture(true)
  const ignored=capture(foreign.target)
  assert(!ignored.owned&&ignored.available);assert.equal(foreign.state.calls,0)
  Object.getOwnPropertyDescriptor(foreign.target,'quodNativeEntry')
  assert.equal(foreign.state.calls,1)
})

test('native capture rejects API wrappers before any host access check',()=>{
  const addon=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
  const nonce='d'.repeat(64),capture=addon.createCapture(nonce)
  const {target,state,apiWrapper}=addon.createNativeDataFixture(false,true)
  assert.equal(apiWrapper,true);assert.equal(state.calls,0)
  const payload={__proto__:null,quodNativeEntry:nonce,api:'test',argument:target,
    argumentCount:1,receiverBased:false,describeOwnAccessors:true}
  const result=capture(payload)
  assert(result.owned&&!result.available&&result.cleared)
  assert.equal(state.calls,0);assert.equal(payload.argument,null)
  const descriptor=vm.runInNewContext('Object.getOwnPropertyDescriptor(target,"tmp")',{target})
  assert.equal(descriptor.value,'host callback value')
  assert(state.calls>=2,'the ordinary cross-context read must exercise the host access check and getter')
})

test('native ordinary-owner guards reject host objects before scalar or module descriptor access',()=>{
  const addon=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON),ownerGuard=nativeOwnerGuard()
  const {target,state,apiWrapper}=addon.createNativeDataFixture(false,true)
  assert(apiWrapper);assert.equal(state.calls,0);assert.equal(ownerGuard(target),false)
  const scalar=observeScalarProperty(target,'tmp',{ownerGuard})
  assert.equal(scalar.report().installed,false);assert.equal(scalar.report().ownerGuardPassed,false)
  assert(scalar.report().gaps.includes('unsupported native scalar owner'))
  class Graph {ensureModule(){return {id:'/a',url:'/a',file:'/a',meta:undefined}}invalidateModule(){}}
  const module=observeModuleMetadata(Graph,{observation:'accessor',ownerGuard})
  assert.equal(module.observeMetadata(target,'fixture'),target)
  assert(module.report().gaps.includes('unsupported native metadata owner'));assert.equal(module.report().metadata.length,0)
  const plain={tmp:'/tmp/fixture',invalidate:false}
  assert.equal(module.observeMetadata(plain,'fixture'),plain)
  assert(module.report().metadata[0].scalars.every(s=>s.installed&&s.ownerGuardRequired&&s.ownerGuardPassed))
  const seed=observeScalarProperty({seed:7},'seed',{ownerGuard})
  assert(seed.report().installed&&seed.report().ownerGuardPassed)
  seed.close();scalar.close();module.close();assert.equal(state.calls,0)
  let traps=0;assert.equal(ownerGuard(new Proxy({},{getPrototypeOf(){traps++;throw Error('trap')}})),false)
  assert.equal(traps,0);assert.equal(ownerGuard(process.env),false)
})

test('native wrapper exclusion is conditional on the guarded owner scope and never inspects host properties',()=>{
  const addon=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON),nonce='e'.repeat(64)
  const {target,state}=addon.createNativeDataFixture(false,true)
  const payload=()=>({__proto__:null,quodNativeEntry:nonce,api:'fixture',argument:target,argumentCount:1,receiverBased:false,describeOwnAccessors:true})
  const strict=addon.createCapture(nonce)(payload())
  assert(strict.owned&&!strict.available&&strict.cleared);assert.equal(strict.excludedOwner,0)
  const guarded=addon.createCapture(nonce,true)(payload())
  assert(guarded.owned&&guarded.available&&guarded.cleared&&!guarded.inspected)
  assert.equal(guarded.excludedOwner,1);assert.equal(guarded.mask,0);assert.equal(state.calls,0)
})

test('foreign-context bridge retains the guarded owner scope without losing ordinary targets',()=>{
  const data=fixture(`const foreignNative=vm.runInNewContext('Object.getOwnPropertyDescriptor');
globalThis.__quodEntryFixture.foreignNative=foreignNative;
const stop=captureEntries([['foreign.descriptor','globalThis.__quodEntryFixture.foreignNative',{describeOwnAccessors:true}]]);
foreignNative(globalThis,'process');foreignNative(target,key);stop();`,{directArguments:true,nativeCapture:true,ordinaryOwners:true})
  complete(data,2);assert.equal(data.capture.ownerPolicy,'native-ordinary')
  assert.equal(data.capture.contextCapture.contextQueries,1)
  assert.equal(data.events[0].target.inspectionScope,'excluded-owner-type')
  assert.equal(data.events[0].target.ownerType,'api-wrapper')
  assert.equal(data.events[1].target.inspectionScope,'watched-properties')
  assert.deepEqual(data.events[1].target.watchedAccessorKeys,['tmp'])
})

for(const foreign of [false,true])test('native capture distinguishes foreign execution-context exceptions: '+foreign,()=>{
  const data=fixture(`let contextId;
session.on('Runtime.executionContextCreated',({params})=>{if(params.context.name==='quod-foreign-exception')contextId=params.context.id});
const context=vm.createContext({}, {name:'quod-foreign-exception'});
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
const outcome=post('Runtime.evaluate',{expression:'throw new Error("unrelated fixture exception")',silent:false,
  ...(${JSON.stringify(foreign)}?{contextId}:{})});
extra.exceptionObserved=!!outcome.exceptionDetails;
// Runtime.evaluate returns this exception to its caller instead of emitting
// Runtime.exceptionThrown. Send that exact response through the same handler.
stop.inspectException(outcome);
native(target,key);stop();`,{directArguments:true,nativeCapture:true})
  assert(data.exceptionObserved)
  complete(data,1)
  assert.equal(data.capture.ignored,1)
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

test('measure direct native arguments against accessor-only protocol inspection with full compilation logging',t=>{
  const rows=[],compiled=new URL('./worker-module-probe.mjs',import.meta.url).href,
    logs=new URL('./compiled-script-log.mjs',import.meta.url).href
  for(const width of [0,80])for(const directArguments of [false,true]) {
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
      {ballast:2000,targetAccessorsOnly:true,directArguments})
    complete(data,300);assert.equal(data.observed,300);assert.equal(data.values,300)
    if(directArguments){assert.equal(data.commands['Runtime.getProperties'],600);assert(data.commands['Runtime.releaseObject']<2400)}
    else {assert.equal(data.commands['Runtime.getProperties'],900);assert.equal(data.commands['Runtime.releaseObject'],2400)}
    assert(data.compiledEvents>=600)
    rows.push({width,directArguments,calls:300,elapsedMs:data.elapsedMs,commands:data.commands,
      compiledEvents:data.compiledEvents,capture:data.capture,replayEligible:false})
  }
  t.diagnostic(JSON.stringify({directNativeArgumentMeasurements:rows}))
})

test('compare scoped inspection on narrow and accessor-heavy targets with complete compilation logging',t=>{
  const rows=[],compiled=new URL('./worker-module-probe.mjs',import.meta.url).href,
    logs=new URL('./compiled-script-log.mjs',import.meta.url).href
  for(const width of [0,80,2247])for(const mode of ['protocol','watched','adaptive','native']) {
    const nativeCapture=mode==='native',targetInspection=nativeCapture?'protocol':mode
    const data=fixture(`const {compiledScriptRecord}=await import(${JSON.stringify(compiled)});
const {dictionaryCompiledScriptLog,validateCompilationCounts}=await import(${JSON.stringify(logs)});
const log=dictionaryCompiledScriptLog();
session.on('Debugger.scriptParsed',({params})=>log.append(compiledScriptRecord(params,post('Debugger.getScriptSource',{scriptId:params.scriptId}))));
const value={get tmp(){gets++;throw Error('getter')}};
for(let i=0;i<${width};i++)Object.defineProperty(value,'field'+i,{get(){gets++;throw Error('getter')}});
const now=performance.now.bind(performance);now();let observed=0;
const stop=captureEntries([
 ['util.types.isProxy','process.getBuiltinModule("util").types.isProxy',{}],
 ['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]
],entry=>{
 if(entry.api!=='Object.getOwnPropertyDescriptor'||entry.kind!=='object'||entry.target.watchedAccessorKeys.join(',')!=='tmp')throw Error('incorrect scoped evidence');
 if(entry.target.inspectionScope==='all-own-accessors'&&entry.target.ownAccessorCount!==${width+1})throw Error('incorrect total count');
 if(entry.target.inspectionScope==='watched-properties'&&entry.target.watchedAccessorCount!==1)throw Error('incorrect scoped count');
 observed++;
});
function invoke(){return native(value,key)}
const start=now();let values=0;
for(let i=0;i<300;i++)if(typeof invoke().get==='function')values++;
extra.elapsedMs=now()-start;stop();extra.observed=observed;extra.values=values;
const report=log.report();validateCompilationCounts(report);extra.compiledEvents=report.events;extra.commands={...commands};`,
      {ballast:2000,targetAccessorsOnly:true,directArguments:true,targetInspection,nativeCapture})
    complete(data,300);assert.equal(data.observed,300);assert.equal(data.values,300)
    assert.equal(data.capture.scopedInspections,nativeCapture||targetInspection==='watched'?300:targetInspection==='adaptive'&&width>=64?299:0)
    assert(data.compiledEvents>=600)
    if(nativeCapture){assert.equal(data.commands['Runtime.getProperties']??0,0);assert.equal(data.commands['Runtime.callFunctionOn'],300);assert.equal(data.commands['Runtime.releaseObject'],300)}
    rows.push({width,targetInspection,nativeCapture,calls:300,elapsedMs:data.elapsedMs,commands:data.commands,
      compiledEvents:data.compiledEvents,capture:data.capture,replayEligible:false})
  }
  t.diagnostic(JSON.stringify({scopedTargetInspectionMeasurements:rows}))
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
