import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const directory=fs.mkdtempSync(path.join(os.tmpdir(),'quod-stat-constructor-'))
const file=path.join(directory,'input')
fs.writeFileSync(file,'fixture')
test.after(()=>fs.rmSync(directory,{recursive:true,force:true}))
function run(body) {
  const source=`import fs from 'node:fs';
import {createStatConstructorProbe} from ${JSON.stringify(new URL('./stat-constructor-probe.mjs',import.meta.url).href)};
const file=${JSON.stringify(file)};
const probe=createStatConstructorProbe();
try {${body}}finally{probe.close()}`
  const out=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:15000})
  assert.equal(out.status,0,out.stderr||String(out.error))
  return JSON.parse(out.stdout)
}
function checked(report) {
  assert.deepEqual(report.failures,[])
  assert.equal(report.replayEligible,false)
  assert.equal(report.completeOwnership,false)
}

for(const bigint of [false,true])test(`constructor observation precedes synchronous consumers, bigint=${bigint}`,t=>{
  const r=run(`const s=fs.statSync(file,{bigint:${bigint}});const value={file:s.isFile(),size:String(s.size)};
console.log(JSON.stringify({value,report:probe.report()}));`)
  checked(r.report)
  assert.deepEqual(r.value,{file:true,size:'7'})
  assert.deepEqual(r.report.rejections,[])
  assert.equal(r.report.rows.length,1)
  assert.equal(r.report.rows[0].kind,bigint?'BigIntStats':'Stats')
  assert.deepEqual(r.report.rows[0].fields,['mode','size'])
  assert.equal(r.report.rows[0].intact,true)
  assert.equal(r.report.pauses,2)
  t.diagnostic(JSON.stringify({synchronous:{bigint,...r}}))
})

test('constructor observation precedes the first registered promise consumer',t=>{
  const r=run(`let timestamp;const p=fs.promises.stat(file);
const first=p.then(s=>{timestamp=s.mtimeMs;return s});
const s=await first;const size=s.size;
console.log(JSON.stringify({timestampIsNumber:typeof timestamp==='number',size,report:probe.report()}));`)
  checked(r.report)
  assert.equal(r.timestampIsNumber,true);assert.equal(r.size,7)
  assert.deepEqual(r.report.rejections,[])
  assert.equal(r.report.rows.length,1)
  assert.deepEqual(r.report.rows[0].fields,['mtimeMs','size'])
  assert.equal(r.report.rows[0].intact,true)
  t.diagnostic(JSON.stringify({firstPromiseConsumer:r}))
})

test('constructor entry rejects a transient inherited setter that publishes metadata',t=>{
  const r=run(`let copied;
Object.defineProperty(fs.Stats.prototype,'birthtimeMs',{configurable:true,set(value){
 copied=this.mtimeMs;delete fs.Stats.prototype.birthtimeMs;
 Object.defineProperty(this,'birthtimeMs',{value,writable:true,enumerable:true,configurable:true});
}});
const s=fs.statSync(file);
console.log(JSON.stringify({copiedIsNumber:typeof copied==='number',size:s.size,
 prototypeRestored:!Object.hasOwn(fs.Stats.prototype,'birthtimeMs'),report:probe.report()}));`)
  checked(r.report)
  assert.equal(r.copiedIsNumber,true);assert.equal(r.size,7);assert.equal(r.prototypeRestored,true)
  assert.equal(r.report.rows.length,0)
  assert.deepEqual(r.report.rejections,[{kind:'Stats',reason:'inherited constructor field: birthtimeMs'}])
  t.diagnostic(JSON.stringify({constructorSetter:r}))
})

test('constructor observation preserves queued microtask order',()=>{
  const r=run(`let advanced=false;queueMicrotask(()=>{advanced=true});
const s=fs.statSync(file);const value=s.size;const before=advanced;
const report=probe.report();const afterReport=advanced;
await Promise.resolve();console.log(JSON.stringify({value,before,afterReport,after:advanced,report}));`)
  checked(r.report)
  assert.equal(r.value,7);assert.equal(r.before,false);assert.equal(r.afterReport,false);assert.equal(r.after,true)
})
