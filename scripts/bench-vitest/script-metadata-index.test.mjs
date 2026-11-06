import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {scriptMetadataIndex} from './script-metadata-index.mjs'
const metadata=i=>({url:'file:///source'+i+'.mjs',isModule:!!(i&1),hasSourceURL:!!(i&2),language:i&4?'WebAssembly':'JavaScript'})

test('script metadata index preserves dense, sparse and noncanonical IDs and overwrites',()=>{
  const index=scriptMetadataIndex(),expected=new Map()
  const ids=['0','1','01','-0','-1','4095','4096','4194303','4194304','9007199254740991','9007199254740992','NaN','', 'foreign\ud800']
  for(let i=0;i<ids.length;i++){const value=metadata(i);assert(index.set(ids[i],value));expected.set(ids[i],value)}
  assert(index.set('1',metadata(42)));expected.set('1',metadata(42))
  for(const [id,value] of expected){assert(index.has(id));assert.deepEqual(index.get(id),value)}
  for(const id of ['missing','2','4097',null,1]){assert.equal(index.has(id),false);assert.equal(index.get(id),undefined)}
  assert.equal(index.counts().entries,expected.size);assert.equal(index.counts().events,ids.length+1)
  assert.equal(index.counts().overflow,false)
})

test('metadata interning preserves every field and owns immutable copies',()=>{
  const index=scriptMetadataIndex(),a=metadata(0),variants=[a,{...a,url:'\ud800'},{...a,url:'\ufffd'},
    {...a,isModule:true},{...a,hasSourceURL:true},{...a,language:'WebAssembly'},{...a,url:undefined},{...a,url:''}]
  for(let i=0;i<variants.length;i++)assert(index.set(String(i),variants[i]))
  assert(index.set('42',{...a}));assert.equal(index.get('0'),index.get('42'))
  const original={...a};a.url='changed';assert.deepEqual(index.get('0'),original)
  assert.throws(()=>{index.get('0').url='changed'})
  assert.equal(index.counts().metadata,variants.length)
  assert.throws(()=>index.set('bad',{...metadata(0),extra:true}))
  assert.throws(()=>index.set('symbol',{...metadata(0),[Symbol('extra')]:true}))
  assert.throws(()=>index.set('inherited',Object.assign(Object.create(metadata(0)),{a:1,b:2,c:3,d:4})))
})

test('index limits remain unavailable instead of silently dropping new identities',()=>{
  for(const index of [scriptMetadataIndex({entryLimit:1}),scriptMetadataIndex({metadataLimit:1})]) {
    assert(index.set('1',metadata(0)));assert.equal(index.set('2',metadata(1)),false)
    assert.equal(index.counts().overflow,true);assert(index.has('1'));assert(!index.has('2'))
    assert.equal(index.set('1',metadata(0)),false)
  }
})

