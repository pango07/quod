import {test} from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {spawnSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'

const bootstrap=fileURLToPath(new URL('./startup-stat-probe.cjs',import.meta.url))
const marker='--require="'+bootstrap.replace(/\\/g,'\\\\').replace(/"/g,'\\"')+'"'
function run(kind) {
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'quod-startup-'))
  try {
    const preload=path.join(dir,'user preload.cjs'),entry=path.join(dir,'main.cjs')
    const bodies={
      method:"require('node:fs').Stats.prototype.isFile=function(){return this.mtimeMs>0}",
      poisoned:"require('node:fs').Stats.prototype.isFile=function(){return this.mtimeMs>0};Array.prototype.every=function(){return true}",
      setter:"Object.defineProperty(require('node:fs').Stats.prototype,'mtimeMs',{configurable:true,set(value){globalThis.exposed=this;Object.defineProperty(this,'mtimeMs',{value,enumerable:true})}})",
      binding:"const b=process.binding('fs'),stat=b.stat;b.stat=function(...args){return Reflect.apply(stat,this,args)}",
      promise:"const fs=require('node:fs'),stat=fs.promises.stat;fs.promises.stat=async function(...args){const s=await Reflect.apply(stat,this,args);globalThis.observedMtime=s.mtimeMs;return s}",
      inspector:"const s=new (require('node:inspector').Session)();s.connect();globalThis.foreignSession=s",
      import:"globalThis.userImportRan=true",
      prior:"require('node:fs').Stats.prototype.isFile=function(){return this.mtimeMs>0}",
    }
    fs.writeFileSync(preload,`globalThis.observedStartupOptions=process.env.NODE_OPTIONS;${bodies[kind]??''};`)
    let original=kind==='none'?undefined:kind==='empty'?'':kind==='unsupported'?'--trace-warnings':
      `${kind==='import'?'--import':'--require'}="${preload}"`
    const env={...process.env};delete env.NODE_OPTIONS
    if(kind==='late') {original=undefined}
    else env.NODE_OPTIONS=kind==='prior'?`${original} ${marker}`:marker+(original===undefined?'':' '+original)
    fs.writeFileSync(entry,`const fs=require('node:fs'),early=require(${JSON.stringify(bootstrap)});
const before=early.snapshot(),native=early.observerRegistry.nativeStatSnapshot();
const options=process.env.NODE_OPTIONS,visibleInPreload=globalThis.observedStartupOptions;
const foreign=early.observerRegistry.inspectorAccess();
const stats=fs.statSync(__filename);globalThis.foreignSession?.disconnect();early.observerRegistry.close();
console.log(JSON.stringify({before,native,foreign,options,hasOptions:Object.hasOwn(process.env,'NODE_OPTIONS'),
 visibleInPreload,importRan:globalThis.userImportRan===true,escaped:globalThis.exposed===stats}));`)
    const r=spawnSync(process.execPath,[entry],{env,encoding:'utf8',timeout:15000})
    assert.equal(r.status,0,r.stderr||String(r.error));return {result:JSON.parse(r.stdout),original}
  } finally {fs.rmSync(dir,{recursive:true,force:true})}
}

for(const kind of ['none','empty','report','import','method','poisoned','setter','binding','promise','inspector','prior','late','unsupported'])
test('first-preload origin diagnostic: '+kind,()=>{
  const {result:r,original}=run(kind),s=r.before
  assert.equal(s.replayEligible,false)
  assert.equal(s.startup.available,!['prior','late','unsupported'].includes(kind),JSON.stringify(r))
  if(kind!=='prior') {
    assert.equal(r.options,original,'restore the exact original NODE_OPTIONS before user code')
    assert.equal(r.hasOptions,original!==undefined)
    if(!['none','empty','late','unsupported'].includes(kind))assert.equal(r.visibleInPreload,original)
  }
  assert.equal(s.nativeBindingUnchanged,kind!=='binding')
  assert.equal(s.publicAsyncUnchanged,kind!=='promise')
  assert.equal(s.publicSyncUnchanged,true)
  assert.equal(s.prototypeUnchanged,!['method','poisoned','setter'].includes(kind))
  assert.equal(r.native.unchanged,kind!=='binding')
  if(kind==='setter')assert.equal(r.escaped,true)
  if(kind==='import')assert.equal(r.importRan,true,'later ESM imports must still execute after restoring NODE_OPTIONS')
  if(kind==='inspector')assert.deepEqual(r.foreign.entries.flatMap(e=>e.entries),['Connection'])
  if(kind==='prior')assert.equal(s.prototypeUnchanged,true,'a late snapshot is already modified; startup order must reject it')
})
