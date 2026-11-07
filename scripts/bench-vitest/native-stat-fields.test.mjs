// Bounded feasibility test on actual Node Stats objects. This does not
// install a production observer or grant replay: publication-time ownership
// and native/debugger observer exclusion remain separate requirements.
import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {createRequire} from 'node:module'
import {inspect} from 'node:util'
import {serialize,deserialize} from 'node:v8'

const addon=createRequire(import.meta.url)(process.env.QUOD_NATIVE_CAPTURE_ADDON)
const directory=fs.mkdtempSync(path.join(os.tmpdir(),'quod-native-stat-fields-'))
const file=path.join(directory,'input')
fs.writeFileSync(file,'unchanged bytes')
test.after(()=>fs.rmSync(directory,{recursive:true,force:true}))

function watch(stats) {
  const fields=Reflect.ownKeys(stats),reads=new Map(),observers=[]
  try {
    for(const key of fields)observers.push(addon.createDataReadObserver(stats,key,()=>
      reads.set(key,(reads.get(key)??0)+1)))
  }catch(error){for(const observer of observers)observer.close();throw error}
  return {
    fields,reads,
    report(){return {fields:[...reads.keys()].sort(),calls:[...reads.values()].reduce((a,b)=>a+b,0),
      available:observers.every(o=>o.report().available),replayEligible:false}},
    close(){for(const observer of observers)observer.close()},
  }
}

const consumers=[
  ['type',s=>s.isFile()],['size',s=>s.size],['timestamp',s=>s.mtimeMs],
  ['date',s=>s.mtime.getTime()],['inode',s=>s.ino],
  ['descriptor',s=>Object.getOwnPropertyDescriptor(s,'mtimeMs').value],
  ['reflect descriptor',s=>Reflect.getOwnPropertyDescriptor(s,'mtimeMs').value],
  ['spread',s=>({...s})],['inspect',s=>inspect(s)],
  ['structured clone',s=>structuredClone(s)],['v8 serialization',s=>deserialize(serialize(s))],
]
for(const bigint of [false,true])test(`native Stats consumer semantics, bigint=${bigint}`,t=>{
  const rows=[]
  for(const [name,consume] of consumers) {
    const plain=fs.statSync(file,{bigint}),observed=fs.statSync(file,{bigint})
    const before=Object.getOwnPropertyDescriptors(plain),prototype=Object.getPrototypeOf(plain)
    const tracker=watch(observed)
    try {
      assert.equal(Object.getPrototypeOf(observed),prototype)
      assert.deepEqual(Reflect.ownKeys(observed),Reflect.ownKeys(plain))
      assert.deepEqual(consume(observed),consume(plain),name)
      const report=tracker.report()
      assert.equal(report.available,true,name)
      if(name==='type')assert.deepEqual(report.fields,['mode'])
      if(name==='size')assert.deepEqual(report.fields,['size'])
      if(name==='timestamp'||name.includes('descriptor'))assert.deepEqual(report.fields,['mtimeMs'])
      if(name==='inode')assert.deepEqual(report.fields,['ino'])
      if(name==='date')assert.ok(report.fields.includes('mtimeMs')||report.fields.includes('mtimeNs')||report.fields.includes('mtime'))
      assert.ok(report.calls>0,name+' must observe at least one metadata value')
      rows.push({consumer:name,...report})
      // Reflection itself reads values, so check shape after the measured
      // operation, preserving the operation's separate field-read report.
      assert.deepEqual(Object.getOwnPropertyDescriptors(observed),Object.getOwnPropertyDescriptors(plain))
      assert.equal(before.size.value,plain.size)
    }finally{tracker.close()}
  }
  t.diagnostic(JSON.stringify({nativeStatsFields:{bigint,rows},replayEligible:false}))
})

test('same content and size retain type/size projection while timestamp consumers change',t=>{
  function phase(seconds) {
    fs.utimesSync(file,seconds,seconds)
    const stats=fs.statSync(file),tracker=watch(stats)
    try {
      const projected={file:stats.isFile(),size:stats.size}
      const typeSize=tracker.report()
      assert.deepEqual(typeSize.fields,['mode','size'])
      const timestamp=stats.mtimeMs
      assert.deepEqual(tracker.report().fields,['mode','mtimeMs','size'])
      return {projected,timestamp,typeSize}
    }finally{tracker.close()}
  }
  const before=phase(1700000000),after=phase(1700000060)
  assert.deepEqual(before.projected,after.projected)
  assert.notEqual(before.timestamp,after.timestamp)
  assert.equal(fs.readFileSync(file,'utf8'),'unchanged bytes')
  t.diagnostic(JSON.stringify({metadataMutation:{before,after},replayEligible:false}))
})

test('async stat fulfillment preserves native Stats and tracks later reads',async()=>{
  const stats=await fs.promises.stat(file),tracker=watch(stats)
  try {
    assert.equal(stats instanceof fs.Stats,true)
    assert.equal(stats.isFile(),true)
    assert.equal(stats.size,Buffer.byteLength('unchanged bytes'))
    assert.deepEqual(tracker.report().fields,['mode','size'])
    assert.equal(tracker.report().available,true)
  }finally{tracker.close()}
})

test('callback replacement on native Stats rejects the observation',()=>{
  const stats=fs.statSync(file),tracker=watch(stats)
  try {
    const value=stats.size
    stats.size=value
    assert.equal(tracker.report().available,false)
    assert.equal(stats.size,value)
  }finally{tracker.close()}
})

test('late installation cannot certify a timestamp copied by an earlier consumer',async t=>{
  const promise=fs.promises.stat(file)
  let earlierTimestamp
  // This reference consumer deliberately runs before installation. A
  // production factory must exclude such a path; intact callbacks alone
  // cannot establish that the result had no earlier observers.
  const first=promise.then(stats=>{earlierTimestamp=stats.mtimeMs})
  await first
  const stats=await promise,tracker=watch(stats)
  try {
    assert.equal(stats.size,Buffer.byteLength('unchanged bytes'))
    const report=tracker.report()
    assert.equal(report.available,true)
    assert.deepEqual(report.fields,['size'])
    assert.equal(typeof earlierTimestamp,'number')
    assert.equal(report.replayEligible,false)
    t.diagnostic(JSON.stringify({earlierConsumer:{timestampWasCopied:true,
      timestampReadObserved:report.fields.includes('mtimeMs'),callbacksIntact:report.available},replayEligible:false}))
  }finally{tracker.close()}
})
