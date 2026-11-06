import test from 'node:test'
import assert from 'node:assert/strict'
import {EventEmitter} from 'node:events'
import {observeModuleMetadata} from './worker-module-metadata-probe.mjs'
import {observeTransportMetadata} from './worker-transport-metadata-probe.mjs'

const fetchEvent={type:'custom',event:'vite:invoke',data:{name:'fetchModule',data:[]}}
function setup(Transport=class {invoke(){return Promise.resolve({result:{}})}}) {
  class Graph {ensureModule(){} invalidateModule(){}}
  const metadata=observeModuleMetadata(Graph),transport=observeTransportMetadata(Transport,metadata)
  return {metadata,transport,close(){transport.close();metadata.close()}}
}

test('observes IPC metadata before later consumers but retains the earlier-alias limitation',()=>{
  const p=setup(),emitter=new EventEmitter(),raw={tmp:'/private-path',invalidate:true}
  let early,later
  emitter.on('message',message=>{early=message.r})
  emitter.on('message',p.transport.received)
  emitter.on('message',message=>{later=message.r;assert.equal(message.r.tmp,'/private-path')})
  p.transport.sent({t:'q',m:'fetch',i:'one',a:[]})
  emitter.emit('message',{t:'s',i:'one',r:raw})
  assert.equal(early,raw);assert.notEqual(later,raw)
  let r=p.metadata.report()
  assert(r.events.some(e=>e.key==='tmp'&&e.operation==='get'&&e.origins.includes('rpc:fetch')))
  const events=JSON.stringify(r.events)
  assert.equal(early.tmp,'/private-path')
  assert.equal(JSON.stringify(p.metadata.report().events),events)
  assert(!JSON.stringify(p.transport.report()).includes('/private-path'))
  assert.equal(p.transport.report().pending,0);assert.deepEqual(p.transport.report().errors,[])
  assert.equal(p.transport.report().completeOwnership,false)
  p.close()
})

test('warm metadata retains shared aliases and descriptors; error responses are not wrapped',()=>{
  const p=setup(),raw={tmp:'/temporary',invalidate:false},warm=Object.create(null)
  Object.defineProperty(warm,'a',{value:raw,enumerable:false,writable:true,configurable:false})
  warm.b=raw
  p.transport.sent({t:'q',m:'fetchWarmModules',i:'warm'})
  const packet={t:'s',i:'warm',r:warm};p.transport.received(packet)
  assert.equal(packet.r,warm);assert.equal(warm.a,warm.b);assert.notEqual(warm.a,raw)
  assert.deepEqual(Object.getOwnPropertyDescriptor(warm,'a'),{value:warm.a,enumerable:false,writable:true,configurable:false})
  assert.equal(warm.a.invalidate,false)
  p.transport.sent({t:'q',m:'fetch',i:'failed'})
  const failed={t:'s',i:'failed',e:Error('expected')};p.transport.received(failed)
  assert(!Object.hasOwn(failed,'r'))
  assert.deepEqual(p.transport.report().counts,{requests:2,responses:2,errorResponses:1,rpcMetadata:2,
    invocations:0,settled:0,transportMetadata:0,rejections:0})
  p.close()
})

test('transport observes derived metadata before the awaiting caller and preserves the original promise',async()=>{
  const raw={tmp:'/temporary',invalidate:true},response={result:raw},promise=Promise.resolve(response)
  let receiver,event
  class Transport {invoke(value){receiver=this;event=value;return promise}}
  const original=Transport.prototype.invoke,p=setup(Transport),instance=new Transport()
  const returned=instance.invoke(fetchEvent)
  assert.equal(returned,promise);assert.equal(receiver,instance);assert.equal(event,fetchEvent)
  assert.equal(await returned,response)
  assert.notEqual(response.result,raw);assert.equal(response.result.invalidate,true)
  assert(p.metadata.report().events.some(e=>e.key==='invalidate'&&e.origins.includes('transport')))
  assert.equal(p.transport.report().counts.transportMetadata,1)
  assert.equal(p.transport.report().unsettled,0);assert.deepEqual(p.transport.report().errors,[])
  p.close();assert.equal(Transport.prototype.invoke,original)
})

test('original throws and rejections remain intact; unknown thenables are reported without invoking them',async()=>{
  const failure=Error('expected'),rejection=Promise.reject(failure)
  let thenCalls=0
  const thenable={then(){thenCalls++}}
  class Transport {invoke(event) {
    if(event.mode==='throw')throw failure
    return event.mode==='reject'?rejection:thenable
  }}
  const p=setup(Transport),t=new Transport()
  assert.throws(()=>t.invoke({...fetchEvent,mode:'throw'}),e=>e===failure)
  const result=t.invoke({...fetchEvent,mode:'reject'});assert.equal(result,rejection)
  await assert.rejects(result,e=>e===failure)
  assert.equal(t.invoke(fetchEvent),thenable);assert.equal(thenCalls,0)
  assert.equal(p.transport.report().counts.rejections,1)
  assert(p.transport.report().errors.includes('transport did not return a native Promise'))
  assert.equal(p.transport.report().replayEligible,false)
  p.close()
})

test('packet accessors and fixed response slots fail explicitly without calling application getters',()=>{
  const p=setup();let getters=0
  p.transport.sent({get t(){getters++;return 'q'}})
  p.transport.sent({t:'q',m:'fetch',i:'one'})
  p.transport.sent({t:'q',m:'fetch',i:'one'})
  const packet={t:'s',i:'one'}
  Object.defineProperty(packet,'r',{value:{tmp:'/private'},writable:false})
  p.transport.received(packet)
  assert.equal(getters,0)
  assert.deepEqual(p.transport.report().errors,['transport accessor','duplicate module request identifier','unwritable transport metadata'])
  assert.equal(p.transport.report().pending,0);assert.equal(p.transport.report().replayEligible,false)
  p.close()
})
