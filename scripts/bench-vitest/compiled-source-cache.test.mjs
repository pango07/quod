import test from 'node:test'
import assert from 'node:assert/strict'
import vm from 'node:vm'
import {Session} from 'node:inspector'
import {createHash} from 'node:crypto'
import {compiledScriptRecord,createCompiledScriptRecorder} from './worker-module-probe.mjs'

const hash=(s,encoding='utf8')=>createHash('sha256').update(s,encoding).digest('hex')
const params=(source,extra={})=>({scriptId:'fixture',scriptLanguage:'JavaScript',length:source.length,hash:hash(source),...extra})

test('source cache retains exact code and every identity field across repeated compilations',()=>{
  let reads=0,source='1+1'
  const recorder=createCompiledScriptRecorder(()=>{reads++;return {scriptSource:source}})
  for(const extra of [{},{url:'file:///a'},{url:'b',isModule:true},{url:'c',hasSourceURL:true},{}]) {
    for(const failed of [false,true]) {
      const p=params(source,extra),actual=recorder.record(p,failed)
      assert.deepEqual(actual,compiledScriptRecord(p,{scriptSource:source},failed))
      assert.throws(()=>{actual.code.sha256='changed'},TypeError)
      assert.throws(()=>{actual.identity.hash='changed'},TypeError)
    }
  }
  assert.equal(reads,1);assert.equal(recorder.counts().cacheHits,9)
  source='2+2';assert.equal(recorder.record(params(source)).code.sha256,hash(source,'utf16le'))
  assert.equal(reads,2);assert.equal(recorder.counts().events,11)
})

test('space/NUL collisions and non-ASCII source always fetch complete source',()=>{
  const pairs=[['"a b"','"a\0b"'],['"\ud800"','"\ud801"'],['"é"','"é"']]
  for(const pair of pairs) {
    let source,reads=0
    const recorder=createCompiledScriptRecorder(()=>{reads++;return {scriptSource:source}})
    for(const value of [...pair,...pair]) {
      source=value
      const p=params(source,{hash:hash(source.replaceAll('\0',' '))})
      assert.deepEqual(recorder.record(p),compiledScriptRecord(p,{scriptSource:source}))
    }
    assert.equal(reads,4);assert.equal(recorder.counts().cacheHits,0);assert.equal(recorder.counts().entries,0)
  }
})

test('nested source acquisition cannot count the same cached source twice',()=>{
  let nested=false,recorder
  const source='1+1',p=params(source)
  recorder=createCompiledScriptRecorder(()=>{
    if(!nested){nested=true;recorder.record({...p,url:'nested'})}
    return {scriptSource:source}
  },{maxCachedSources:2})
  assert.deepEqual(recorder.record(p),compiledScriptRecord(p,{scriptSource:source}))
  assert.equal(recorder.counts().entries,1);assert.equal(recorder.counts().sourceReads,2)
  assert.deepEqual(recorder.record(p),compiledScriptRecord(p,{scriptSource:source}))
  assert.equal(recorder.counts().cacheHits,1)
})

test('capacity, absent or mismatched hashes, live edits and Wasm retain full-source fallback',()=>{
  let source='1+1',reads=0
  const recorder=createCompiledScriptRecorder(()=>{reads++;return {scriptSource:source}},{maxCachedSources:1})
  for(const value of ['1+1','2+2','1+1','2+2']){source=value;recorder.record(params(source))}
  assert.equal(reads,3);assert.equal(recorder.counts().entries,1)
  source='1+1'
  for(const extra of [{hash:undefined},{hash:'0'.repeat(64)},{length:2},{length:3.5},{isLiveEdit:true}]) {
    const before=reads,p=params(source,extra)
    assert.deepEqual(recorder.record(p),compiledScriptRecord(p,{scriptSource:source}));assert.equal(reads,before+1)
  }
  let bytecode,wasmReads=0
  const wasm=createCompiledScriptRecorder(()=>{wasmReads++;return {scriptSource:'',bytecode}})
  const p={scriptId:'wasm',scriptLanguage:'WebAssembly',length:4,hash:hash('')}
  for(const bytes of [Buffer.from([0,97,115,109]),Buffer.from([0,97,115,110])]) {
    bytecode=bytes.toString('base64')
    assert.equal(wasm.record(p).code.sha256,hash(bytes))
  }
  assert.equal(wasmReads,2);assert.equal(wasm.counts().cacheHits,0)
  const failed=createCompiledScriptRecorder(()=>{throw Error('missing source')})
  assert.throws(()=>failed.record(params('1+1')),/missing source/)
  assert.equal(failed.counts().entries,0)
})

