import test from 'node:test'
import assert from 'node:assert/strict'
import {spawn,spawnSync} from 'node:child_process'

const module=new URL('./promise-observers-probe.mjs',import.meta.url).href
const warningWrapper=new URL('./fixtures/vitest-suppress-warnings.cjs',import.meta.url).pathname
function run(kind) {
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
import {EventEmitter} from 'node:events';
import {createRequire} from 'node:module';
const kind=${JSON.stringify(kind)};
if(kind.startsWith('vitest-wrapper'))createRequire(import.meta.url)(${JSON.stringify(warningWrapper)});
const p=inspectPromiseObservers({compiledScripts:true});
const delivered=[],copied=[],message={seed:123,tmp:'/private-message'};
function observer(value){delivered.push(value.seed)}
process.prependListener('message',observer);
const emit=process.emit;
if(kind==='wrong-function')process.emit=EventEmitter.prototype.removeAllListeners;
const initial=p.captureMessageObserver();process.emit=emit;
let getterCalls=0,traps=0,mid=null;
function thief(value){copied.push([value.seed,value.tmp]);process.removeListener('message',thief)}
if(kind==='self-removing'||kind==='vitest-wrapper-thief')process.prependListener('message',thief);
if(kind==='once')process.prependOnceListener('message',thief);
if(kind==='same-source') {
  // Same code is insufficient: this is a different closure instance.
  const impostor=eval('('+observer.toString()+')');
  process.prependListener('message',impostor);
}
if(kind==='nested') {
  function nested(value){process.removeListener('message',nested);emit.call(process,'message',{seed:456})}
  process.prependListener('message',nested);
}
if(kind==='unpaired')observer(message);
else if(kind==='other-receiver')new EventEmitter().emit('message',message);
else if(kind==='removed') {process.removeListener('message',observer);emit.call(process,'message',message)}
else if(kind==='overflow') {process.removeListener('message',observer);for(let i=0;i<33;i++)emit.call(process,'message',{seed:i})}
else if(kind==='alternate-dispatch') {process.emit=function(type,value){if(type==='message')observer.call(process,value)};process.emit('message',message);process.emit=emit}
else if(kind==='getter') {
  const events=process._events,original=Object.getOwnPropertyDescriptor(events,'message');
  Object.defineProperty(events,'message',{get(){getterCalls++;return observer},configurable:true});
  emit.call(process,'message',message);
  Object.defineProperty(events,'message',original);
}
else if(kind==='proxy') {
  const events=process._events;
  process._events=new Proxy(events,{get(target,key,receiver){traps++;return Reflect.get(target,key,receiver)}});
  emit.call(process,'message',message);process._events=events;
}
else if(kind==='close') {p.close();emit.call(process,'message',message)}
else {
  emit.call(process,'message',message);
  if(kind==='self-removing'){mid=p.messageDispatch();emit.call(process,'message',{seed:789})}
  if(kind==='primitive')emit.call(process,'message',42);
}
const report=p.messageDispatch(),compilation=p.compiledScriptReport(),listeners=process.rawListeners('message');
p.close();process.removeAllListeners('message');
console.log(JSON.stringify({initial,report,mid,delivered,copied,getterCalls,traps,observerFirst:listeners[0]===observer,
  compilationErrors:compilation.errors,closed:p.messageDispatch()}));`
  const result=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(result.status,0,result.stderr||String(result.error))
  return JSON.parse(result.stdout)
}
for(const kind of ['normal','self-removing','once','same-source','nested','unpaired','other-receiver','removed','overflow','alternate-dispatch','getter','proxy','primitive','close','wrong-function','vitest-wrapper','vitest-wrapper-thief']) {
  test('message dispatch entry: '+kind,()=>{
    const r=run(kind),report=r.report
    assert.equal(r.initial.installed,kind!=='wrong-function',JSON.stringify(r.initial))
    assert.equal(r.initial.available,kind!=='wrong-function',JSON.stringify(r.initial))
    assert.equal(report.replayEligible,false)
    assert.equal(report.completeOwnership,false)
    assert.deepEqual(r.compilationErrors,[])
    assert.equal(r.closed.closed,true)
    assert.equal(r.closed.groupsCreated,r.closed.groupsReleased)
    if(kind.startsWith('vitest-wrapper')) {
      assert.equal(report.wrapperSource.sha256,'a5012da3d2027007781baecda154d343d8c7ad2327b79bec65b1b60a41f42dd1')
      assert.deepEqual(report.source,{sha256:'113b0cd4b1c81c909022c572525ae844136a746dd26e2b7e4a3161841e744281',line:455,column:43})
    }
    if(kind==='normal'||kind==='primitive'||kind==='vitest-wrapper') {
      assert.equal(report.available,true,JSON.stringify(report))
      assert.equal(report.dispatches,kind==='primitive'?2:1)
      assert.equal(report.paired,report.dispatches)
      assert.equal(r.observerFirst,true,'diagnostic did not replace the listener')
    } else if(kind==='other-receiver') {
      assert.equal(report.available,true,JSON.stringify(report));assert.equal(report.dispatches,0)
      assert.equal(report.ignoredReceivers,1);assert.deepEqual(r.delivered,[])
    } else if(kind==='close') {
      assert.equal(report.closed,true);assert.equal(report.dispatches,0);assert.deepEqual(r.delivered,[123])
    } else {
      assert.equal(report.available,false,JSON.stringify(report))
      if(kind==='wrong-function') {
        assert.equal(report.source.sha256,'113b0cd4b1c81c909022c572525ae844136a746dd26e2b7e4a3161841e744281')
        assert(report.errors.includes('process emit function differs from pinned Node'))
      }
      if(['self-removing','once','same-source','nested','vitest-wrapper-thief'].includes(kind))assert(report.errors.includes('observer was not first at message dispatch'),JSON.stringify(report))
      if(kind==='self-removing') {
        assert.deepEqual(r.copied,[[123,'/private-message']]);assert.equal(r.observerFirst,true)
        assert.equal(r.mid.available,false);assert.equal(report.dispatches,2);assert.equal(report.paired,2)
      }
      if(kind==='nested'){assert.equal(report.paired,2);assert.deepEqual(r.delivered,[456,123])}
      if(kind==='removed'){assert.equal(report.pending,1);assert.equal(report.callbacks,0)}
      if(kind==='overflow') {
        assert.equal(report.pending,32)
        assert.equal(report.failureCounts['pending message capacity exceeded'],1)
        assert.equal(report.errors.length,20,'error samples remain bounded while counters retain every failure')
      }
      if(kind==='unpaired'||kind==='alternate-dispatch')assert(report.errors.includes('observer callback has no matching message dispatch'),JSON.stringify(report))
      if(kind==='getter')assert.equal(r.getterCalls,1,'only original emit executes the getter')
      if(kind==='proxy')assert.equal(r.traps,1,'only original emit reads the proxy')
    }
    assert(!JSON.stringify(report).includes('/private-message'),'reports never contain message values')
  })
}

test('actual child-process IPC reaches the observer with the same input and released handles',async()=>{
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
const p=inspectPromiseObservers();
function observer(message){
  const report=p.messageDispatch();p.close();process.removeListener('message',observer);
  process.send({kind:'report',report,received:message},()=>process.disconnect());
}
process.prependListener('message',observer);
process.send({kind:'ready',initial:p.captureMessageObserver()});`
  const child=spawn(process.execPath,['--input-type=module','-e',source],{stdio:['ignore','pipe','pipe','ipc']})
  const errors=[],reports=[]
  child.stderr.on('data',chunk=>errors.push(chunk.toString()))
  const input={seed:987,tmp:'/private-ipc',nested:{unchanged:true}}
  const outcome=await new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{child.kill();reject(Error('IPC regression timed out'))},20000)
    child.on('error',error=>{clearTimeout(timer);reject(error)})
    child.on('message',message=>{
      reports.push(message)
      if(message.kind==='ready')child.send(input)
    })
    child.on('exit',(code,signal)=>{clearTimeout(timer);resolve({code,signal})})
  })
  assert.equal(outcome.code,0,errors.join('')||String(outcome.signal))
  assert.deepEqual(reports.map(r=>r.kind),['ready','report'])
  assert.equal(reports[0].initial.available,true,JSON.stringify(reports[0]))
  const {report,received}=reports[1]
  assert.deepEqual(received,input)
  assert.equal(report.available,true,JSON.stringify(report))
  assert.equal(report.dispatches,1);assert.equal(report.callbacks,1);assert.equal(report.paired,1)
  assert.equal(report.groupsCreated,report.groupsReleased)
  assert.equal(report.completeOwnership,false);assert.equal(report.replayEligible,false)
})
