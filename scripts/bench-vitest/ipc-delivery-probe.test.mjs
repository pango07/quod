import test from 'node:test'
import assert from 'node:assert/strict'
import {spawn} from 'node:child_process'
import {DefaultSerializer} from 'node:v8'
import {types} from 'node:util'

// Put both protocol frames in one pipe write. Two child.send calls can reach
// two otherwise identical processes in different read batches, which does
// not define a stable cross-run order for decoding versus message callbacks.
function ipcFrames(messages,serialization) {
  if(serialization==='json')return Buffer.from(messages.map(m=>JSON.stringify(m)+'\n').join(''))
  class FixtureSerializer extends DefaultSerializer {
    _writeHostObject(value) {
      const view=types.isArrayBufferView(value)
      this.writeUint32(view?0:1)
      if(view)return super._writeHostObject(value)
      this.writeValue({...value})
    }
  }
  return Buffer.concat(messages.map(message=>{
    const serializer=new FixtureSerializer();serializer.writeHeader();serializer.writeValue(message)
    const body=serializer.releaseBuffer(),length=Buffer.alloc(4);length.writeUInt32BE(body.length)
    return Buffer.concat([length,body])
  }))
}
function ipcPipe(process) {
  const symbol=Object.getOwnPropertySymbols(process).find(s=>s.description==='kChannelHandle')
  assert(symbol,'fixture requires the pinned Node IPC channel')
  return process[symbol]
}

