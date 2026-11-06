import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const module=new URL('./promise-observers-probe.mjs',import.meta.url).href
function fixture(body) {
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
let gets=0;
const target={get tmp(){gets++;return 'private-value'}},key='tmp',change={value:42,configurable:true},changes={tmp:change};
${body}`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error))
  const data=JSON.parse(r.stdout)
  assert.equal(data.report.available,true,JSON.stringify(data.report))
  assert.equal(data.report.overflow,false);assert.equal(data.report.replayEligible,false)
  assert.equal(data.report.completeDescriptorCoverage,false)
  assert(!JSON.stringify(data.report).includes('private-value'))
  return data
}

for(const [api,args] of [
  ['Object.getOwnPropertyDescriptor','target,key'],['Reflect.getOwnPropertyDescriptor','target,key'],
  ['Object.getOwnPropertyDescriptors','target'],['Object.defineProperty','target,key,change'],
  ['Reflect.defineProperty','target,key,change'],['Object.defineProperties','target,changes'],
  ['Reflect.deleteProperty','target,key'],['Object.freeze','target'],['Object.seal','target'],
  ['Object.preventExtensions','target'],['Reflect.preventExtensions','target'],
  ['Object.isFrozen','target'],['Object.isSealed','target'],
  ['Object.setPrototypeOf','target,null'],['Reflect.setPrototypeOf','target,null'],
])test('cached native descriptor entry and target: '+api,()=>{
  const data=fixture(`const native=${api};const p=inspectPromiseObservers({descriptorAccess:true});
native(${args});const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets}));`)
  assert.equal(data.gets,0,'inspecting the target must not call its accessor')
  const events=data.report.entries.filter(e=>e.entries.includes(api))
  assert.equal(events.reduce((n,e)=>n+e.count,0),1,JSON.stringify(data.report))
  assert(events.every(e=>e.argument.available&&e.argument.kind==='object'&&e.argument.target.watchedAccessorKeys.includes('tmp')),JSON.stringify(events))
})

test('legacy getter lookup is observed through a cached intrinsic and retains an unknown receiver',()=>{
  const data=fixture(`const lookup=Object.prototype.__lookupGetter__;const p=inspectPromiseObservers({descriptorAccess:true});
const getter=lookup.call(target,key);const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets,found:typeof getter==='function'}));`)
  assert.equal(data.found,true);assert.equal(data.gets,0)
  const event=data.report.entries.find(e=>e.entries.includes('Object.prototype.__lookupGetter__'))
  assert(event);assert.equal(event.argument.available,false)
})

test('a later argument that changes the target binding is never reconstructed as its new value',()=>{
  const data=fixture(`const native=Object.getOwnPropertyDescriptor;let first=target;
const p=inspectPromiseObservers({descriptorAccess:true});
const result=native(first,(first={},key));const report=p.descriptorAccess();p.close();
console.log(JSON.stringify({report,gets,found:typeof result.get==='function'}));`)
  assert.equal(data.found,true);assert.equal(data.gets,0)
  const event=data.report.entries.find(e=>e.entries.includes('Object.getOwnPropertyDescriptor'))
  assert(event);assert.equal(event.argument.available,false)
})

test('self-replacing argument getter stays unknown without an extra getter invocation',()=>{
  const data=fixture(`const native=Object.getOwnPropertyDescriptor;let calls=0;
const box={get value(){calls++;Object.defineProperty(this,'value',{value:{}});return target}};
const p=inspectPromiseObservers({descriptorAccess:true});const result=native(box.value,key);
const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets,calls,found:typeof result.get==='function'}));`)
  assert.equal(data.found,true);assert.equal(data.gets,0);assert.equal(data.calls,1)
  const event=data.report.entries.find(e=>e.entries.includes('Object.getOwnPropertyDescriptor'))
  assert(event);assert.equal(event.argument.available,false)
})

test('direct syntax deletion remains an explicit uncovered channel',()=>{
  const data=fixture(`const p=inspectPromiseObservers({descriptorAccess:true});delete target.tmp;
const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets,present:'tmp' in target}));`)
  assert.equal(data.present,false);assert.equal(data.gets,0);assert.deepEqual(data.report.entries,[])
})
