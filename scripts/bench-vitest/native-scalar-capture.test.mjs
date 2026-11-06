import test from 'node:test'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {getCallSites} from 'node:util'
import {spawnSync} from 'node:child_process'
import vm from 'node:vm'

const addon=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
const observe=addon.createScalarReadObserver
function make(value,enumerable=true,prototype=Object.prototype) {
  if(arguments.length===0)value=17
  return Object.defineProperty(Object.create(prototype),'tmp',{value,writable:true,enumerable,configurable:true})
}

test('native scalar reads preserve primitive identity, descriptors and aliases',()=>{
  for(const value of [undefined,null,false,true,0,-0,NaN,Infinity,-Infinity,17,42n,'','\u0000\u03bb\ud800',Symbol('scalar')])
    for(const enumerable of [false,true])for(const prototype of [null,Object.prototype]) {
      const target=make(value,enumerable,prototype),before=Object.getOwnPropertyDescriptor(target,'tmp')
      let reads=0;const observer=observe(target,'tmp',()=>reads++)
      assert(observer.report().available);assert.equal(reads,0,'integrity checks must not notify the read sink')
      assert.deepEqual(observer.currentValue(),{available:true,value,enumerable,writable:true,configurable:true})
      assert.equal(reads,0,'private scalar snapshots must not be reported as application reads')
      assert(Object.is(target.tmp,value));assert.equal(reads,1)
      assert.deepEqual(Object.getOwnPropertyDescriptor(target,'tmp'),before);assert.equal(reads,2)
      assert(Object.is(Reflect.get(target,'tmp',{}),value));assert.equal(reads,3)
      assert(Object.is(Object.create(target).tmp,value));assert.equal(reads,4)
      assert.equal(Object.prototype.__lookupGetter__.call(target,'tmp'),undefined)
      assert.equal(Object.prototype.__lookupSetter__.call(target,'tmp'),undefined)
      assert.equal(reads,4);assert.equal(observer.report().calls,4)
      observer.close();const report=observer.report()
      assert(report.closed&&report.restored&&!report.retainedAfterClose&&!report.failed&&!report.replayEligible)
      assert.deepEqual(Object.getOwnPropertyDescriptor(target,'tmp'),before)
      assert(Object.is(target.tmp,value));assert.equal(reads,4)
    }
})

test('native scalar callback loss is detected even when the original value is restored',()=>{
  for(const mutate of [
    t=>{t.tmp=17},t=>{t.tmp=99;t.tmp=17},
    t=>{const d=Object.getOwnPropertyDescriptor(t,'tmp');delete t.tmp;Object.defineProperty(t,'tmp',d)},
    t=>Object.defineProperty(t,'tmp',{value:17}),
    t=>Object.defineProperty(t,'tmp',{writable:false}),
    t=>Object.defineProperty(t,'tmp',{configurable:false}),
    t=>{delete t.tmp},
  ]) {
    const target=make();let reads=0;const observer=observe(target,'tmp',()=>reads++)
    mutate(target);const before=Object.getOwnPropertyDescriptor(target,'tmp'),count=reads,report=observer.report()
    assert.equal(report.intact,false);assert.equal(report.available,false);assert.equal(reads,count)
    assert.deepEqual(observer.currentValue(),{available:false});assert.equal(reads,count)
    observer.close();assert.deepEqual(Object.getOwnPropertyDescriptor(target,'tmp'),before)
    assert.equal(observer.report().restored,false,'closing must not overwrite replacement application data')
  }
})

test('native scalar integrity checks never invoke a replacement JavaScript accessor',()=>{
  const target=make();let reads=0,getters=0
  const observer=observe(target,'tmp',()=>reads++)
  const getter=()=>{getters++;return 17}
  Object.defineProperty(target,'tmp',{get:getter,configurable:true})
  const before=reads
  assert.equal(observer.report().available,false);assert.equal(reads,before);assert.equal(getters,0)
  assert.deepEqual(observer.currentValue(),{available:false});assert.equal(getters,0)
  observer.close();assert.equal(Object.getOwnPropertyDescriptor(target,'tmp').get,getter)
  assert.equal(getters,0)
})

