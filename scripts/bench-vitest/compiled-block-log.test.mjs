import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {blockCompiledScriptLog,blockCompilationRuns} from './compiled-block-log.mjs'
import {validateCompilationCounts,compilationSequence,dictionaryCompiledScriptLog} from './compiled-script-log.mjs'
import {createCompiledScriptRecorder} from './worker-module-probe.mjs'
import {createHash} from 'node:crypto'

const record=i=>({identity:{available:true,hash:'identity-'+i},code:{encoding:'utf16le',bytes:2,sha256:'code-'+i},language:'JavaScript'})
const restore=report=>Array.from(blockCompilationRuns(report)).flatMap(([id,count])=>{
  const {count:total,...row}=report.scripts[id]
  return Array.from({length:count},()=>row)
})

test('owned serialization and mutable fallback retain identical full compilation records',()=>{
  const source='1+1',params={scriptId:'1',length:source.length,hash:createHash('sha256').update(source).digest('hex')}
  const recorder=createCompiledScriptRecorder(()=>({scriptSource:source})),cached=blockCompiledScriptLog({ownedRecordKey:recorder.cachedKey}),plain=blockCompiledScriptLog()
  const owned=recorder.record(params),mutable={...owned,code:{...owned.code}},input=[]
  for(let i=0;i<1027;i++) {
    mutable.code.bytes=i
    const row=i%2?owned:mutable
    input.push(structuredClone(row));cached.append(row);plain.append(row)
  }
  validateCompilationCounts(cached.report());assert.deepEqual(cached.report(),plain.report())
  assert.deepEqual(restore(cached.report()),input)
})

test('compare repeated owned compilation serialization while preserving every event',t=>{
  const moduleURL=new URL('./worker-module-probe.mjs',import.meta.url).href,logURL=new URL('./compiled-block-log.mjs',import.meta.url).href,
    validateURL=new URL('./compiled-script-log.mjs',import.meta.url).href,rows=[]
  for(const cached of [false,true,true,false]) {
    const source=`import {createHash} from 'node:crypto';
import {createCompiledScriptRecorder} from ${JSON.stringify(moduleURL)};
import {blockCompiledScriptLog} from ${JSON.stringify(logURL)};
import {validateCompilationCounts} from ${JSON.stringify(validateURL)};
const code=['1+1','2+2'],recorder=createCompiledScriptRecorder(id=>({scriptSource:code[Number(id)]}));
const records=code.map((source,i)=>recorder.record({scriptId:String(i),length:source.length,hash:createHash('sha256').update(source).digest('hex')}));
const log=blockCompiledScriptLog(${cached?'{ownedRecordKey:recorder.cachedKey}':'{}'}),start=performance.now();
for(let i=0;i<2000000;i++)log.append(records[i%2]);
const elapsedMs=performance.now()-start,report=log.report();validateCompilationCounts(report);
const serialized=JSON.stringify(report);console.log(JSON.stringify({cached:${cached},elapsedMs,counts:log.counts(),serializedBytes:Buffer.byteLength(serialized),sha256:createHash('sha256').update(serialized).digest('hex')}));`
    const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:30000})
    assert.equal(r.status,0,r.stderr||String(r.error));const row=JSON.parse(r.stdout)
    assert.equal(row.counts.events,2000000);assert.equal(row.counts.runs,2000000);assert.equal(row.counts.records,2)
    assert.equal(row.counts.overflow,false)
    if(rows.length){assert.equal(row.sha256,rows[0].sha256);assert.equal(row.serializedBytes,rows[0].serializedBytes)}
    rows.push(row)
  }
  t.diagnostic(JSON.stringify({compiledRecordKeyMeasurements:rows,scope:'isolated exact record serialization/logging; not workload or CI savings'}))
})

test('blocks preserve every record field, event and position through interrupted repeated patterns',()=>{
  const input=Array.from({length:7001},(_,i)=>record(i%4))
  for(const at of [0,255,256,257,2047,2048,6999])input[at]={...input[at],language:'WebAssembly'}
  input.splice(1023,0,input[1022],input[1022]);input.push({...record(0),code:{...record(0).code,sha256:'changed'}})
  const log=blockCompiledScriptLog({blockSize:16})
  for(const row of input)log.append(row)
  const report=log.report();validateCompilationCounts(report)
  assert.equal(report.events,input.length);assert.deepEqual(restore(report),input)
  assert(log.counts().storedPairs<input.length/4)
})

test('reports do not mutate the live tail and cannot change retained records or block indexes',()=>{
  const log=blockCompiledScriptLog({blockSize:2}),input=[record(0),record(1),record(2),record(3)]
  for(const row of input)log.append(row)
  const before=log.report();validateCompilationCounts(before)
  before.scripts[0].code.sha256='poison';before.blocks[0][0]=99;before.blockRuns[0][1]=99;before.tail[0]=99
  input[0].code.sha256='later mutation'
  log.append(record(3));log.append(record(0))
  const report=log.report();validateCompilationCounts(report)
  assert.deepEqual(restore(report),[record(0),record(1),record(2),record(3),record(3),record(0)])
  assert.equal(log.counts().runs,5)
})

