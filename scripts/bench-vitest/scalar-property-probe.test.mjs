import test from 'node:test'
import assert from 'node:assert/strict'
import {inspect,types} from 'node:util'
import {observeScalarProperty} from './scalar-property-probe.mjs'

test('observes reads through original and non-plain aliases without replacing object identity',()=>{
  const sequence={seed:123,shuffle:false},alias=sequence
  class Holder {constructor(value){this.value=value}read(){return this.value.seed}}
  const holder=new Holder(sequence),original=Object.getOwnPropertyDescriptor(sequence,'seed')
  const p=observeScalarProperty(sequence,'seed')
  assert.equal(p.value,sequence);assert.equal(types.isProxy(sequence),false)
  assert.equal(alias.seed,123);assert.equal(holder.read(),123)
  assert.equal(p.report().events.reduce((n,e)=>n+e.count,0),2)
  assert(p.report().events.every(e=>e.operation==='get'&&e.sites.length>0))
  assert.equal(Reflect.set(sequence,'seed',456),true);assert.equal(alias.seed,456)
  assert.equal(p.report().intact,true);assert.deepEqual(p.report().gaps,[])
  p.close();assert.deepEqual(Object.getOwnPropertyDescriptor(sequence,'seed'),{...original,value:456})
})

test('a primitive copied before installation remains an explicit unobserved channel',()=>{
  const sequence={seed:'private-original-value'},copied=sequence.seed,p=observeScalarProperty(sequence,'seed')
  assert.equal(copied,'private-original-value');assert.deepEqual(p.report().events,[])
  assert.equal(p.report().earlierCopiesExcluded,false);assert.equal(p.report().completeOwnership,false)
  assert(!JSON.stringify(p.report()).includes('private-original-value'))
  p.close()
})

test('descriptor and native inspection differences remain explicit instead of authorizing reuse',()=>{
  const sequence={seed:123},before=Object.getOwnPropertyDescriptor(sequence,'seed'),p=observeScalarProperty(sequence,'seed')
  const after=Object.getOwnPropertyDescriptor(sequence,'seed')
  assert.equal(before.value,123);assert.equal(after.value,undefined);assert.equal(typeof after.get,'function')
  assert.match(inspect(sequence),/Getter/);assert.deepEqual(p.report().events,[])
  assert.match(inspect(sequence,{getters:true}),/123/)
  assert(p.report().events.some(e=>e.operation==='get'))
  assert.equal(p.report().descriptorObservable,true);assert.equal(p.report().nativeReadsExcluded,false)
  assert.equal(p.report().replayEligible,false)
  p.close()
})

test('unsupported owners and descriptors do not invoke application getters or proxy traps',()=>{
  let calls=0
  for(const target of [{get seed(){calls++;return 1}},Object.freeze({seed:1}),
    new Proxy({seed:1},{getOwnPropertyDescriptor(){calls++;return {value:1,configurable:true,writable:true}}})]) {
    const p=observeScalarProperty(target,'seed')
    assert.equal(p.report().installed,false);assert(p.report().gaps.length>0);p.close()
  }
  assert.equal(calls,0)
})

test('restoring the accessor does not prove the absence of intermediate descriptor replacement',()=>{
  const sequence={seed:123},p=observeScalarProperty(sequence,'seed'),installed=Object.getOwnPropertyDescriptor(sequence,'seed')
  Object.defineProperty(sequence,'seed',{value:999,writable:true,configurable:true,enumerable:true})
  assert.equal(sequence.seed,999);assert.equal(p.report().intact,false)
  Object.defineProperty(sequence,'seed',installed)
  assert.equal(p.report().intact,true);assert.deepEqual(p.report().events,[])
  assert.equal(p.report().completeOwnership,false);assert.equal(p.report().replayEligible,false)
  p.close()
})

test('alternate write receivers and object replacement stay unsupported while ordinary values remain usable',()=>{
  const sequence={seed:123},p=observeScalarProperty(sequence,'seed'),child=Object.create(sequence)
  child.seed=456
  assert.equal(child.seed,456);assert.equal(sequence.seed,123)
  assert(p.report().gaps.includes('alternate scalar write receiver'))
  const value={changed:true};sequence.seed=value;assert.equal(sequence.seed,value)
  assert(p.report().gaps.includes('scalar replaced by object'))
  p.close();assert.equal(Object.getOwnPropertyDescriptor(sequence,'seed').value,value)
})