test('native scalar setup rejects unknown owners and callbacks before reading application values',()=>{
  let traps=0,gets=0
  const proxy=new Proxy(make(),{getOwnPropertyDescriptor(){traps++;throw Error('trap')}})
  const getter={get tmp(){gets++;throw Error('getter')}}
  const native=addon.createNativeDataFixture()
  for(const target of [proxy,getter,native.target,Object.create({tmp:17}),{},Object.freeze(make()),{tmp:{}},{tmp:()=>{}}])
    assert.throws(()=>observe(target,'tmp',()=>{}),/unsupported native scalar/)
  assert.equal(traps,0);assert.equal(gets,0);assert.equal(native.state.calls,0)
})

test('native scalar sink errors, nested reads and reentrant close preserve the application value and fail observation',()=>{
  for(const operation of ['throw','nested','close','report']) {
    const target=make();let observer,sinks=0
    observer=observe(target,'tmp',()=>{
      sinks++
      if(operation==='throw')throw Error('private diagnostic error')
      if(operation==='nested')assert.equal(target.tmp,17)
      if(operation==='close')observer.close()
      if(operation==='report')observer.report()
    })
    assert.equal(target.tmp,17);assert.equal(sinks,1)
    const report=observer.report();assert(report.failed&&!report.available&&!report.replayEligible)
    assert(!JSON.stringify(report).includes('private diagnostic error'))
    assert.equal(target.tmp,17);assert.equal(sinks,1,'failed observation must not repeatedly run the sink')
    observer.close();assert.equal(target.tmp,17);assert(observer.report().closed)
  }
})

test('native scalar sinks preserve queued microtasks and retain the actual JS caller',async()=>{
  const target=make();let microtasks=0,frames
  const observer=observe(target,'tmp',()=>{frames=getCallSites(100,{sourceMap:false})})
  queueMicrotask(()=>microtasks++)
  function readActual(){return target.tmp}
  assert.equal(readActual(),17);assert.equal(microtasks,0)
  assert(frames.some(f=>f.functionName==='readActual'&&typeof f.scriptId==='string'))
  observer.close();await Promise.resolve();assert.equal(microtasks,1)
})

test('native scalar installation and reads work across foreign realms without changing the descriptor',()=>{
  const context=vm.createContext({}),target=vm.runInContext('({tmp:17})',context)
  const before=Object.getOwnPropertyDescriptor(target,'tmp');let reads=0
  const observer=observe(target,'tmp',()=>reads++)
  context.target=target
  assert.equal(vm.runInContext('target.tmp',context),17)
  assert.equal(reads,1);assert.deepEqual(Object.getOwnPropertyDescriptor(target,'tmp'),before)
  assert(observer.report().available);observer.close()
  assert.deepEqual(Object.getOwnPropertyDescriptor(target,'tmp'),before)
})

test('frozen scalar owners keep their flags and closed callbacks stop retaining the JS sink',()=>{
  const target=make();let reads=0;const observer=observe(target,'tmp',()=>reads++)
  Object.freeze(target)
  assert.equal(Reflect.set(target,'tmp',99),false);assert.equal(target.tmp,17)
  const before=Object.getOwnPropertyDescriptor(target,'tmp'),count=reads
  assert(observer.report().available);observer.close()
  const report=observer.report()
  assert(report.closed&&!report.restored&&report.retainedAfterClose&&!report.failed)
  assert.deepEqual(Object.getOwnPropertyDescriptor(target,'tmp'),before);assert.equal(target.tmp,17)
  assert.equal(reads,count)
})

test('native scalar owner/sink cycles collect and closing a frozen owner releases captured sink values',()=>{
  const source=`import {createRequire} from 'node:module';
const addon=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON);
function cycle(){const owner={tmp:17};const observer=addon.createScalarReadObserver(owner,'tmp',()=>owner.tmp);return [new WeakRef(owner),new WeakRef(observer)]}
function retained(){const owner={tmp:17},captured={};const observer=addon.createScalarReadObserver(owner,'tmp',()=>captured);
Object.freeze(owner);observer.close();return {owner,observer,reference:new WeakRef(captured)}}
const references=cycle(),closed=retained();await new Promise(resolve=>setImmediate(resolve));
for(let i=0;i<5;i++)globalThis.gc();
console.log(JSON.stringify({collected:references.map(r=>r.deref()===undefined),sinkCollected:closed.reference.deref()===undefined,
value:closed.owner.tmp,report:closed.observer.report()}));`
  const r=spawnSync(process.execPath,['--expose-gc','--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error));const result=JSON.parse(r.stdout)
  assert.deepEqual(result.collected,[true,true]);assert.equal(result.sinkCollected,true)
  assert.equal(result.value,17);assert(result.report.closed&&result.report.retainedAfterClose)
})
