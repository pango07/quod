import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {readFileSync} from 'node:fs'
import {createHash} from 'node:crypto'

const registryUrl=new URL('./promise-observers-probe.mjs',import.meta.url).href
const transportUrl=new URL('./worker-transport-metadata-probe.mjs',import.meta.url).href
const viteUrl=new URL('./fixtures/vite-module-runner.mjs',import.meta.url).href
assert.equal(createHash('sha256').update(readFileSync(new URL(viteUrl))).digest('hex'),
  '9e3f0bc4ea6d01e55bb9a73ee3666c750e139340bb84fa46e35f92085f2535b7')

function run(mode='consumer',kind='normal') {
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(registryUrl)};
import {observeTransportMetadata} from ${JSON.stringify(transportUrl)};
import {ModuleRunner} from ${JSON.stringify(viteUrl)};
import {promiseHooks} from 'node:v8';
const mode=${JSON.stringify(mode)},kind=${JSON.stringify(kind)};
const observed=[],order=[],promiseEvents=[],ids=new Map(),resolvers=[],trapEvents=[];
let gets=0,traps=0,unhandled=0,lastPromise=null;
const failure=new Error('fixture rejection');
const raw={invalidate:true,tmp:'/private-continuation'};
const response=kind==='error'?{error:failure}:kind==='accessor'?{get result(){gets++;return raw}}:
  kind==='proxy'?new Proxy({result:raw},{has(target,key){traps++;trapEvents.push(['has',String(key)]);return Reflect.has(target,key)},get(target,key){traps++;trapEvents.push(['get',String(key)]);return Reflect.get(target,key)}}):{result:raw};
class Transport {invoke(event){
  order.push('invoke:'+event.data.data[0]);
  lastPromise=kind==='rejected'||kind==='orphan'?Promise.reject(failure):kind==='out-of-order'?
    new Promise(resolve=>resolvers.push(resolve)):Promise.resolve(response);
  return lastPromise;
}}
const metadata={observeMetadata(value){if(kind==='callback-throw')throw Error('fixture callback failure');observed.push(value===raw?'raw':value.id);return value}};
const p=inspectPromiseObservers({compiledScripts:true});
const transport=mode==='disabled'?{received(){},report(){return null}}:observeTransportMetadata(Transport,metadata,{delivery:mode});
function observer(message){transport.received(message)}
process.prependListener('message',observer);p.captureMessageObserver();
const initial=mode==='consumer'?p.captureTransportContinuation(${JSON.stringify(viteUrl)}+(kind==='missing-source'?'.missing':'')):null;
const instance=new Transport(),runner=new ModuleRunner({transport:instance,hmr:false,sourcemapInterceptor:false});
if(kind==='closed')p.close();
const id=value=>{if(!ids.has(value))ids.set(value,ids.size+1);return ids.get(value)};
const stop=promiseHooks.createHook({init(value,parent){promiseEvents.push(['init',id(value),parent?id(parent):0])},
  before(value){promiseEvents.push(['before',id(value)])},after(value){promiseEvents.push(['after',id(value)])},settled(value){promiseEvents.push(['settled',id(value)])}});
