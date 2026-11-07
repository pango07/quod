import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {fileURLToPath} from 'node:url'
import {spawn} from 'node:child_process'
import options from './worker-startup-options.cjs'

const warning=fileURLToPath(new URL('./fixtures/vitest-suppress-warnings.cjs',import.meta.url))
const bootstrap=fileURLToPath(new URL('./worker-module-metadata-bootstrap.mjs',import.meta.url))
test('startup launch contract preserves accepted arguments and rejects earlier execution modes',()=>{
  const args=['--conditions','development','--require',warning,'--import',bootstrap]
  const original=[...args],env={NODE_OPTIONS:'',SAFE:'retained'}
  assert.deepEqual(options.workerStartupArguments(args,env),['--require',options.preload,...args])
  assert.deepEqual(args,original);assert.deepEqual(env,{NODE_OPTIONS:'',SAFE:'retained'})
  for(const key of ['NODE_OPTIONS','LD_PRELOAD','LD_AUDIT','DYLD_INSERT_LIBRARIES'])
    assert.throws(()=>options.workerStartupArguments(args,{[key]:'untrusted'}),new RegExp(key))
  for(const args of [['--snapshot-blob=/tmp/fixture'],['--experimental-loader','/tmp/fixture'],
    ['--experimental-config-file=/tmp/fixture'],['--inspect'],['--import','/tmp/fixture'],
    ['--require',fileURLToPath(import.meta.url)],['--require',warning,'--require',warning],['--conditions'],
    ['--require',options.preload]])assert.throws(()=>options.workerStartupArguments(args,{}))
  assert.throws(()=>options.validateWorkerLaunch(['--require',warning,'--require',options.preload],{},{installed:true}))
})

async function run(mode,enabled) {
  const directory=fs.mkdtempSync(path.join(os.tmpdir(),'quod-startup-'))
  const main=path.join(directory,'main.mjs')
  fs.writeFileSync(main,`import {createRequire} from 'node:module';
import v8 from 'node:v8';
const require=createRequire(import.meta.url),mode=${JSON.stringify(mode)},enabled=${enabled};
const startup=enabled?require(${JSON.stringify(options.preload)}):null;
const before=startup?.report(),registry=startup?.registry;
const values=[],order=[],copies=[],wireCopies=[];let preparations=0,handoffResult=null;
const prepared=new WeakSet(),transport={received(){},delivered(){return true}};
function prepare(message){prepared.add(message);preparations++;return true}
if(mode==='decoder-self-restore'){
 const original=v8.Deserializer.prototype.readValue;
 v8.Deserializer.prototype.readValue=function(...args){
  v8.Deserializer.prototype.readValue=original;
  const result=Reflect.apply(original,this,args);copies.push({value:result.value,prepared:prepared.has(result)});order.push('copy');return result;
 };
}
if(mode==='onread-self-restore'){
 const pipe=process[Object.getOwnPropertySymbols(process).find(s=>s.description==='kChannelHandle')],original=pipe.onread;
 pipe.onread=function(...args){pipe.onread=original;wireCopies.push(new Uint8Array(args[0]).slice());return Reflect.apply(original,this,args)};
}
if(mode==='unknown-wrapper'){
 const emit=process.emit;process.emit=function(...args){return Reflect.apply(emit,this,args)};
}
function handoff(){if(enabled)handoffResult=startup.handoff({transport,prepare})}
if(mode!=='early')handoff();
if(mode==='repeated')handoff();
process.on('message',message=>{
 if(!enabled&&!(mode==='early'&&values.length===0))prepare(message);
 values.push(message.value);order.push('consumer');
 if(values.length===1){if(mode==='early')handoff();process.send({kind:'next'});return}
 const report=startup?.report(),ipc=registry?.ipcDelivery(),dispatch=registry?.messageDispatch();
 startup?.close();
 process.send({kind:'report',before,report,ipc,dispatch,closed:registry?.ipcDelivery(),handoffResult,
  values,order,copies,wireCopyBytes:wireCopies.reduce((n,b)=>n+b.byteLength,0),preparations},()=>process.disconnect());
});
process.send({kind:'ready'});
`)
  const args=[...(enabled?['--require',options.preload]:[]),...(mode==='warning'?['--require',warning]:[]),main]
  const child=spawn(process.execPath,args,{serialization:'advanced',stdio:['ignore','pipe','pipe','ipc'],
    env:{...process.env,NODE_OPTIONS:'',QUOD_WORKER_BOUNDARY_PROBE:'0'}})
  const messages=[],errors=[]
  child.stderr.on('data',b=>errors.push(b.toString()))
  try {
    const outcome=await new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>{child.kill();reject(Error('startup regression timed out'))},25000)
      child.on('error',e=>{clearTimeout(timer);reject(e)})
      child.on('message',message=>{
        messages.push(message)
        if(message.kind==='ready')child.send({value:1})
        if(message.kind==='next')child.send({value:2})
      })
      child.on('exit',(code,signal)=>{clearTimeout(timer);resolve({code,signal})})
    })
    assert.equal(outcome.code,0,errors.join('')||String(outcome.signal))
    assert.deepEqual(messages.map(m=>m.kind),['ready','next','report'])
    return messages.at(-1)
  }finally{fs.rmSync(directory,{recursive:true,force:true})}
}