test('stored pair and record limits reject incomplete logs without dropping the overflow flag',()=>{
  for(const log of [blockCompiledScriptLog({recordLimit:1}),blockCompiledScriptLog({blockSize:2,pairLimit:3})]) {
    for(let i=0;i<10;i++)log.append(record(i))
    const report=log.report();assert.equal(report.events,10);assert.equal(report.overflow,true)
    assert.throws(()=>validateCompilationCounts(report))
    assert(log.counts().storedPairs<=log.counts().pairLimit)
  }
  for(const options of [{blockSize:0},{blockSize:4097},{pairLimit:0},{recordLimit:NaN},{pairLimit:1.5}])
    assert.throws(()=>blockCompiledScriptLog(options))
  const empty=blockCompiledScriptLog().report();validateCompilationCounts(empty);assert.deepEqual(restore(empty),[])
})

test('outer block runs retain order across dense storage chunks',()=>{
  const input=Array.from({length:20004},(_,i)=>record(i%4)),log=blockCompiledScriptLog({blockSize:2})
  for(const row of input)log.append(row)
  const report=log.report();validateCompilationCounts(report)
  assert(log.counts().blockRuns>4096);assert.deepEqual(restore(report),input)
})

test('validation rejects corrupt block references, counts, incomplete blocks and arithmetic overflow',()=>{
  const log=blockCompiledScriptLog({blockSize:2})
  for(let i=0;i<100;i++)log.append(record(i%4))
  const report=log.report();validateCompilationCounts(report)
  for(const change of [r=>r.events++,r=>r.blocks[0][0]=999,r=>r.blocks[0][1]=0,r=>r.blocks[0].pop(),
    r=>r.blockRuns[0][0]=999,r=>r.blockRuns[0][1]=0,r=>r.blockRuns[0][1]=Number.MAX_SAFE_INTEGER,
    r=>r.tail.push(0),r=>r.tail[0]=0.5,r=>delete r.blocks,r=>r.scripts[0].count--,
    r=>r.blocks.push([...r.blocks[0]]),r=>r.tail.push(0,1)]) {
    const bad=structuredClone(report);change(bad);assert.throws(()=>validateCompilationCounts(bad))
  }
  // A valid reordering with the same counts must still compare differently.
  const swapped=structuredClone(report);[swapped.blocks[0][0],swapped.blocks[0][2]]=[swapped.blocks[0][2],swapped.blocks[0][0]]
  validateCompilationCounts(swapped);assert.notDeepEqual(compilationSequence(report),compilationSequence(swapped))
})

test('block keys do not consult inherited array serialization',()=>{
  const log=blockCompiledScriptLog({blockSize:2});let calls=0
  const original=Object.getOwnPropertyDescriptor(Array.prototype,'toJSON')
  Object.defineProperty(Array.prototype,'toJSON',{configurable:true,value(){calls++;throw Error('application serializer')}})
  try {for(let i=0;i<100;i++)log.append(record(i%2))}
  finally {if(original)Object.defineProperty(Array.prototype,'toJSON',original);else delete Array.prototype.toJSON}
  const report=log.report();validateCompilationCounts(report)
  assert.equal(calls,0);assert.equal(report.events,100);assert.equal(report.blocks.length,1)
})

test('measure twelve million alternating events with every restored occurrence checked',t=>{
  const blocks=new URL('./compiled-block-log.mjs',import.meta.url).href,logs=new URL('./compiled-script-log.mjs',import.meta.url).href
  const source=`import {blockCompiledScriptLog,blockCompilationRuns} from ${JSON.stringify(blocks)};
import {validateCompilationCounts} from ${JSON.stringify(logs)};
const records=[{code:'a'},{code:'b'}],events=12000000;globalThis.gc();const before=process.memoryUsage(),start=performance.now();
const log=blockCompiledScriptLog();for(let i=0;i<events;i++)log.append(records[i%2]);
const appendMs=performance.now()-start;globalThis.gc();const after=process.memoryUsage(),report=log.report();validateCompilationCounts(report);
let matches=0;for(const [id,count] of blockCompilationRuns(report)){if(count!==1||id!==matches%2)throw Error('sequence differs');matches++;}
console.log(JSON.stringify({events:report.events,matches,appendMs,heapDelta:after.heapUsed-before.heapUsed,arrayBufferDelta:after.arrayBuffers-before.arrayBuffers,serializedBytes:Buffer.byteLength(JSON.stringify(report)),counts:log.counts()}));`
  const r=spawnSync(process.execPath,['--expose-gc','--input-type=module','-e',source],{encoding:'utf8',timeout:60000})
  assert.equal(r.status,0,r.stderr||String(r.error));const row=JSON.parse(r.stdout)
  assert.equal(row.events,12000000);assert.equal(row.matches,row.events);assert.equal(row.counts.overflow,false)
  assert(row.counts.storedPairs<1024);assert(row.serializedBytes<10000)
  t.diagnostic(JSON.stringify({compilationBlockMeasurement:row,scope:'isolated lossless log storage; not CI savings'}))
})

test('block log agrees with the original dictionary on an irregular deterministic sequence',()=>{
  const a=blockCompiledScriptLog({blockSize:7}),b=dictionaryCompiledScriptLog(),input=[]
  let state=123456789
  for(let i=0;i<10000;i++) {
    state=(Math.imul(state,1664525)+1013904223)>>>0
    const row=record(state%19);input.push(row);a.append(row);b.append(row)
  }
  const x=a.report(),y=b.report();validateCompilationCounts(x);validateCompilationCounts(y)
  assert.deepEqual(x.scripts,y.scripts);assert.deepEqual(Array.from(blockCompilationRuns(x)),y.runs)
  assert.deepEqual(restore(x),input)
})
