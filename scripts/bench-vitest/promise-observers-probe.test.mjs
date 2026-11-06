import {test} from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

// Each isolated process avoids the test runner's own hooks. The probe may
// inspect runtime state but must never grant reuse or change hook delivery.
const probe=new URL('./promise-observers-probe.mjs',import.meta.url).href
function run(kind) {
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(probe)};
import {promiseHooks} from 'node:v8';
import {createHook} from 'node:async_hooks';
import fs from 'node:fs';
import {inspect} from 'node:util';
const p=inspectPromiseObservers(), snapshots=[p.snapshot()];
let callbacks=0, captured='', native=null, stop=()=>{};
if(${JSON.stringify(kind)}==='promise') stop=promiseHooks.createHook({init(){callbacks++},before(){},after(){},settled(){}});
if(${JSON.stringify(kind)}==='async') {const hook=createHook({init(){callbacks++},before(){},after(){},destroy(){},promiseResolve(){}}).enable();stop=()=>hook.disable()}
if(${JSON.stringify(kind)}==='inspect') stop=promiseHooks.onSettled(promise=>{callbacks++;setImmediate(()=>{const value=inspect(promise);if(value.includes('Stats')) captured=value})});
snapshots.push(p.snapshot());
native=fs.promises.stat(${JSON.stringify(new URL('./promise-observers-probe.mjs',import.meta.url).pathname)});
const stats=await native;
await new Promise(resolve=>setImmediate(resolve));
snapshots.push(p.snapshot());stop();snapshots.push(p.snapshot());p.close();
console.log(JSON.stringify({snapshots,callbacks,sawMetadata:captured.includes('mtimeMs'),size:stats.size,nativePromise:native instanceof Promise}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:15000})
  assert.equal(r.status,0,r.stderr||String(r.error))
  return JSON.parse(r.stdout)
}
for(const kind of ['none','promise','async','inspect']) test('passive observer registry: '+kind,()=>{
  const r=run(kind)
  assert.equal(r.nativePromise,true)
  assert.ok(r.size>0)
  for(const s of r.snapshots) {assert.equal(s.available,true,JSON.stringify(s));assert.equal(s.replayEligible,false)}
  const [before,during,after,disabled]=r.snapshots
  assert.deepEqual(before.promiseHooks,{init:0,before:0,after:0,settled:0})
  assert.equal(before.asyncHooks.kTotals,0)
  if(kind==='promise') {assert.deepEqual(during.promiseHooks,{init:1,before:1,after:1,settled:1});assert.ok(r.callbacks>0)}
  if(kind==='async') {assert.ok(during.asyncHooks.kTotals>0);assert.ok(after.asyncHooks.kInit>0);assert.ok(r.callbacks>0)}
  if(kind==='inspect') {assert.equal(during.promiseHooks.settled,1);assert.equal(r.sawMetadata,true,'a promise hook can inspect Stats without a .then reaction')}
  assert.deepEqual(disabled.promiseHooks,before.promiseHooks)
  assert.equal(disabled.asyncHooks.kTotals,0)
})