function sessionWithRecorder(onEvent) {
  const session=new Session()
  function post(method,p={}) {
    let done=false,error,value
    session.post(method,p,(e,r)=>{done=true;error=e;value=r})
    assert(done);if(error)throw error;return value
  }
  session.on('Debugger.scriptParsed',({params})=>onEvent(params,false,post))
  session.on('Debugger.scriptFailedToParse',({params})=>onEvent(params,true,post))
  session.connect();post('Debugger.enable',{maxScriptsCacheSize:0})
  return session
}

test('actual V8 events match complete UTF-16 source records, including a native-hash collision',()=>{
  const errors=[],rows=[],url='quod-native-source-cache-fixture'
  let activePost
  const recorder=createCompiledScriptRecorder(scriptId=>activePost('Debugger.getScriptSource',{scriptId}))
  const session=sessionWithRecorder((p,failed,post)=>{
    if(p.url!==url)return
    try {
      activePost=post
      const full=post('Debugger.getScriptSource',{scriptId:p.scriptId}),actual=recorder.record(p,failed)
      assert.deepEqual(actual,compiledScriptRecord(p,full,failed))
      rows.push({source:full.scriptSource,hash:p.hash,code:actual.code.sha256,failed})
    }catch(e){errors.push(e.message)}
  })
  try {
    for(let i=0;i<64;i++)assert.equal(vm.runInThisContext(i%2?'1+1':'2+2',{filename:url}),i%2?2:4)
    for(const code of ['"a b"','"a\0b"','"\ud800"','"\ud801"'])vm.runInThisContext(code,{filename:url})
    assert.throws(()=>new vm.Script('const=bad',{filename:url}),SyntaxError)
  }finally{session.disconnect()}
  assert.deepEqual(errors,[]);assert.equal(rows.length,69)
  const [space,nul]=rows.slice(64,66)
  assert.equal(space.hash,nul.hash,'the pinned native hash maps NUL to space')
  assert.notEqual(space.code,nul.code,'the recorder must retain the exact source distinction')
  assert.equal(rows.at(-1).failed,true)
  assert.equal(recorder.counts().cacheHits,62);assert.equal(recorder.counts().sourceReads,7)
})

test('measure repeated compilation capture while retaining all occurrences and exact code',t=>{
  const measurements=[]
  for(const cached of [false,true]) {
    const url='quod-source-cache-measurement-'+(cached?'cached':'full'),iterations=20000,errors=[]
    let activePost,events=0,sourceReads=0,matches=0
    const read=scriptId=>{sourceReads++;return activePost('Debugger.getScriptSource',{scriptId})}
    const recorder=cached?createCompiledScriptRecorder(read):null
    const expected=['1+1','2+2'].map(source=>hash(source,'utf16le'))
    const session=sessionWithRecorder((p,failed,post)=>{
      if(p.url!==url)return
      try {
        activePost=post
        const record=recorder?recorder.record(p,failed):compiledScriptRecord(p,read(p.scriptId),failed)
        if(record.code.sha256===expected[events%2]&&record.code.bytes===6)matches++
        events++
      }catch(e){errors.push(e.message)}
    })
    assert.equal(events,0,'enabling a session must not count scripts from the other measurement')
    const started=performance.now()
    try{for(let i=0;i<iterations;i++)vm.runInThisContext(i%2?'2+2':'1+1',{filename:url})}
    finally{session.disconnect()}
    const elapsedMs=performance.now()-started
    assert.deepEqual(errors,[]);assert.equal(events,iterations);assert.equal(matches,iterations)
    assert.equal(sourceReads,cached?2:iterations)
    measurements.push({cached,events,matches,sourceReads,elapsedMs,cache:recorder?.counts()??null})
  }
  t.diagnostic(JSON.stringify({compiledSourceCacheMeasurements:measurements,scope:'isolated compilation recording, not CI savings'}))
})
