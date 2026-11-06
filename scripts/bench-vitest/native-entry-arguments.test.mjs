// Diagnostic experiment only. No production key or replay decision uses this.
import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
const probe=new URL('./native-entry-probe.mjs',import.meta.url).href

function fixture(body,{ballast=0,gc=false}={}) {
  const source=`import {Session} from 'node:inspector';
import vm from 'node:vm';
import {types} from 'node:util';
import {observeNativeEntryArguments} from ${JSON.stringify(probe)};
${Array.from({length:ballast},(_,i)=>`export const unused${i}=${i};`).join('\n')}
const session=new Session(),errors=[],pauses=[],events=[],extra={};
function post(method,params={}){
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
 const capture=observeNativeEntryArguments(post,{onEntry});
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

test('actual native capture handles C++ proxy checks and receiver-based descriptor APIs',()=>{
  const data=fixture(`let traps=0;const proxy=new Proxy({}, {get(){traps++;throw Error('trap')},getOwnPropertyDescriptor(){traps++;throw Error('trap')}});
const isProxy=types.isProxy,lookup=Object.prototype.__lookupGetter__;
const stop=captureEntries([
 ['util.types.isProxy','process.getBuiltinModule("util").types.isProxy',{}],
 ['Object.prototype.__lookupGetter__','Object.prototype.__lookupGetter__',{receiver:true,describeOwnAccessors:true}]
]);
extra.values=[isProxy(proxy),isProxy(plain),typeof lookup.call(target,key),typeof lookup.bind(target)(key)];
stop();extra.traps=traps;`)
  complete(data,4);assert.equal(data.traps,0)
  assert.deepEqual(data.values,[true,false,'function','function'])
  assert.deepEqual(data.events.map(e=>e.kind),['proxy','object','object','object'])
  assert(data.events.slice(2).every(e=>e.receiverBased&&e.target.watchedAccessorKeys.includes('tmp')))
})

test('native capture sees current arguments without running getters, conversion hooks or queued microtasks',()=>{
  const data=fixture(`let keyConversions=0,microtasks=0;const objectKey={toString(){keyConversions++;return key}};
const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
let current=target;queueMicrotask(()=>{current=plain;microtasks++});
function originalCaller(value,k){return native(value,k)}
const a=originalCaller(current,objectKey);current=plain;const b=originalCaller(current,key);
extra.during=microtasks;extra.values=[typeof a.get,b.value];extra.keyConversions=keyConversions;
stop();await Promise.resolve();extra.after=microtasks;`)
  complete(data,2);assert.equal(data.during,0);assert.equal(data.after,1)
  assert.equal(data.keyConversions,1);assert.deepEqual(data.values,['function',7])
  assert.deepEqual(data.events.map(e=>e.target.watchedAccessorKeys),[['tmp'],[]])
  assert(data.events.every(e=>e.frames.some(f=>f.name==='originalCaller'&&typeof f.scriptId==='string')),
    'native call-site capture must preserve the actual caller without materializing lexical scopes')
})

test('clearing owned exception wrappers does not retain application arguments',()=>{
  const data=fixture(`const stop=captureEntries([['Object.getOwnPropertyDescriptor','Object.getOwnPropertyDescriptor',{describeOwnAccessors:true}]]);
function temporary(){const value={tmp:1};const weak=new WeakRef(value);native(value,key);return weak}
const weak=temporary();stop();
await new Promise(resolve=>setImmediate(resolve));
for(let i=0;i<4;i++)globalThis.gc();extra.collected=weak.deref()===undefined;`,{gc:true})
  complete(data,1);assert.equal(data.collected,true,'the Inspector console must retain only a cleared diagnostic wrapper')
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
