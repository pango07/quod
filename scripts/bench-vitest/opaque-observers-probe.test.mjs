import {test} from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const module=new URL('./promise-observers-probe.mjs',import.meta.url).href
function run(kind) {
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
import workers from 'node:worker_threads';import children from 'node:child_process';import {once} from 'node:events';
const kind=${JSON.stringify(kind)},CachedWorker=workers.Worker,cachedDlopen=process.dlopen,cachedLinked=process._linkedBinding;
let worker=null;
if(kind==='existing') {worker=new CachedWorker('setInterval(()=>{},1000)',{eval:true,execArgv:[]});await once(worker,'online')}
const p=inspectPromiseObservers(),before=p.opaqueAccess();
if(kind==='worker') {
 workers.Worker=function(){throw Error('public constructor replaced')};
 worker=new CachedWorker('setInterval(()=>{},1000)',{eval:true,execArgv:[]});await once(worker,'online');
}
let loadFailed=false;
if(kind==='dlopen') {process.dlopen=function(){throw Error('public function replaced')};try{Reflect.apply(cachedDlopen,process,[{exports:{}},'/nonexistent-quod-native-fixture.node'])}catch{loadFailed=true}}
if(kind==='linked') {process._linkedBinding=function(){throw Error('public function replaced')};try{Reflect.apply(cachedLinked,process,['nonexistent_quod_native_fixture'])}catch{loadFailed=true}}
if(kind==='spawn'){const child=children.spawn(process.execPath,['--version'],{stdio:'ignore'});await once(child,'exit')}
if(kind==='sync'){const r=children.spawnSync(process.execPath,['--version']);if(r.status!==0)throw Error('sync child failed')}
if(worker)await worker.terminate();
const after=p.opaqueAccess(),snapshot=p.opaqueSnapshot();p.close();
console.log(JSON.stringify({before,after,snapshot,loadFailed}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:15000})
  assert.equal(r.status,0,r.stderr||String(r.error));return JSON.parse(r.stdout)
}
let baselineCount=null
for(const kind of ['none','existing','worker','dlopen','linked','spawn','sync']) test('opaque native entry diagnostic: '+kind,t=>{
  const r=run(kind)
  assert.equal(r.before.available,true,JSON.stringify(r.before))
  assert.equal(r.after.available,true,JSON.stringify(r.after))
  assert.equal(r.snapshot.replayEligible,false)
  if(kind==='none')baselineCount=r.before.initial.nativeWorkerObjects
  assert.ok(Number.isInteger(baselineCount)&&baselineCount>=0)
  // A single public Worker creates more than one native-prototype object.
  // This is an object census, not a count of active threads.
  if(kind==='existing')assert.ok(r.before.initial.nativeWorkerObjects>baselineCount)
  else assert.equal(r.before.initial.nativeWorkerObjects,baselineCount)
  assert.deepEqual(r.after.entries.flatMap(e=>e.entries),kind==='worker'?['Worker','Worker.startThread']:
    kind==='dlopen'?['dlopen']:kind==='linked'?['linkedBinding']:kind==='spawn'?['Process.spawn']:kind==='sync'?['spawnSync']:[])
  assert.equal(r.loadFailed,kind==='dlopen'||kind==='linked')
  t.diagnostic(JSON.stringify({kind,initial:r.before.initial,counts:r.snapshot.counts}))
})

test('a worker can connect to the main Inspector without a main-isolate connection entry',()=>{
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
import {Worker} from 'node:worker_threads';import {once} from 'node:events';
const p=inspectPromiseObservers();globalThis.__quodWorkerInspectorFixture='main-only';
const worker=new Worker(\`const {parentPort}=require('node:worker_threads');
// Inspector requests to another isolate do not keep this worker alive.
const keepAlive=setInterval(()=>{},1000);
const s=new(require('node:inspector').Session)();s.connectToMainThread();
s.post('Runtime.evaluate',{expression:'globalThis.__quodWorkerInspectorFixture'},(e,r)=>{
 parentPort.postMessage({error:e?.message??null,value:r?.result?.value});s.disconnect();clearInterval(keepAlive);
});\`,{eval:true,execArgv:[]});
const [message]=await once(worker,'message');await worker.terminate();
const inspector=p.inspectorAccess(),opaque=p.opaqueSnapshot();p.close();delete globalThis.__quodWorkerInspectorFixture;
console.log(JSON.stringify({message,inspector,opaque}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:15000})
  assert.equal(r.status,0,r.stderr||String(r.error));const data=JSON.parse(r.stdout)
  assert.deepEqual(data.message,{error:null,value:'main-only'})
  assert.deepEqual(data.inspector.entries,[],'a worker connection bypasses main-isolate Connection call breakpoints')
  assert.equal(data.opaque.available,true)
  assert.equal(data.opaque.counts['Worker.startThread'],1,'the earlier worker start still records the capability')
  assert.equal(data.opaque.replayEligible,false)
})
