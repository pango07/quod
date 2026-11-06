import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
const strings=new URL('./native-string-transport.mjs',import.meta.url).href

// Run with GC exposed remotely. These are lifetime tests, not replay proof.
function fixture(body) {
  const source=`import {Session} from 'node:inspector';
import vm from 'node:vm';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const {withInspectorContext:guard}=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON);
const session=new Session(),contexts=[];
session.connect();session.on('Runtime.executionContextCreated',({params})=>contexts.push(params.context));
function post(method,params={}) {let done=false,error,result;session.post(method,params,(e,r)=>{done=true;error=e;result=r});if(!done||error)throw error??Error('async Inspector');return result}
post('Runtime.enable');
${body}
session.disconnect();console.log('verified');`
  const r=spawnSync(process.execPath,['--expose-gc','--input-type=module','-e',source],{encoding:'utf8',timeout:20000,maxBuffer:1024*1024})
  assert.equal(r.status,0,r.stderr||String(r.error));assert.equal(r.stdout.trim(),'verified')
}

test('context guard validates inputs and propagates callback failures',()=>fixture(`
const id=contexts.find(c=>c.auxData.isDefault).id;
let calls=0;
assert.equal(guard(id,()=>{calls++}),true);assert.equal(calls,1);
assert.equal(guard(2147483647,()=>{calls++}),false);assert.equal(calls,1);
for(const bad of [0,-1,1.5,NaN,'1',null,2147483648])assert.throws(()=>guard(bad,()=>{}));
assert.throws(()=>guard(id,null));
const marker=new Error('callback');assert.throws(()=>guard(id,()=>{throw marker}),e=>e===marker);
`))

test('context guard retains a foreign realm across nested collection and releases it afterward',()=>fixture(`
vm.runInNewContext('42');
const id=contexts.find(c=>!c.auxData.isDefault).id;
assert.equal(guard(id,()=>{
  for(let i=0;i<3;i++)globalThis.gc();
  const value=post('Runtime.evaluate',{expression:'40+2',contextId:id,returnByValue:true});
  assert.equal(value.result.value,42);
}),true);
await new Promise(resolve=>setImmediate(resolve));
for(let i=0;i<3;i++)globalThis.gc();
let called=false;assert.equal(guard(id,()=>{called=true}),false);assert.equal(called,false);
`))

test('context guard refuses a collected realm before its destruction notification arrives',()=>fixture(`
vm.runInNewContext('42');
const id=contexts.find(c=>!c.auxData.isDefault).id;
for(let i=0;i<3;i++)globalThis.gc();
let called=false;assert.equal(guard(id,()=>{called=true}),false);assert.equal(called,false);
`))

test('primitive transport retires collected realms and still installs a later live realm',()=>fixture(`
const {nativeStringTransport}=await import(${JSON.stringify(strings)});
const bridge=nativeStringTransport(post,'a'.repeat(64),{onSummary(){}});bridge.initialize();
bridge.contextCreated(contexts.find(c=>c.auxData.isDefault));
vm.runInNewContext('42');
const dead=contexts.find(c=>!c.auxData.isDefault);
for(let i=0;i<3;i++)globalThis.gc();
bridge.contextCreated(dead);
assert.equal(bridge.report().retiredContexts,1);assert.equal(bridge.report().failures,0);
assert.equal(bridge.report().contextQueries,0);
const live=vm.createContext({});
const current=contexts.at(-1);assert.notEqual(current.id,dead.id);
bridge.contextCreated(current);
assert.equal(vm.runInContext('40+2',live),42);
assert.equal(bridge.report().contextQueries,1);assert.equal(bridge.report().failures,0);
assert.equal(bridge.report().contexts,2);assert.equal(bridge.report().replayEligible,false);
bridge.contextDestroyed(current.id);assert.equal(bridge.report().contexts,1);
bridge.contextsCleared();assert.equal(bridge.report().failures,1);assert.equal(bridge.report().contexts,0);
bridge.close();
`))
