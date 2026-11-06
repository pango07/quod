import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const module=new URL('./promise-observers-probe.mjs',import.meta.url).href
for(const kind of ['none','promise','before','after','settled','cached-promise','async','resource','binding']) {
  test('hook and async resource lifetime observation: '+kind,()=>{
    const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
import {promiseHooks} from 'node:v8';
import {createHook,executionAsyncResource} from 'node:async_hooks';
const kind=${JSON.stringify(kind)},cachedOnInit=promiseHooks.onInit,cachedBinding=process.binding;
const hookName={promise:'onInit',before:'onBefore',after:'onAfter',settled:'onSettled','cached-promise':'onInit'}[kind];
const cachedHook=hookName&&promiseHooks[hookName];
const captured=[],metadata={tmp:'private-metadata-value'};
const p=inspectPromiseObservers({hookAccess:true});
let rawRead=false,resourcePresent=false,bindingPresent=false;
if(hookName) {
  if(kind==='cached-promise')promiseHooks.onInit=()=>()=>{};
  const stop=cachedHook(promise=>captured.push(promise));
  if(kind==='promise'||kind==='cached-promise') {
    Promise.resolve(metadata);stop();
    const values=await Promise.all(captured.map(p=>p.then(v=>v,()=>null)));
    rawRead=values.includes(metadata);
  }else {await Promise.resolve(metadata);stop()}
  promiseHooks.onInit=cachedOnInit;
}
if(kind==='async') {
  const hook=createHook({init(id,type,trigger,resource){if(type==='PROMISE')captured.push(resource)}});
  hook.enable();Promise.resolve(metadata);hook.disable();
  const values=await Promise.all(captured.map(p=>p.then(v=>v,()=>null)));
  rawRead=values.includes(metadata);
}
if(kind==='resource')resourcePresent=typeof executionAsyncResource()==='object';
if(kind==='binding')bindingPresent=typeof cachedBinding('inspector')==='object';
await Promise.resolve(1);
const report=p.hookAccess();p.close();
console.log(JSON.stringify({report,rawRead,resourcePresent,bindingPresent}));`
    const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:15000})
    assert.equal(r.status,0,r.stderr||String(r.error))
    const {report,rawRead,resourcePresent,bindingPresent}=JSON.parse(r.stdout)
    assert.equal(report.available,true,JSON.stringify(report))
    assert.equal(report.overflow,false)
    assert.equal(report.replayEligible,false)
    assert(!JSON.stringify(report).includes('private-metadata-value'))
    assert(Object.values(report.initial.promiseHooks).every(n=>n===0))
    assert(Object.values(report.current.promiseHooks).every(n=>n===0))
    assert.equal(report.current.asyncHooks.kTotals,0)
    const names=report.entries.flatMap(e=>e.entries)
    if(kind==='none')assert.deepEqual(names,[])
    if(['promise','before','after','settled','cached-promise'].includes(kind)) {
      if(kind==='promise'||kind==='cached-promise')assert.equal(rawRead,true,'temporary hook must actually retain the raw result')
      assert(names.includes('promiseHooks.register'))
      assert(names.includes('async_wrap.setPromiseHooks'))
    }
    if(kind==='async') {
      assert.equal(rawRead,true,'temporary async hook must actually retain the raw result')
      assert(names.includes('AsyncHook.enable'));assert(names.includes('AsyncHook.disable'))
    }
    if(kind==='resource') {
      assert.equal(resourcePresent,true)
      assert(names.includes('async_hooks.executionAsyncResource'))
    }
    if(kind==='binding') {
      assert.equal(bindingPresent,true)
      assert(names.includes('process.binding'),'cached private binding entry must remain observable')
    }
  })
}
