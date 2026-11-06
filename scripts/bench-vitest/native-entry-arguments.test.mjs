// Diagnostic experiment only. No production key or replay decision uses this.
import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

function fixture(body) {
  const source=`import {Session} from 'node:inspector';
import vm from 'node:vm';
const session=new Session(),errors=[],pauses=[],events=[];
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
${body}
post('Runtime.releaseObjectGroup',{objectGroup:'quod-entry-fixture'});session.disconnect();delete globalThis.__quodEntryFixture;
console.log(JSON.stringify({gets,memberGets,pauses:pauses.length,events,errors}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error))
  const data=JSON.parse(r.stdout)
  assert.equal(data.gets,0);assert.deepEqual(data.errors,[])
  return data
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