for(const mode of ['normal','warning','early','decoder-self-restore','onread-self-restore','unknown-wrapper','repeated']) {
  test('first preload and persistent handoff: '+mode,async()=>{
    const plain=await run(mode,false),r=await run(mode,true)
    assert.deepEqual(r.values,plain.values);assert.deepEqual(r.order,plain.order)
    assert.deepEqual(r.copies,plain.copies);assert.equal(r.wireCopyBytes,plain.wireCopyBytes)
    assert.equal(r.before.firstRequire,true);assert.equal(r.before.preloadParent,true)
    assert.equal(r.before.priorCachedModules,0);assert.equal(r.before.handoffs,0)
    assert.equal(r.before.installedBeforeApplicationImports,true)
    assert.equal(r.report.handoffs,1);assert.equal(r.report.closed,false)
    assert.equal(r.report.completeStartupIntegrity,false);assert.equal(r.report.replayEligible,false)
    assert.equal(r.report.available,['normal','warning'].includes(mode),JSON.stringify(r))
    assert.equal(r.closed.closed,true);assert.equal(r.closed.baselineReleased,true)
    for(const part of [r.closed,r.closed.decoder,r.closed.reads])assert.equal(part.groupsCreated,part.groupsReleased)
    if(mode==='normal'||mode==='warning') {
      assert.equal(r.handoffResult,true);assert.equal(r.ipc.decoded,2);assert.equal(r.ipc.paired,2)
      assert.equal(r.preparations,2);assert.equal(r.report.prematureMessages,0)
      assert.deepEqual(r.report.failures,[]);assert.deepEqual(r.ipc.errors,[]);assert.deepEqual(r.dispatch.errors,[])
      assert.equal(r.dispatch.invocation.wrapperChecks,mode==='warning'?2:0)
    }
    if(mode==='early'){
      assert.equal(r.report.prematureMessages,1);assert.equal(r.preparations,1);assert.equal(r.handoffResult,false)
      assert(r.ipc.errors.includes('IPC preparation callback did not accept the message'))
      assert(r.dispatch.errors.includes('warning wrapper handoff is not at the initial boundary'))
    }
    if(mode==='decoder-self-restore'){
      assert.deepEqual(r.copies,[{value:1,prepared:false}]);assert(r.ipc.errors.some(e=>e.startsWith('IPC decoder method changed')))
    }
    if(mode==='onread-self-restore'){
      assert(r.wireCopyBytes>0);assert(r.ipc.errors.includes('IPC read callback has an unexpected caller'))
    }
    if(mode==='unknown-wrapper')assert(r.dispatch.errors.includes('late warning wrapper differs from pinned Node forwarding'))
    if(mode==='repeated')assert(r.report.failures.includes('invalid or repeated startup handoff'))
  })
}
