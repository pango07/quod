import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {parse} from '../../crates/quod-cli/src/vitest/acorn.mjs'
import {privateModuleCallees} from './private-module-callees.mjs'

const observer=new URL('./promise-observers-probe.mjs',import.meta.url).href
function fixture(body,vm=false) {
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(observer)};
let gets=0;const target={get tmp(){gets++;return 'not-read'}},plain={tmp:7},key='tmp';
${body}`
  const r=spawnSync(process.execPath,[...(vm?['--experimental-vm-modules']:[]),'--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error))
  const data=JSON.parse(r.stdout)
  assert.equal(data.gets,0);assert.equal(data.report.available,true);assert.equal(data.report.overflow,false)
  assert.equal(data.report.replayEligible,false)
  return data
}

test('private callee guard rejects mutation, escape, eval, duplicate declarations and shadowing',()=>{
  const check=source=>[...privateModuleCallees(parse(source,{ecmaVersion:'latest',sourceType:'module'}))]
  assert.deepEqual(check('var native=Object.defineProperty; function invoke(v){native(v,"x",{})}'),['native'])
  for(const extra of [
    'native=Reflect.defineProperty;', 'const escape=()=>native;', 'export {native};',
    'eval("native=Reflect.defineProperty");', 'var native;', 'const x={native};',
    'function shadow(native){native({})}', 'native?.({});',
  ])assert(!check('var native=Object.defineProperty;'+extra).includes('native'),extra)
})

test('a verified private callee avoids repeated module reads while targets remain fresh',()=>{
  const data=fixture(`const native=Object.getOwnPropertyDescriptor;
function invoke(target,key){return native(target,key)}
const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:true,privateCalleeCache:true});
const a=invoke(target,key),b=invoke(plain,key),c=invoke(target,key);
const report=p.descriptorAccess(),identity=p.nativeIdentity();p.close();
console.log(JSON.stringify({report,identity,gets,values:[typeof a.get,b.value,typeof c.get]}));`)
  assert.deepEqual(data.values,['function',7,'function'])
  assert.equal(data.report.calls,3)
  assert(data.report.entries.every(e=>e.argument.available),JSON.stringify(data.report))
  assert(data.report.entries.some(e=>e.argument.target.watchedAccessorKeys.length===0))
  assert(data.report.entries.some(e=>e.argument.target.watchedAccessorKeys.includes('tmp')))
  assert.equal(data.report.argumentDiagnostics.privateCalleeCacheHits,2)
  assert.equal(data.report.argumentDiagnostics.propertyRequests.module,1)
  assert.equal(data.identity.comparisons,1)
})

test('another Inspector connection disables reuse of a previously verified private callee',()=>{
  const data=fixture(`import {Session} from 'node:inspector';
const native=Object.getOwnPropertyDescriptor;
function invoke(target,key){return native(target,key)}
const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:true,privateCalleeCache:true});
invoke(target,key);const other=new Session();other.connect();invoke(plain,key);
const report=p.descriptorAccess(),inspector=p.inspectorAccess();p.close();other.disconnect();
console.log(JSON.stringify({report,inspector,gets}));`)
  assert.equal(data.report.calls,2)
  assert(data.report.entries.every(e=>e.argument.available),JSON.stringify(data.report))
  assert(data.inspector.entries.length>0)
  assert.equal(data.report.argumentDiagnostics.privateCalleeCacheHits,0)
  assert.equal(data.report.argumentDiagnostics.propertyRequests.module,2)
})

test('same-source cached modules in distinct contexts cannot share a callee identity',()=>{
  const data=fixture(`import vm from 'node:vm';
const source='const native=globalThis.chosen; export function invoke(target,key){return native(target,key)}';
const a=new vm.SourceTextModule(source,{context:vm.createContext({chosen:Object.getOwnPropertyDescriptor}),identifier:'quod-same-module'});
const cachedData=a.createCachedData();await a.link(()=>{});await a.evaluate();
const b=new vm.SourceTextModule(source,{context:vm.createContext({chosen:Object.getOwnPropertyDescriptor.bind(null,target)}),identifier:'quod-same-module',cachedData});
await b.link(()=>{});await b.evaluate();
const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:true,privateCalleeCache:true});
const left=a.namespace.invoke(target,key),right=b.namespace.invoke(key,'ignored');
const report=p.descriptorAccess();p.close();
console.log(JSON.stringify({report,gets,found:[typeof left.get==='function',typeof right.get==='function']}));`,true)
  assert.deepEqual(data.found,[true,true]);assert.equal(data.report.calls,2)
  assert.equal(new Set(data.report.entries.map(e=>e.frames[0].location.scriptId)).size,2,
    'the pinned runtime must distinguish the actual module scripts, including cached code')
  assert.equal(data.report.entries.filter(e=>e.argument.available).length,1)
  assert.equal(data.report.entries.filter(e=>!e.argument.available).length,1)
  assert.equal(data.report.argumentDiagnostics.privateCalleeCacheHits,0)
})