test('nested insertion during metadata freezing cannot alias two dictionary slots',()=>{
  const url=new URL('./script-metadata-index.mjs',import.meta.url).href
  const source=`const freeze=Object.freeze;let index,entered=false;
const a={url:'a',isModule:true,hasSourceURL:false,language:'JavaScript'},b={...a,url:'b'};
Object.freeze=value=>{if(index&&!entered){entered=true;index.set('2',b)}return freeze(value)};
const {scriptMetadataIndex}=await import(${JSON.stringify(url)});
index=scriptMetadataIndex();index.set('1',a);Object.freeze=freeze;
console.log(JSON.stringify({a:index.get('1'),b:index.get('2'),counts:index.counts()}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error));const data=JSON.parse(r.stdout)
  assert.equal(data.a.url,'a');assert.equal(data.b.url,'b')
  assert.equal(data.counts.entries,2);assert.equal(data.counts.metadata,2);assert.equal(data.counts.events,2)
  assert.equal(data.counts.failures,0)
})

test('nested insertion rechecks capacity and updates to the same identity',()=>{
  const url=new URL('./script-metadata-index.mjs',import.meta.url).href
  for(const sameId of [false,true]) {
    const source=`const freeze=Object.freeze;let index,entered=false;
const a={url:'a',isModule:true,hasSourceURL:false,language:'JavaScript'},b={...a,url:'b'};
Object.freeze=value=>{if(index&&!entered){entered=true;index.set(${sameId?"'1'":"'2'"},b)}return freeze(value)};
const {scriptMetadataIndex}=await import(${JSON.stringify(url)});
index=scriptMetadataIndex({entryLimit:1});const result=index.set('1',a);Object.freeze=freeze;
console.log(JSON.stringify({result,a:index.get('1')??null,b:index.get('2')??null,counts:index.counts()}));`
    const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
    assert.equal(r.status,0,r.stderr||String(r.error));const data=JSON.parse(r.stdout)
    assert.equal(data.counts.entries,1);assert.equal(data.counts.events,2)
    assert.equal(data.result,sameId);assert.equal(data.counts.overflow,!sameId)
    if(sameId){assert.equal(data.a.url,'a');assert.equal(data.b,null)}
    else{assert.equal(data.a,null);assert.equal(data.b.url,'b')}
  }
})

test('application intrinsic changes and inherited toJSON cannot alter index keys',()=>{
  const url=new URL('./script-metadata-index.mjs',import.meta.url).href
  const source=`import {scriptMetadataIndex} from ${JSON.stringify(url)};
const original={freeze:Object.freeze,keys:Reflect.ownKeys,own:Object.hasOwn,descriptor:Object.getOwnPropertyDescriptor,
 get:Map.prototype.get,set:Map.prototype.set,floor:Math.floor,integer:Number.isSafeInteger};
let poisonCalls=0;const poison=()=>{poisonCalls++;throw Error('application intrinsic')};
const index=scriptMetadataIndex();
Object.freeze=Reflect.ownKeys=Object.hasOwn=Object.getOwnPropertyDescriptor=Map.prototype.get=Map.prototype.set=Math.floor=Number.isSafeInteger=poison;
Array.prototype.toJSON=Object.prototype.toJSON=poison;
let record,counts;
try {index.set('1',{url:'a|10|1:x',isModule:true,hasSourceURL:false,language:'JavaScript'});record=index.get('1');counts=index.counts()}
finally {Object.freeze=original.freeze;Reflect.ownKeys=original.keys;Object.hasOwn=original.own;Object.getOwnPropertyDescriptor=original.descriptor;
Map.prototype.get=original.get;Map.prototype.set=original.set;Math.floor=original.floor;Number.isSafeInteger=original.integer;
delete Array.prototype.toJSON;delete Object.prototype.toJSON}
console.log(JSON.stringify({record,counts,poisonCalls}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error));const data=JSON.parse(r.stdout)
  assert.equal(data.poisonCalls,0);assert.equal(data.record.url,'a|10|1:x');assert.equal(data.counts.failures,0)
})

test('invalid metadata records a failure stage without executing its accessor',()=>{
  const index=scriptMetadataIndex();let reads=0
  assert.throws(()=>index.set('1',{...metadata(0),get url(){reads++;throw Error('getter')}}))
  assert.equal(reads,0);assert.equal(index.counts().failures,1)
  assert.equal(index.counts().failureStages.validation,1);assert.equal(index.counts().entries,0)
  assert(index.set('1',metadata(0)));assert.equal(index.counts().failures,1)
})

test('compare exact script metadata indexes with isolated memory measurements',t=>{
  const url=new URL('./script-metadata-index.mjs',import.meta.url).href,rows=[]
  for(const mode of ['map','compact']) {
    const source=`import {scriptMetadataIndex} from ${JSON.stringify(url)};
globalThis.gc();const before=process.memoryUsage(),start=performance.now();
const index=${mode==='map'?'new Map()':'scriptMetadataIndex()'};
for(let i=0;i<250000;i++)index.set(String(i),{url:'file:///source'+i%32+'.mjs',isModule:!!(i&1),hasSourceURL:!!(i&2),language:i&4?'WebAssembly':'JavaScript'});
const insertMs=performance.now()-start;globalThis.gc();const after=process.memoryUsage();
const checkStart=performance.now();let matches=0;
for(let i=0;i<250000;i++){const v=index.get(String(i));if(v.url==='file:///source'+i%32+'.mjs'&&v.isModule===!!(i&1)&&v.hasSourceURL===!!(i&2)&&v.language===(i&4?'WebAssembly':'JavaScript'))matches++;}
console.log(JSON.stringify({mode:${JSON.stringify(mode)},entries:250000,matches,insertMs,lookupMs:performance.now()-checkStart,heapDelta:after.heapUsed-before.heapUsed,arrayBufferDelta:after.arrayBuffers-before.arrayBuffers,counts:index.counts?.()??null}));`
    const r=spawnSync(process.execPath,['--expose-gc','--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
    assert.equal(r.status,0,r.stderr||String(r.error));const row=JSON.parse(r.stdout)
    assert.equal(row.matches,row.entries);if(row.counts){assert.equal(row.counts.entries,row.entries);assert.equal(row.counts.overflow,false)}
    rows.push(row)
  }
  t.diagnostic(JSON.stringify({scriptMetadataIndexMeasurements:rows}))
})
