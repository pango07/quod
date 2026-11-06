import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {nativeEntryLog,nativeEntryFrames,validateNativeEntryLog,nativeFrameMapper} from './native-entry-log.mjs'
import {scriptMetadataIndex} from './script-metadata-index.mjs'

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

const mapped=(stack,scripts)=>stack.map(f=>({name:f.name,
  location:{scriptId:f.scriptId,lineNumber:f.line-1,columnNumber:f.column-1},script:scripts.get(f.scriptId)??null}))
const site=(i=1)=>({name:'caller'+i,scriptId:String(i),line:i+1,column:8,scriptName:'ignored just as in the original mapping'})
const metadata=(url='file:///exact/source.mjs')=>({url,isModule:true,hasSourceURL:false,language:'JavaScript'})

test('interned frames retain every mapped field, ordering and immutable metadata replacement',()=>{
  const scripts=scriptMetadataIndex({storage:'ranges'}),mapper=nativeFrameMapper(scripts)
  const old=nativeEntryLog(),next=nativeEntryLog(),argument={available:true,kind:'object',target:{watchedAccessorKeys:['tmp']}}
  scripts.set('1',metadata());scripts.set('2',metadata(undefined))
  const first=mapper.map([site(1)])[0]
  assert.equal(mapper.map([site(1)])[0],first)
  assert.throws(()=>{first.name='changed'});assert.throws(()=>{first.location.lineNumber=42})
  assert.throws(()=>{first.script.url='changed'})
  const stacks=[[site(1),site(2)],[site(2),site(1)],[{...site(1),name:'a\u0000b\ud800:2'}],
    [{...site(1),scriptId:'12:foo|,',line:0,column:0}],[{...site(1),name:'12:foo|,'}],
    [{...site(1),name:undefined}],[{...site(1),line:NaN}]]
  for(const url of ['changed','',undefined,'\ud800','\ufffd']) {
    scripts.set('1',{...metadata(),url})
    for(const stack of stacks) {
      old.append(['api'],mapped(stack,scripts),argument)
      next.append(['api'],mapper.map(stack),argument)
    }
  }
  const a=JSON.parse(JSON.stringify(old.report())),b=JSON.parse(JSON.stringify(next.report()))
  validateNativeEntryLog(a);validateNativeEntryLog(b);assert.deepEqual(b,a)
  assert.equal(first.script.url,'file:///exact/source.mjs','old immutable snapshots remain intact')
})

test('unowned metadata, new script IDs and cache capacity keep exact fallback frames',()=>{
  const scripts=new Map([['1',{...metadata(),extra:'retained'}]]),mapper=nativeFrameMapper(scripts,{cacheLimit:1})
  const first=mapper.map([site(1)])[0];scripts.get('1').url='changed'
  const second=mapper.map([site(1)])[0]
  assert.notEqual(first,second);assert.equal(mapper.counts().entries,0)
  assert.deepEqual(second,mapped([site(1)],scripts)[0])
  const owned=scriptMetadataIndex({storage:'ranges'}),bounded=nativeFrameMapper(owned,{cacheLimit:1}),a=nativeEntryLog(),b=nativeEntryLog()
  for(const id of ['1','2']) {
    for(const hasMetadata of [false,true]) {
      if(hasMetadata)owned.set(id,metadata(id))
      const stack=[{...site(1),scriptId:id}]
      a.append(['api'],mapped(stack,owned),{});b.append(['api'],bounded.map(stack),{})
    }
  }
  assert.deepEqual(b.report(),a.report());assert.equal(bounded.counts().entries,1)
  assert(bounded.counts().metadataIdentities<=1)
  for(const limit of [-1,1.5,Infinity,65537])assert.throws(()=>nativeFrameMapper(owned,{cacheLimit:limit}))
})

test('mixed legacy and interned frames deduplicate identical serialized fields',()=>{
  const scripts=scriptMetadataIndex(),mapper=nativeFrameMapper(scripts),log=nativeEntryLog(),stack=[site(1)]
  scripts.set('1',{...metadata(),url:undefined})
  log.append(['api'],mapped(stack,scripts),{})
  log.append(['api'],mapper.map(stack),{})
  const report=log.report();validateNativeEntryLog(report)
  assert.equal(report.calls,2);assert.equal(report.frames.length,1)
  assert.equal(report.entries.length,1);assert.equal(report.entries[0].count,2)
})

