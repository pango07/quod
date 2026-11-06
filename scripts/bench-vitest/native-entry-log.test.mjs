import test from 'node:test'
import assert from 'node:assert/strict'
import {nativeEntryLog,nativeEntryFrames,validateNativeEntryLog} from './native-entry-log.mjs'

const frame=i=>({name:'caller'+i,location:{scriptId:'1',lineNumber:i,columnNumber:7},
  script:{url:'file:///exact/source.mjs',isModule:true,hasSourceURL:false,language:'JavaScript'}})

test('more than 10000 distinct native stacks preserve every frame field, stack order and occurrence count',()=>{
  const log=nativeEntryLog(),prefix=Array.from({length:20},(_,i)=>frame(i)),inputs=[]
  for(let i=0;i<12001;i++) {
    const stack=[...prefix,frame(20+i%101),frame(200+Math.floor(i/101))]
    const argument={available:true,kind:'object',target:{ownAccessorCount:1,watchedAccessorKeys:['tmp']}}
    inputs.push({entries:['Object.getOwnPropertyDescriptor'],frames:stack,argument,count:2})
    log.append(inputs[i].entries,stack,argument);log.append(inputs[i].entries,stack,argument)
  }
  const report=JSON.parse(JSON.stringify(log.report()));validateNativeEntryLog(report)
  assert.equal(report.calls,24002);assert.equal(report.entries.length,12001)
  assert.deepEqual(report.entries.map(({frameIds,...entry})=>({...entry,frames:nativeEntryFrames(report,{frameIds})})),inputs)
  assert(JSON.stringify(report).length<JSON.stringify(inputs).length/5,
    'repeated full frame bodies must no longer dominate the recorded evidence')
})

test('different locations, script identities, frame order and source metadata remain distinct',()=>{
  const a=frame(1),b=frame(2),argument={available:true,kind:'primitive'},log=nativeEntryLog()
  const stacks=[[a,b],[b,a],[a,{...b,name:'changed'}],[a,{...b,location:{...b.location,scriptId:'2'}}],
    [a,{...b,script:{...b.script,hasSourceURL:true}}],[a,{...b,script:{...b.script,extra:'\ud800'}}]]
  for(const stack of stacks)log.append(['api'],stack,argument)
  const report=JSON.parse(JSON.stringify(log.report()));validateNativeEntryLog(report)
  assert.equal(report.entries.length,stacks.length)
  assert.deepEqual(report.entries.map(e=>nativeEntryFrames(report,e)),stacks)
})

test('frame and entry limits remain explicit failures, including when counting later repeats',()=>{
  for(const log of [nativeEntryLog({entryLimit:1}),nativeEntryLog({frameLimit:1})]) {
    log.append(['api'],[frame(1)],{});log.append(['api'],[frame(2)],{});log.append(['api'],[frame(1)],{})
    assert.equal(log.counts().calls,3);assert.equal(log.report().overflow,true)
    assert.throws(()=>validateNativeEntryLog(log.report()))
  }
})

test('malformed references and missing occurrences cannot validate as complete evidence',()=>{
  const log=nativeEntryLog();log.append(['api'],[frame(1)],{})
  for(const change of [r=>r.calls++,r=>r.entries[0].count=0,r=>r.entries[0].frameIds[0]=-1,
    r=>r.entries[0].frameIds[0]=1,r=>r.entries[0].frameIds[0]=0.5,r=>delete r.entries[0].frameIds]) {
    const report=structuredClone(log.report());change(report)
    assert.throws(()=>validateNativeEntryLog(report))
  }
})
