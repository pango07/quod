import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {scriptIdRanges} from './script-id-ranges.mjs'
import {scriptMetadataIndex} from './script-metadata-index.mjs'

test('intervals preserve holes, overwrites, splits and rejoining at either boundary',()=>{
  const store=scriptIdRanges(),expected=new Map()
  const operations=[[1,1],[3,1],[2,1],[0,1],[4,1],[2,2],[0,2],[4,2],[1,2],[3,2],
    [2,3],[1,3],[0,3],[4,3],[3,3],[7,4],[6,4],[5,4],[6,3],[5,3],[7,3],
    [Number.MAX_SAFE_INTEGER,9],[Number.MAX_SAFE_INTEGER-2,9],[Number.MAX_SAFE_INTEGER-1,9]]
  for(const [id,value] of operations) {
    assert(store.set(id,value));expected.set(id,value)
    for(let i=0;i<10;i++)assert.equal(store.get(i),expected.get(i),'lookup '+i)
    for(const [id,value] of expected)assert.equal(store.get(id),value)
  }
  assert.equal(store.counts().ranges,2)
})

test('range capacity rejects splits atomically while exact overwrites and merges remain possible',()=>{
  const store=scriptIdRanges(2)
  for(let i=0;i<5;i++)assert(store.set(i,1))
  assert.equal(store.set(2,2),false)
  for(let i=0;i<5;i++)assert.equal(store.get(i),1)
  assert(store.set(0,2));assert.equal(store.counts().ranges,2)
  assert.equal(store.set(3,2),false);assert.equal(store.get(3),1)
  assert(store.set(1,2));assert(store.set(2,2));assert(store.set(3,2));assert(store.set(4,2))
  assert.equal(store.counts().ranges,1)
  for(const [id,value] of [[-1,1],[1.5,1],[NaN,1],[1,0],[1,NaN]])assert.throws(()=>store.set(id,value))
})

test('numeric range updates match a Map through irregular insertions and overwrites',()=>{
  const store=scriptIdRanges(),expected=new Map();let state=17
  for(let i=0;i<20000;i++) {
    state=(Math.imul(state,1664525)+1013904223)>>>0
    const id=state%1024,value=(state>>>16)%7+1
    assert(store.set(id,value));expected.set(id,value)
    for(const n of [id,0,1023,(id+1)%1024,(id+1023)%1024])assert.equal(store.get(n),expected.get(n))
  }
  for(let i=0;i<1024;i++)assert.equal(store.get(i),expected.get(i))
})

test('range entries survive typed-storage boundaries, reverse insertions and a full rejoin',()=>{
  const store=scriptIdRanges(10000)
  for(let i=0;i<5000;i++)assert(store.set(i*2,i+1))
  for(let i=4999;i>=0;i--)assert(store.set(i*2+1,5001+i))
  for(let i=0;i<10000;i++)assert.equal(store.get(i),i%2?5001+(i-1)/2:i/2+1)
  for(let i=9999;i>=0;i--)assert(store.set(i,1))
  assert.equal(store.counts().ranges,1)
  for(let i=0;i<10000;i++)assert.equal(store.get(i),1)
})

test('metadata range mode keeps noncanonical IDs distinct and fails closed at physical limits',()=>{
  const m={url:undefined,isModule:false,hasSourceURL:false,language:'JavaScript'}
  const index=scriptMetadataIndex({storage:'ranges',rangeLimit:2,sparseLimit:2})
  for(const id of ['1','2','3','01','-0'])assert(index.set(id,m))
  assert.equal(index.counts().ranges,1);assert.equal(index.counts().entries,5)
  assert(index.has('01'));assert(index.has('-0'));assert(!index.has('0'))
  assert.equal(index.set('another',m),false);assert(index.counts().overflow)
  const split=scriptMetadataIndex({storage:'ranges',rangeLimit:2})
  for(const id of ['1','2','3'])assert(split.set(id,m))
  assert.equal(split.set('2',{...m,url:'different'}),false);assert(split.counts().overflow)
  assert.deepEqual(split.get('2'),m);assert.equal(split.counts().metadata,1)
})

test('range commits use no inherited numeric array setters or replaced intrinsics',()=>{
  const url=new URL('./script-id-ranges.mjs',import.meta.url).href
  const source=`import {scriptIdRanges} from ${JSON.stringify(url)};
const store=scriptIdRanges(),push=Array.prototype.push;let calls=0,values;
Object.defineProperty(Array.prototype,'0',{configurable:true,set(){calls++;throw Error('setter')}});
Array.prototype.push=()=>{calls++;throw Error('push')};
try {store.set(1,1);store.set(3,1);store.set(2,1);store.set(2,2);values={a:store.get(1),b:store.get(2),c:store.get(3)}}
finally {delete Array.prototype[0];Array.prototype.push=push}
console.log(JSON.stringify({values,calls,counts:store.counts()}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error));const data=JSON.parse(r.stdout)
  assert.equal(data.calls,0);assert.deepEqual(data.values,{a:1,b:2,c:1});assert.equal(data.counts.ranges,3)
})

test('twelve million metadata IDs retain exact lookup beyond the former entry limit',t=>{
  const url=new URL('./script-metadata-index.mjs',import.meta.url).href
  const source=`import {scriptMetadataIndex} from ${JSON.stringify(url)};
globalThis.gc();const before=process.memoryUsage(),start=performance.now(),index=scriptMetadataIndex({storage:'ranges'});
const metadata={url:'',isModule:false,hasSourceURL:false,language:'JavaScript'},count=12000000;
for(let i=0;i<count;i++)if(!index.set(String(i),metadata))throw Error('index incomplete');
const insertMs=performance.now()-start;globalThis.gc();const after=process.memoryUsage();let matches=0;
for(let i=0;i<count;i++){const m=index.get(String(i));if(m.url===''&&!m.isModule&&!m.hasSourceURL&&m.language==='JavaScript')matches++;}
console.log(JSON.stringify({matches,insertMs,heapDelta:after.heapUsed-before.heapUsed,arrayBufferDelta:after.arrayBuffers-before.arrayBuffers,counts:index.counts()}));`
  const r=spawnSync(process.execPath,['--expose-gc','--input-type=module','-e',source],{encoding:'utf8',timeout:60000})
  assert.equal(r.status,0,r.stderr||String(r.error));const data=JSON.parse(r.stdout)
  assert.equal(data.matches,12000000);assert.equal(data.counts.entries,data.matches)
  assert.equal(data.counts.events,data.matches);assert.equal(data.counts.ranges,1)
  assert.equal(data.counts.overflow,false);assert.equal(data.counts.failures,0)
  t.diagnostic(JSON.stringify({scriptRangeMeasurement:data,scope:'isolated exact metadata-ID storage; not CI savings'}))
})
