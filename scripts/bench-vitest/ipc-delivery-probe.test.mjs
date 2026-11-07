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
  const boundWire=ipcFrames([1,2].map(value=>({kind:'data',value,privatePath:'/private-ipc-delivery'})),serialization).toString('base64')
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(registryUrl)};
import v8 from 'node:v8';
import {executionAsyncId,executionAsyncResource,createHook} from 'node:async_hooks';
const enabled=${enabled},kind=${JSON.stringify(kind)};
const fragmented=kind==='normal-fragmented';
const p=inspectPromiseObservers({compiledScripts:true}),preparedMessages=new WeakSet();
const values=[],order=[],copies=[],deliveryEvents=[],rawCopies=[],wireCopies=[];
const byteCopyPreparations=[];
let preparations=0,gets=0,asyncHook=null;
const pipe=process[Object.getOwnPropertySymbols(process).find(s=>s.description==='kChannelHandle')];
const onread=pipe.onread;
function activeRead(){return executionAsyncResource()===pipe?executionAsyncId():0}
function prepareWorkerMessage(message){
  if(kind==='callback-throw')throw Error('fixture preparation failure');
  if(message&&typeof message==='object')preparedMessages.add(message);
  preparations++;return true;
}
function copy(message){copies.push({prepared:message&&typeof message==='object'&&preparedMessages.has(message),value:message.value});order.push('copy');deliveryEvents.push({operation:'copy',value:message.value,read:activeRead()})}
function observer(message){
  if(!enabled&&kind!=='callback-throw')prepareWorkerMessage(message);
  values.push(typeof message==='object'?message.value:message);order.push('consumer');
  deliveryEvents.push({operation:'consumer',value:values.at(-1),read:activeRead()});
  if(values.length===(kind==='synthetic'?3:2)) {
    const report=p.ipcDelivery(),dispatch=p.messageDispatch(),compilation=p.compiledScriptReport();
    asyncHook?.disable();p.close();process.removeListener('message',observer);
    if(kind==='onread-shadow-getter')delete pipe.onread;
    if(kind.startsWith('buffer-setter')){delete v8.Deserializer.prototype.buffer;delete Object.prototype.buffer;}
    if(kind.startsWith('species-'))delete Uint8Array[Symbol.species];
    const rawCopiedValues=rawCopies.map(bytes=>{const d=new v8.Deserializer(bytes);d.readHeader();return d.readValue().value});
    process.send({kind:'report',report,dispatch,preparations,gets,values,order,copies,deliveryEvents,
      rawCopiedValues,wireCopyBytes:wireCopies.reduce((n,b)=>n+b.byteLength,0),
      byteCopyPreparations,
      compilationErrors:compilation.errors,closed:p.ipcDelivery()},()=>process.disconnect());
  }
}
process.prependListener('message',observer);p.captureMessageObserver();
const initial=enabled?p.captureIpcDelivery():null;
if(['onread-copy','onread-self-restore','async-resource-copy'].includes(kind)) {
  const replacement=function(...args){
    if(kind!=='onread-copy')pipe.onread=onread;
    if(args[0])wireCopies.push(new Uint8Array(args[0]).slice());
    return Reflect.apply(onread,this,args);
  };
  pipe.onread=replacement;
}
if(kind==='onread-shadow-getter')Object.defineProperty(pipe,'onread',{configurable:true,get(){gets++;throw Error('public onread getter must stay unused')}});
if(kind==='onread-bound')pipe.onread=onread.bind(pipe);
if(kind==='onread-bound-extra') {
  const bytes=Buffer.from(${JSON.stringify(boundWire)},'base64'),copy=new Uint8Array(bytes.length);copy.set(bytes);
  pipe.onread=onread.bind(pipe,copy.buffer);
}
if(kind.startsWith('async-resource')){executionAsyncResource();asyncHook=createHook({before(){}}).enable();}
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
if(['constructor-copy','constructor-self-restore','constructor-proxy','constructor-late-swap'].includes(kind)) {
  const original=Object.getPrototypeOf(v8.DefaultDeserializer);
  function copyBytes(bytes){const d=new original(bytes);d.readHeader();copy(d.readValue())}
  const replacement=function(...args){
    if(kind!=='constructor-copy')Object.setPrototypeOf(v8.DefaultDeserializer,original);
    copyBytes(args[0]);return Reflect.construct(original,args,new.target);
  };
  if(kind==='constructor-late-swap')Object.defineProperty(Uint8Array,Symbol.species,{configurable:true,get(){
    gets++;delete Uint8Array[Symbol.species];Object.setPrototypeOf(v8.DefaultDeserializer,replacement);return Uint8Array;
  }});
  else Object.setPrototypeOf(v8.DefaultDeserializer,kind==='constructor-proxy'?new Proxy(original,{construct(target,args,newTarget){
    Object.setPrototypeOf(v8.DefaultDeserializer,original);copyBytes(args[0]);return Reflect.construct(target,args,newTarget);
  }}):replacement);
}
if(kind.startsWith('buffer-setter')) {
  const owner=kind==='buffer-setter-root'?Object.prototype:v8.Deserializer.prototype;
  function install(){Object.defineProperty(owner,'buffer',{configurable:true,set(bytes){
    if(kind!=='buffer-setter')delete owner.buffer;
    rawCopies.push(Buffer.from(bytes));
    Object.defineProperty(this,'buffer',{configurable:true,writable:true,enumerable:true,value:bytes});
  }})}
  if(kind==='buffer-setter-late-install')Object.defineProperty(Uint8Array,Symbol.species,{configurable:true,get(){
    gets++;delete Uint8Array[Symbol.species];install();return Uint8Array;
  }});
  else install();
}
if(kind.startsWith('species-')) {
  const original=Uint8Array;
  function replacement(buffer,offset,length){
    if(kind!=='species-copy')delete original[Symbol.species];
    const view=new original(buffer,offset,length);
    wireCopies.push(Buffer.from(view));byteCopyPreparations.push(preparations);
    return view;
  }
  if(kind==='species-getter-self-restore')Object.defineProperty(original,Symbol.species,{configurable:true,get(){
    gets++;delete original[Symbol.species];return replacement;
  }});
  else Object.defineProperty(original,Symbol.species,{configurable:true,value:replacement});
}
if(kind==='view-constructor-getter') {
  const descriptor=Object.getOwnPropertyDescriptor(Uint8Array.prototype,'constructor');
  Object.defineProperty(Uint8Array.prototype,'constructor',{configurable:true,get(){
    gets++;Object.defineProperty(Uint8Array.prototype,'constructor',descriptor);
    wireCopies.push(Buffer.from(this));byteCopyPreparations.push(preparations);
    return descriptor.value;
  }});
}
if(kind==='unrelated-decoder')v8.deserialize(v8.serialize({unrelated:true}));
if(kind==='closed')p.close();
if(kind==='synthetic')process.emit('message',{value:9,kind:'data'});
if(fragmented){const timer=setInterval(()=>{if(pipe.buffering){clearInterval(timer);process.send({kind:'fragment-ready'})}},1)}
process.send({kind:'ready',initial});`
  const child=spawn(process.execPath,['--input-type=module','-e',source],
    {stdio:['ignore','pipe','pipe','ipc'],serialization})
  const messages=[],errors=[],writes=[]
  let remainingBytes=null
  function write(bytes,reject){
    const request=new (process.binding('stream_wrap').WriteWrap)()
    writes.push({bytes,request});request.oncomplete=status=>{if(status!==0)reject(Error('fixture IPC write failed: '+status))}
    const status=ipcPipe(child).writeBuffer(request,bytes)
    if(status!==0)reject(Error('fixture IPC write failed: '+status))
  }
  child.stderr.on('data',chunk=>errors.push(chunk.toString()))
  const outcome=await new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{child.kill();reject(Error('IPC delivery regression timed out'))},20000)
    child.on('error',error=>{clearTimeout(timer);reject(error)})
    child.on('message',message=>{
      messages.push(message)
      if(message.kind==='ready') {
        const inputs=[1,2].map(value=>kind==='primitive'?value:{kind:'data',value,privatePath:'/private-ipc-delivery',
          ...(kind==='host-object'?{bytes:Buffer.from([value,value+1])}:{})})
        const bytes=ipcFrames(inputs,serialization)
        // Hold the bytes until child exit even if the native write is async.
        if(kind==='normal-fragmented'){remainingBytes=bytes.subarray(3);write(bytes.subarray(0,3),reject)}
        else write(bytes,reject)
      }
      if(message.kind==='fragment-ready'){assert(remainingBytes);write(remainingBytes,reject);remainingBytes=null}
    })
    child.on('exit',(code,signal)=>{clearTimeout(timer);resolve({code,signal})})
  })
  assert.equal(outcome.code,0,errors.join('')||String(outcome.signal))
  assert.deepEqual(messages.map(m=>m.kind),kind==='normal-fragmented'?['ready','fragment-ready','report']:['ready','report'])
  return {initial:messages[0].initial,...messages.at(-1)}
}

for(const [kind,serialization] of [['normal','advanced'],['normal','json'],['normal-fragmented','advanced'],['emit-precopy','advanced'],
  ['prelistener','advanced'],['synthetic','advanced'],['primitive','advanced'],['closed','advanced'],
  ['callback-throw','advanced'],['decoder-copy','advanced'],['decoder-self-restore','advanced'],['decoder-getter','advanced'],
  ['decoder-forged','advanced'],['header-self-restore','advanced'],['generator-copy','advanced'],
  ['protocol-getter','advanced'],['unrelated-decoder','advanced'],['host-object','advanced'],
  ['constructor-copy','advanced'],['constructor-self-restore','advanced'],['constructor-proxy','advanced'],
  ['constructor-late-swap','advanced'],['buffer-setter','advanced'],['buffer-setter-self-restore','advanced'],
  ['buffer-setter-late-install','advanced'],['buffer-setter-root','advanced'],
  ['onread-copy','advanced'],['onread-self-restore','advanced'],['onread-shadow-getter','advanced'],['onread-bound','advanced'],
  ['onread-bound-extra','advanced'],['async-resource','advanced'],['async-resource-copy','advanced'],
  ['species-copy','advanced'],['species-self-restore','advanced'],['species-getter-self-restore','advanced'],
  ['view-constructor-getter','advanced']]) {
  test('decoded IPC delivery: '+kind+' / '+serialization,async()=>{
    const plain=await run(kind,false,serialization),r=await run(kind,true,serialization)
    assert.deepEqual(r.values,plain.values)
    assert.deepEqual(r.order,plain.order,'IPC batch evidence: '+JSON.stringify({plain:plain.deliveryEvents,observed:r.deliveryEvents}))
    if(['decoder-copy','generator-copy','protocol-getter'].includes(kind))for(const run of [plain,r]) {
      const copies=run.deliveryEvents.filter(e=>e.operation==='copy')
      assert.equal(copies.length,2)
      assert.equal(new Set(copies.map(e=>e.read)).size,1,'both copies must execute in the IPC pipe resource')
      assert(copies[0].read>0,'copies must occur inside the native read callback')
      assert.deepEqual(run.order,['copy','copy','consumer','consumer'])
      assert(run.deliveryEvents.filter(e=>e.operation==='consumer').every(e=>e.read===0),
        'message consumers must remain deferred until after the native read callback')
    }
    if(['decoder-copy','generator-copy','protocol-getter'].includes(kind)) {
      assert.equal(r.report.reads.entries,1,'the observed packet batch must use one native read callback')
      assert.equal(r.report.reads.completed,1)
    }
    assert.equal(r.gets,plain.gets,'the decoder checks must not invoke a getter')
    assert.equal(r.initial.available,true,JSON.stringify(r.initial))
    assert.deepEqual(r.compilationErrors,[])
    assert.equal(r.report.replayEligible,false);assert.equal(r.report.completeOwnership,false)
    assert.equal(r.closed.closed,true);assert.equal(r.closed.baselineReleased,true)
    assert.equal(r.closed.groupsCreated,r.closed.groupsReleased)
    assert.equal(r.closed.decoder.groupsCreated,r.closed.decoder.groupsReleased)
    assert.equal(r.closed.decoder.pending,false)
    assert.equal(r.closed.reads.pending,false)
    assert.equal(r.closed.reads.groupsCreated,r.closed.reads.groupsReleased)
    assert.equal(r.report.reads.completeOwnership,false)
    assert.equal(r.report.decoder.initialNativeIntegrity,false)
    assert.equal(r.report.decoder.completeOwnership,false)
    if(kind.startsWith('species-')||kind==='view-constructor-getter') {
      assert(r.wireCopyBytes>0,'the fixture must copy IPC bytes before decoding')
      assert.equal(r.wireCopyBytes,plain.wireCopyBytes,'observation must preserve the byte-copy behavior')
      assert.equal(r.byteCopyPreparations.length,plain.byteCopyPreparations.length)
      assert.equal(r.byteCopyPreparations[0],0,'the first byte copy must precede preparation')
      assert(r.report.errors.some(e=>e.startsWith('IPC slice ')))
    }
    assert(!JSON.stringify(r.report).includes('/private-ipc-delivery'))
    if(['normal','normal-fragmented','emit-precopy','prelistener','protocol-getter','unrelated-decoder','onread-shadow-getter','onread-bound','async-resource'].includes(kind)) {
      assert.equal(r.report.available,true,JSON.stringify(r.report))
      assert.equal(r.report.decoded,2);assert.equal(r.report.prepared,2);assert.equal(r.report.paired,2)
      assert.equal(r.report.pending,0);assert.equal(r.report.failures,0)
      assert.equal(r.report.groupsCreated,r.report.groupsReleased)
      assert.deepEqual(r.report.parsers.map(p=>p.mode),[serialization])
      assert.equal(r.preparations,2)
      const count=serialization==='advanced'?2:0
      for(const key of ['started','constructorEntries','headerEntries','valueEntries','completed'])assert.equal(r.report.decoder[key],count)
      assert.equal(r.report.decoder.hostEntries,0);assert.deepEqual(r.report.decoder.failureFrames,[])
      assert.equal(r.report.reads.entries,kind==='normal-fragmented'?2:1);assert.equal(r.report.reads.completed,r.report.reads.entries)
      assert.equal(r.report.reads.delivered,2);assert.deepEqual(r.report.reads.failureFrames,[])
      assert.equal(r.report.reads.groupsCreated,r.report.reads.groupsReleased)
      assert.equal(r.report.reads.entries,r.report.reads.directEntries+r.report.reads.trampolineEntries)
      assert.equal(r.report.slices.entries,count*2);assert.equal(r.report.slices.checked,count*2)
      assert.equal(r.report.slices.failures,0)
      if(kind==='normal-fragmented')assert.equal(r.report.slices.buffer,4)
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
    if(kind==='unrelated-decoder')assert.equal(r.report.decoder.unrelatedConstructorEntries,1)
    if(['onread-copy','onread-self-restore','async-resource-copy'].includes(kind)) {
      assert(r.wireCopyBytes>0);assert.equal(r.wireCopyBytes,plain.wireCopyBytes)
      assert(r.report.errors.includes('IPC read callback has an unexpected caller'))
    }
    if(kind==='onread-shadow-getter'){assert.equal(r.gets,0);assert.equal(r.wireCopyBytes,0)}
    if(kind==='onread-bound-extra')assert(r.report.errors.includes('IPC read callback has unexpected arguments'))
    if(kind==='async-resource')assert.equal(r.report.reads.trampolineEntries,1)
    if(kind.startsWith('constructor-')) {
      assert(r.copies.length>0)
      assert(r.copies.every(c=>!c.prepared),'constructor copies occur before preparation and must reject the record')
      assert(r.report.errors.some(e=>e.startsWith('IPC decoder constructor chain changed')||e.startsWith('IPC native constructor has an unexpected caller')))
      if(kind==='constructor-late-swap') {
        assert.equal(r.gets,1)
        assert(!r.report.errors.includes('IPC decoder constructor chain changed'),
          'the late swap must occur after the pre-construction chain check')
        assert(r.report.errors.includes('IPC native constructor has an unexpected caller'),
          'the actual native entry must reject the self-restored late replacement')
      }
    }
    if(kind.startsWith('buffer-setter')) {
      const expected=kind==='buffer-setter'?[1,2]:[1]
      assert.deepEqual(r.rawCopiedValues,expected)
      assert.deepEqual(r.rawCopiedValues,plain.rawCopiedValues,'the observer must not cause extra buffer copies')
      assert(r.report.errors.includes('IPC decoder buffer initialization is observable'))
      if(kind==='buffer-setter-late-install') {
        assert.equal(r.gets,1)
        const failures=r.report.decoder.failureFrames.filter(f=>f.reason==='IPC decoder buffer initialization is observable')
        assert.deepEqual(failures.map(f=>f.kinds),[['native:construct']],
          'late buffer setter must be detected at native entry, after the initial check')
      }
    }
    if(kind==='host-object'){assert.equal(r.report.decoder.hostEntries,2);assert(r.report.errors.includes('IPC host-object decoder callbacks are unverified'))}
    if(kind==='synthetic')assert(r.report.errors.includes('message callback did not match the next decoded IPC message'))
    if(kind==='callback-throw')assert(r.report.errors.includes('IPC preparation callback did not accept the message'))
    if(kind==='primitive')assert(r.report.errors.includes('IPC decoded message is not an ordinary object'))
    if(kind==='closed'){assert.equal(r.report.decoded,0);assert.equal(r.preparations,0)}
  })
}