test('owned frame keys never invoke inherited serialization hooks',()=>{
  const scripts=scriptMetadataIndex(),mapper=nativeFrameMapper(scripts);scripts.set('1',metadata())
  const original=Object.getOwnPropertyDescriptor(Object.prototype,'toJSON')
  let calls=0,frames
  Object.defineProperty(Object.prototype,'toJSON',{configurable:true,get(){calls++;throw Error('inherited hook')}})
  try{frames=mapper.map([site(1),{...site(1),name:'changed'}]);mapper.map([site(1)])}
  finally{if(original)Object.defineProperty(Object.prototype,'toJSON',original);else delete Object.prototype.toJSON}
  assert.equal(calls,0)
  const log=nativeEntryLog();log.append(['api'],frames,{})
  assert.deepEqual(nativeEntryFrames(log.report(),log.report().entries[0]),mapped([site(1),{...site(1),name:'changed'}],scripts))
})

test('nested frame freezing rechecks cache capacity before retaining a frame',()=>{
  const moduleURL=new URL('./native-entry-log.mjs',import.meta.url).href
  const indexURL=new URL('./script-metadata-index.mjs',import.meta.url).href
  const source=`const freeze=Object.freeze;let mapper,armed=false,nested;
Object.freeze=value=>{if(armed){armed=false;nested=mapper.map([{name:'nested',scriptId:'1',line:1,column:1}])[0]}return freeze(value)};
const {nativeFrameMapper}=await import(${JSON.stringify(moduleURL)}),{scriptMetadataIndex}=await import(${JSON.stringify(indexURL)});
const scripts=scriptMetadataIndex();scripts.set('1',{url:'source',isModule:true,hasSourceURL:false,language:'JavaScript'});
mapper=nativeFrameMapper(scripts,{cacheLimit:1});armed=true;
const outer=mapper.map([{name:'outer',scriptId:'1',line:2,column:2}])[0];Object.freeze=freeze;
console.log(JSON.stringify({outer,nested,counts:mapper.counts()}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error));const row=JSON.parse(r.stdout)
  assert.equal(row.outer.name,'outer');assert.equal(row.nested.name,'nested');assert.equal(row.counts.entries,1)
})

test('compare complete native frame records with fresh and interned mapping',t=>{
  const moduleURL=new URL('./native-entry-log.mjs',import.meta.url).href,indexURL=new URL('./script-metadata-index.mjs',import.meta.url).href,rows=[]
  for(const cacheLimit of [0,20000,20000,0]) {
    const source=`import {createHash} from 'node:crypto';
import {nativeFrameMapper,nativeEntryLog,validateNativeEntryLog} from ${JSON.stringify(moduleURL)};
import {scriptMetadataIndex} from ${JSON.stringify(indexURL)};
const scripts=scriptMetadataIndex({storage:'ranges'}),mapper=nativeFrameMapper(scripts,{cacheLimit:${cacheLimit}}),log=nativeEntryLog();
for(let i=0;i<32;i++)scripts.set(String(i),{url:'file:///source'+i+'.mjs',isModule:!!(i&1),hasSourceURL:!!(i&2),language:i&4?'WebAssembly':'JavaScript'});
const stacks=Array.from({length:64},(_,variant)=>Array.from({length:24},(_,i)=>({name:'caller'+i,scriptId:String(i),line:i+variant+1,column:8})));
const argument={available:true,kind:'object',actualNativeEntry:true,argumentCount:2,receiverBased:false,replayEligible:false,target:{watchedAccessorKeys:['tmp']}};
const start=performance.now();for(let i=0;i<100000;i++)log.append(['Object.getOwnPropertyDescriptor'],mapper.map(stacks[i%64]),argument);
const elapsedMs=performance.now()-start,report=log.report();validateNativeEntryLog(report);
const bytes=JSON.stringify(report);console.log(JSON.stringify({cacheLimit:${cacheLimit},elapsedMs,calls:report.calls,frames:report.frames.length,entries:report.entries.length,sha256:createHash('sha256').update(bytes).digest('hex'),serializedBytes:Buffer.byteLength(bytes),cache:mapper.counts()}));`
    const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:40000})
    assert.equal(r.status,0,r.stderr||String(r.error));const row=JSON.parse(r.stdout)
    assert.equal(row.calls,100000);assert.equal(row.frames,1536);assert.equal(row.entries,64)
    if(rows.length){assert.equal(row.sha256,rows[0].sha256);assert.equal(row.serializedBytes,rows[0].serializedBytes)}
    assert.equal(row.cache.entries,cacheLimit?1536:0);rows.push(row)
  }
  t.diagnostic(JSON.stringify({nativeFrameMappingMeasurements:rows,scope:'isolated exact frame mapping/logging; not workload or CI savings'}))
})