let value,error=null,samePromise=null;
if(kind==='orphan') {
  process.on('unhandledRejection',()=>unhandled++);
  instance.invoke({type:'custom',event:'vite:invoke',data:{name:'fetchModule',data:['orphan']}});
  await new Promise(resolve=>setImmediate(resolve));
}else if(kind==='identity') {
  const result=instance.invoke({type:'custom',event:'vite:invoke',data:{name:'fetchModule',data:['direct']}});
  samePromise=result===lastPromise;value=await result;
}else if(kind==='out-of-order') {
  const a=runner.transport.invoke('fetchModule',['a']),b=runner.transport.invoke('fetchModule',['b']);
  resolvers[1]({result:{id:'b'}});resolvers[0]({result:{id:'a'}});
  value=await Promise.all([a,b]);order.push('received:'+value.map(v=>v.id).join(','));
}else {
  const result=runner.transport.invoke('fetchModule',['one']);
  queueMicrotask(()=>order.push('queued'));
  try{value=await result;order.push('received')}catch(e){error=e.message;order.push('rejected')}
}
stop();
const report=p.transportContinuation(),transportReport=transport.report(),compilation=p.compiledScriptReport();
p.close();transport.close?.();process.removeListener('message',observer);
console.log(JSON.stringify({initial,report,transport:transportReport,observed,order,promiseEvents,gets,traps,trapEvents,unhandled,samePromise,
  rawIdentity:value===raw,error,compilationErrors:compilation.errors,closed:p.transportContinuation()}));`
  const result=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(result.status,0,result.stderr||String(result.error))
  return JSON.parse(result.stdout)
}

test('existing Vite continuation preserves Promise lifecycle and application ordering',()=>{
  const plain=run('disabled'),consumer=run(),reaction=run('reaction')
  assert.deepEqual(consumer.order,plain.order)
  assert.deepEqual(consumer.promiseEvents,plain.promiseEvents,'no additional Promise or reaction may be introduced')
  assert.notDeepEqual(reaction.promiseEvents,plain.promiseEvents,'the control must detect the legacy extra reaction')
  assert.equal(consumer.initial.available,true,JSON.stringify(consumer.initial))
  assert.equal(consumer.report.available,true,JSON.stringify(consumer.report))
  assert.equal(consumer.report.entries,1);assert.equal(consumer.report.delivered,1)
  assert.equal(consumer.report.groupsCreated,consumer.report.groupsReleased)
  assert.deepEqual(consumer.observed,['raw']);assert.equal(consumer.rawIdentity,true)
  assert.equal(consumer.transport.addsPromiseReaction,false)
  assert.equal(consumer.transport.counts.rejections,null)
  assert.equal(consumer.transport.unsettled,0)
  assert.deepEqual(consumer.compilationErrors,[])
  assert.equal(consumer.closed.closed,true)
})

test('concurrent deliveries preserve ordering without claiming per-Promise ownership',()=>{
  const plain=run('disabled','out-of-order'),r=run('consumer','out-of-order')
  assert.deepEqual(r.order,plain.order);assert.deepEqual(r.promiseEvents,plain.promiseEvents)
  assert.deepEqual(r.observed,['b','a']);assert.equal(r.report.delivered,2)
  assert.equal(r.transport.counts.invocations,2);assert.equal(r.transport.unsettled,0)
  assert.equal(r.report.completeOwnership,false);assert.equal(r.report.replayEligible,false)
})

for(const kind of ['error','rejected','identity','orphan','accessor','proxy','callback-throw','missing-source','closed']) {
  test('transport continuation: '+kind,()=>{
    const plain=run('disabled',kind),r=run('consumer',kind)
    assert.deepEqual(r.order,plain.order)
    assert.deepEqual(r.promiseEvents,plain.promiseEvents)
    assert.equal(r.error,plain.error);assert.equal(r.rawIdentity,plain.rawIdentity)
    assert.equal(r.gets,plain.gets);assert.equal(r.traps,plain.traps)
    assert.deepEqual(r.trapEvents,plain.trapEvents)
    assert.equal(r.report.replayEligible,false);assert.equal(r.report.completeOwnership,false)
    assert.equal(r.report.groupsCreated,r.report.groupsReleased)
    assert(!JSON.stringify(r.report).includes('/private-continuation'))
    if(kind==='error') {
      assert.equal(r.report.available,true);assert.equal(r.report.delivered,1)
      assert.equal(r.transport.unsettled,0);assert.deepEqual(r.observed,[])
    }
    if(['rejected','identity','orphan','missing-source','closed'].includes(kind)) {
      assert.equal(r.report.delivered,0);assert.equal(r.transport.unsettled,1)
      if(kind==='identity')assert.equal(r.samePromise,true)
      if(kind==='orphan'){assert.equal(r.unhandled,1);assert.equal(plain.unhandled,1)}
    }
    if(kind==='accessor'){assert.equal(r.gets,1);assert.equal(r.report.available,false)}
    if(kind==='proxy') {
      // Promise.resolve first checks `then`; Vite subsequently tests `error`
      // and reads `result`. Observation must add no access to that sequence.
      assert.deepEqual(r.trapEvents,[['get','then'],['has','error'],['get','result']])
      assert.equal(r.report.available,false)
    }
    if(kind==='callback-throw')assert.equal(r.report.available,false)
    if(kind==='missing-source')assert.equal(r.initial.available,false)
    if(kind==='closed')assert.equal(r.report.closed,true)
  })
}
