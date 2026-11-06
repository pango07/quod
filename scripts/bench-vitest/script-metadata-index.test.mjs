import nodeTest from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {scriptMetadataIndex as createIndex} from './script-metadata-index.mjs'
const metadata=i=>({url:'file:///source'+i+'.mjs',isModule:!!(i&1),hasSourceURL:!!(i&2),language:i&4?'WebAssembly':'JavaScript'})

for(const storage of ['dense','ranges']) {
const test=(name,body)=>nodeTest(storage+': '+name,body)
const scriptMetadataIndex=(options={})=>createIndex({...options,storage})

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

test('bounded lookup cache preserves overwrites, misses and immutable metadata',()=>{
  const index=scriptMetadataIndex({lookupLimit:2}),expected=new Map()
  for(const limit of [-1,.5,Infinity,65537])assert.throws(()=>scriptMetadataIndex({lookupLimit:limit}))
  for(const id of ['1','01','9007199254740991','foreign\ud800']) {
    assert.equal(index.get(id),undefined)
    assert.equal(index.counts().lookupEntries,0,'missing IDs must not fill the cache')
  }
  for(const [i,id] of ['1','01','9007199254740991','foreign\ud800'].entries()) {
    const value=metadata(i);assert(index.set(id,value));expected.set(id,{...value})
    assert.deepEqual(index.get(id),value)
  }
  assert.equal(index.counts().lookupEntries,2)
  for(let i=0;i<100;i++)for(const [id] of expected) {
    const value=metadata(i);assert(index.set(id,value));expected.set(id,{...value})
    value.url='caller mutation'
    for(const [key,want] of expected)assert.deepEqual(index.get(key),want)
  }
  assert.throws(()=>{index.get('1').url='cached mutation'})
  assert.equal(index.counts().lookupEntries,2)
  assert.equal(index.counts().entries,4)
  assert.equal(index.counts().overflow,false)
})

test('failed metadata replacement cannot change a cached lookup',()=>{
  const index=scriptMetadataIndex({lookupLimit:2,metadataLimit:1})
  assert(index.set('1',metadata(0)));const before=index.get('1')
  assert.throws(()=>index.set('1',{...metadata(0),extra:true}))
  assert.equal(index.get('1'),before)
  assert.equal(index.set('1',metadata(1)),false)
  assert.equal(index.get('1'),before)
  assert.equal(index.counts().metadata,1)
  assert.equal(index.counts().lookupEntries,1)
  assert.equal(index.counts().overflow,true)
})

test('nested validation lookups and writes cannot leave stale cached metadata',()=>{
  const url=new URL('./script-metadata-index.mjs',import.meta.url).href
  const source=`const freeze=Object.freeze;let index,armed=false,seen;
const a={url:'a',isModule:true,hasSourceURL:false,language:'JavaScript'},b={...a,url:'b'},c={...a,url:'c'};
Object.freeze=value=>{if(armed){armed=false;seen=index.get('1').url;index.set('1',b)}return freeze(value)};
const {scriptMetadataIndex}=await import(${JSON.stringify(url)});
index=scriptMetadataIndex({storage:${JSON.stringify(storage)},lookupLimit:2});index.set('1',a);
armed=true;index.set('1',c);Object.freeze=freeze;
console.log(JSON.stringify({seen,record:index.get('1'),counts:index.counts()}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error));const data=JSON.parse(r.stdout)
  assert.equal(data.seen,'a');assert.deepEqual(data.record,{...metadata(0),url:'c',isModule:true})
  assert.equal(data.counts.entries,1);assert.equal(data.counts.events,3)
  assert.equal(data.counts.lookupEntries,1);assert.equal(data.counts.failures,0)
})

test('nested insertion during metadata freezing cannot alias two dictionary slots',()=>{
  const url=new URL('./script-metadata-index.mjs',import.meta.url).href
  const source=`const freeze=Object.freeze;let index,entered=false;
const a={url:'a',isModule:true,hasSourceURL:false,language:'JavaScript'},b={...a,url:'b'};
Object.freeze=value=>{if(index&&!entered){entered=true;index.set('2',b)}return freeze(value)};
const {scriptMetadataIndex}=await import(${JSON.stringify(url)});
index=scriptMetadataIndex({storage:${JSON.stringify(storage)}});index.set('1',a);Object.freeze=freeze;
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
index=scriptMetadataIndex({storage:${JSON.stringify(storage)},entryLimit:1});const result=index.set('1',a);Object.freeze=freeze;
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
const index=scriptMetadataIndex({storage:${JSON.stringify(storage)}});
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

if(storage==='dense')test('compare exact script metadata indexes with isolated memory measurements',t=>{
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
}

nodeTest('range capacity failure leaves the cached middle of an interval unchanged',()=>{
  const index=createIndex({storage:'ranges',rangeLimit:1,lookupLimit:2})
  for(const id of ['1','2','3'])assert(index.set(id,metadata(0)))
  const before=index.get('2')
  assert.equal(index.set('2',metadata(1)),false,'a middle replacement would need three intervals')
  assert.equal(index.get('2'),before)
  assert.deepEqual(index.get('1'),metadata(0));assert.deepEqual(index.get('3'),metadata(0))
  assert.equal(index.counts().ranges,1);assert.equal(index.counts().metadata,1)
  assert.equal(index.counts().overflow,true)
})

nodeTest('compare repeated exact metadata lookups with bounded cache and unchanged records',t=>{
  const url=new URL('./script-metadata-index.mjs',import.meta.url).href,rows=[]
  for(const lookupLimit of [0,2048,2048,0]) {
    const source=`import {scriptMetadataIndex} from ${JSON.stringify(url)};
const index=scriptMetadataIndex({storage:'ranges',lookupLimit:${lookupLimit}}),expected=new Map();
for(let i=0;i<2048;i++){
 const id=String(i*2),value={url:'file:///source'+i%64+'.mjs',isModule:!!(i&1),hasSourceURL:!!(i&2),language:i&4?'WebAssembly':'JavaScript'};
 if(!index.set(id,value))throw Error('insertion failed');if(i<128)expected.set(id,value);
}
const hot=[...expected.keys()];for(const id of hot)index.get(id);
const dynamic={url:'',isModule:false,hasSourceURL:false,language:'JavaScript'};
globalThis.gc();const memory=process.memoryUsage(),start=performance.now();let matches=0;
for(let i=0;i<2000000;i++){
 const id=hot[i%128],want=expected.get(id),got=index.get(id);
 if(got.url!==want.url||got.isModule!==want.isModule||got.hasSourceURL!==want.hasSourceURL||got.language!==want.language)throw Error('metadata mismatch');
 matches++;if(i%1024===0&&!index.set(String(100000+i/1024),dynamic))throw Error('dynamic insertion failed');
}
const elapsedMs=performance.now()-start;globalThis.gc();const after=process.memoryUsage();
console.log(JSON.stringify({lookupLimit:${lookupLimit},matches,elapsedMs,heapDelta:after.heapUsed-memory.heapUsed,counts:index.counts()}));`
    const r=spawnSync(process.execPath,['--expose-gc','--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
    assert.equal(r.status,0,r.stderr||String(r.error));const row=JSON.parse(r.stdout)
    assert.equal(row.matches,2000000);assert.equal(row.counts.entries,2048+1954)
    assert.equal(row.counts.events,row.counts.entries);assert.equal(row.counts.metadata,65)
    assert.equal(row.counts.lookupEntries,lookupLimit?128:0)
    assert.equal(row.counts.overflow,false);assert.equal(row.counts.failures,0)
    rows.push(row)
  }
  t.diagnostic(JSON.stringify({hotScriptMetadataMeasurements:rows,scope:'isolated repeated lookups; not workload or CI savings'}))
})