const registryUrl=new URL('./promise-observers-probe.mjs',import.meta.url).href
async function run(kind,enabled,serialization='advanced') {
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(registryUrl)};
import v8 from 'node:v8';
const enabled=${enabled},kind=${JSON.stringify(kind)};
const p=inspectPromiseObservers({compiledScripts:true}),preparedMessages=new WeakSet();
const values=[],order=[],copies=[],deliveryEvents=[];
let preparations=0,gets=0,readId=0,activeRead=0;
const pipe=process[Object.getOwnPropertySymbols(process).find(s=>s.description==='kChannelHandle')];
const onread=pipe.onread;
pipe.onread=function(...args){activeRead=++readId;try{return Reflect.apply(onread,this,args)}finally{activeRead=0}};
function prepareWorkerMessage(message){
  if(kind==='callback-throw')throw Error('fixture preparation failure');
  if(message&&typeof message==='object')preparedMessages.add(message);
  preparations++;return true;
}
function copy(message){copies.push({prepared:message&&typeof message==='object'&&preparedMessages.has(message),value:message.value});order.push('copy');deliveryEvents.push({operation:'copy',value:message.value,read:activeRead})}
function observer(message){
  if(!enabled&&kind!=='callback-throw')prepareWorkerMessage(message);
  values.push(typeof message==='object'?message.value:message);order.push('consumer');
  deliveryEvents.push({operation:'consumer',value:values.at(-1),read:activeRead});
  if(values.length===(kind==='synthetic'?3:2)) {
    const report=p.ipcDelivery(),dispatch=p.messageDispatch(),compilation=p.compiledScriptReport();
    p.close();process.removeListener('message',observer);
    process.send({kind:'report',report,dispatch,preparations,gets,values,order,copies,deliveryEvents,
      compilationErrors:compilation.errors,closed:p.ipcDelivery()},()=>process.disconnect());
  }
}
process.prependListener('message',observer);p.captureMessageObserver();
const initial=enabled?p.captureIpcDelivery():null;
if(kind==='emit-precopy') {
  const emit=process.emit;
  process.emit=function(event,...args){if(event==='message')copy(args[0]);return Reflect.apply(emit,this,[event,...args])};
}
if(kind==='prelistener') {
  function thief(message){copy(message);process.removeListener('message',thief)}
  process.prependListener('message',thief);
}
if(['decoder-copy','decoder-self-restore','decoder-getter','decoder-forged'].includes(kind)) {
  const d=Object.getOwnPropertyDescriptor(v8.Deserializer.prototype,'readValue'),original=d.value;
  const replacement=function(...args){
    if(kind==='decoder-self-restore')Object.defineProperty(v8.Deserializer.prototype,'readValue',d);
    if(kind==='decoder-forged')return {kind:'data',value:7};
    const value=Reflect.apply(original,this,args);
    if(value?.kind==='data')copy(value);
    return value;
  };
  if(kind==='decoder-getter')Object.defineProperty(v8.Deserializer.prototype,'readValue',{configurable:true,get(){
    gets++;Object.defineProperty(v8.Deserializer.prototype,'readValue',d);return replacement;
  }});
  else Object.defineProperty(v8.Deserializer.prototype,'readValue',{...d,value:replacement});
}
if(kind==='header-self-restore') {
  const d=Object.getOwnPropertyDescriptor(v8.Deserializer.prototype,'readHeader'),original=d.value;
  Object.defineProperty(v8.Deserializer.prototype,'readHeader',{...d,value:function(...args){
    Object.defineProperty(v8.Deserializer.prototype,'readHeader',d);return Reflect.apply(original,this,args);
  }});
}
if(kind==='generator-copy') {
  const proto=Object.getPrototypeOf(Object.getPrototypeOf((function*(){})()));
  const d=Object.getOwnPropertyDescriptor(proto,'next'),original=d.value;
  Object.defineProperty(proto,'next',{...d,value:function(...args){
    const value=Reflect.apply(original,this,args);if(value?.value?.kind==='data')copy(value.value);return value;
  }});
}
if(kind==='protocol-getter') {
  Object.defineProperty(Object.prototype,'cmd',{configurable:true,get(){if(this.kind==='data')copy(this);return undefined}});
}
if(kind==='unrelated-decoder')v8.deserialize(v8.serialize({unrelated:true}));
if(kind==='closed')p.close();
if(kind==='synthetic')process.emit('message',{value:9,kind:'data'});
process.send({kind:'ready',initial});`
  const child=spawn(process.execPath,['--input-type=module','-e',source],
    {stdio:['ignore','pipe','pipe','ipc'],serialization})
  const messages=[],errors=[],writes=[]
  child.stderr.on('data',chunk=>errors.push(chunk.toString()))
  const outcome=await new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{child.kill();reject(Error('IPC delivery regression timed out'))},20000)
    child.on('error',error=>{clearTimeout(timer);reject(error)})
    child.on('message',message=>{
      messages.push(message)
      if(message.kind==='ready') {
        const inputs=[1,2].map(value=>kind==='primitive'?value:{kind:'data',value,privatePath:'/private-ipc-delivery',
          ...(kind==='host-object'?{bytes:Buffer.from([value,value+1])}:{})})
        const bytes=ipcFrames(inputs,serialization),request=new (process.binding('stream_wrap').WriteWrap)()
        // Hold the bytes until child exit even if the native write is async.
        writes.push({bytes,request});request.oncomplete=status=>{if(status!==0)reject(Error('fixture IPC write failed: '+status))}
        const status=ipcPipe(child).writeBuffer(request,bytes)
        if(status!==0)reject(Error('fixture IPC write failed: '+status))
      }
    })
    child.on('exit',(code,signal)=>{clearTimeout(timer);resolve({code,signal})})
  })
  assert.equal(outcome.code,0,errors.join('')||String(outcome.signal))
  assert.deepEqual(messages.map(m=>m.kind),['ready','report'])
  return {initial:messages[0].initial,...messages[1]}
}

for(const [kind,serialization] of [['normal','advanced'],['normal','json'],['emit-precopy','advanced'],
  ['prelistener','advanced'],['synthetic','advanced'],['primitive','advanced'],['closed','advanced'],
  ['callback-throw','advanced'],['decoder-copy','advanced'],['decoder-self-restore','advanced'],['decoder-getter','advanced'],
  ['decoder-forged','advanced'],['header-self-restore','advanced'],['generator-copy','advanced'],
  ['protocol-getter','advanced'],['unrelated-decoder','advanced'],['host-object','advanced']]) {
  test('decoded IPC delivery: '+kind+' / '+serialization,async()=>{
    const plain=await run(kind,false,serialization),r=await run(kind,true,serialization)
    assert.deepEqual(r.values,plain.values)
    assert.deepEqual(r.order,plain.order,'IPC batch evidence: '+JSON.stringify({plain:plain.deliveryEvents,observed:r.deliveryEvents}))
    if(['decoder-copy','generator-copy','protocol-getter'].includes(kind))for(const run of [plain,r]) {
      const copies=run.deliveryEvents.filter(e=>e.operation==='copy')
      assert.equal(copies.length,2)
      assert.equal(new Set(copies.map(e=>e.read)).size,1,'both copies must occur in the same native read')
      assert(copies[0].read>0,'copies must occur inside the native read callback')
      assert.deepEqual(run.order,['copy','copy','consumer','consumer'])
      assert(run.deliveryEvents.filter(e=>e.operation==='consumer').every(e=>e.read===0),
        'message consumers must remain deferred until after the native read callback')
    }
    assert.equal(r.gets,plain.gets,'the decoder checks must not invoke a getter')
    assert.equal(r.initial.available,true,JSON.stringify(r.initial))
    assert.deepEqual(r.compilationErrors,[])
    assert.equal(r.report.replayEligible,false);assert.equal(r.report.completeOwnership,false)
    assert.equal(r.closed.closed,true);assert.equal(r.closed.baselineReleased,true)
    assert.equal(r.closed.groupsCreated,r.closed.groupsReleased)
    assert.equal(r.closed.decoder.groupsCreated,r.closed.decoder.groupsReleased)
    assert.equal(r.closed.decoder.pending,false)
    assert.equal(r.report.decoder.initialNativeIntegrity,false)
    assert.equal(r.report.decoder.completeOwnership,false)
    assert(!JSON.stringify(r.report).includes('/private-ipc-delivery'))
    if(['normal','emit-precopy','prelistener','protocol-getter','unrelated-decoder'].includes(kind)) {
      assert.equal(r.report.available,true,JSON.stringify(r.report))
      assert.equal(r.report.decoded,2);assert.equal(r.report.prepared,2);assert.equal(r.report.paired,2)
      assert.equal(r.report.pending,0);assert.equal(r.report.failures,0)
      assert.equal(r.report.groupsCreated,r.report.groupsReleased)
      assert.deepEqual(r.report.parsers.map(p=>p.mode),[serialization])
      assert.equal(r.preparations,2)
      const count=serialization==='advanced'?2:0
      for(const key of ['started','headerEntries','valueEntries','completed'])assert.equal(r.report.decoder[key],count)
      assert.equal(r.report.decoder.hostEntries,0);assert.deepEqual(r.report.decoder.failureFrames,[])
    }else assert.equal(r.report.available,false,JSON.stringify(r.report))
    if(['emit-precopy','prelistener','protocol-getter'].includes(kind)) {
      assert(r.copies.length>0)
      assert(r.copies.every(c=>c.prepared),'pre-dispatch copies must see an already prepared object')
      assert(plain.copies.every(c=>!c.prepared),'the original listener stage must miss these earlier copies')
    }
    if(['decoder-copy','decoder-self-restore','decoder-getter','generator-copy'].includes(kind)) {
      assert(r.copies.length>0)
      assert(r.copies.every(c=>!c.prepared),'the early copy still occurs and the observer must reject its call path')
    }
    if(kind==='decoder-getter')assert.equal(r.gets,1)
    if(kind==='decoder-forged')assert.equal(r.report.decoder.valueEntries,0)
    if(kind==='generator-copy')assert(r.report.errors.includes('IPC parser has an unexpected caller'))
    if(kind==='unrelated-decoder')assert.equal(r.report.decoder.unrelatedNativeEntries,2)
    if(kind==='host-object'){assert.equal(r.report.decoder.hostEntries,2);assert(r.report.errors.includes('IPC host-object decoder callbacks are unverified'))}
    if(kind==='synthetic')assert(r.report.errors.includes('message callback did not match the next decoded IPC message'))
    if(kind==='callback-throw')assert(r.report.errors.includes('IPC preparation callback did not accept the message'))
    if(kind==='primitive')assert(r.report.errors.includes('IPC decoded message is not an ordinary object'))
    if(kind==='closed'){assert.equal(r.report.decoded,0);assert.equal(r.preparations,0)}
  })
}
