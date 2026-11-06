import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import {createHash} from 'node:crypto'
import {inspectPromiseObservers} from './promise-observers-probe.mjs'
import {observeModuleMetadata} from './worker-module-metadata-probe.mjs'

const sha=s=>createHash('sha256').update(s,'utf16le').digest('hex')
function setup() {
  class Graph {ensureModule(){} invalidateModule(){}}
  const registry=inspectPromiseObservers(),metadata=observeModuleMetadata(Graph,{sourceForScript:registry.sourceForScript})
  const value=metadata.observeMetadata({tmp:'private-metadata-value'},'fixture')
  return {registry,metadata,value,close(){metadata.close();registry.close()}}
}

test('native call-site identity resolves the actual ESM source and becomes unavailable after close',()=>{
  const p=setup()
  try {
    assert.equal(p.value.tmp,'private-metadata-value')
    const event=p.metadata.report().events[0],source=event.source
    assert(source.available,JSON.stringify(source));assert.equal(event.sites[0].scriptId,source.scriptId)
    assert.equal(source.script.url,import.meta.url);assert.equal(source.script.isModule,true)
    assert.equal(source.script.hasSourceURL,false)
    assert.deepEqual(source.code,{encoding:'utf16le',bytes:fs.readFileSync(new URL(import.meta.url),'utf8').length*2,
      sha256:sha(fs.readFileSync(new URL(import.meta.url),'utf8'))})
    assert(!JSON.stringify(p.metadata.report()).includes('private-metadata-value'))
    assert.equal(p.registry.sourceForScript(source.scriptId),source)
    assert.equal(p.registry.sourceForScript('missing-script').available,false)
    p.registry.close()
    assert.equal(p.registry.sourceForScript(source.scriptId).available,false)
  }finally{p.close()}
})

test('identical filenames and forged function text cannot merge distinct executable sources',()=>{
  const p=setup(),filename='file:///quod-fixture/trusted-loader.js'
  const a='(function read(value){ return value.tmp })',b='(function read(value){ /* different code */ return value.tmp })'
  try {
    const first=vm.runInThisContext(a,{filename}),second=vm.runInThisContext(b,{filename})
    second.toString=()=>first.toString()
    assert.equal(first(p.value),second(p.value));assert.equal(first.toString(),second.toString())
    const sources=p.metadata.report().events.map(e=>e.source)
    assert.equal(sources.length,2);assert(sources.every(s=>s.available&&s.script.url===filename))
    assert.notEqual(sources[0].scriptId,sources[1].scriptId)
    assert.deepEqual(sources.map(s=>s.code.sha256),[sha(a),sha(b)])
    assert(sources.every(s=>!s.script.isModule&&!s.script.hasSourceURL))
  }finally{p.close()}
})

test('sourceURL impersonation remains explicit without replacing the native resource name',()=>{
  const p=setup(),url='file:///quod-fixture/trusted-loader.js'
  const code='(value=>value.tmp)\n//# sourceURL='+url
  try {
    const read=vm.runInThisContext(code,{filename:'different-actual-filename'})
    assert.equal(read(p.value),'private-metadata-value')
    const event=p.metadata.report().events[0]
    assert.equal(event.sites[0].script,'different-actual-filename')
    assert.equal(event.source.available,true);assert.equal(event.source.script.hasSourceURL,true)
    assert.equal(event.source.code.sha256,sha(code));assert.equal(event.source.replayEligible,false)
  }finally{p.close()}
})

test('reader source digests preserve distinct UTF-16 code units',()=>{
  const p=setup(),a='/*\ud800*/(value=>value.tmp)',b='/*\ud801*/(value=>value.tmp)'
  try {
    assert.deepEqual(Buffer.from(a),Buffer.from(b),'UTF-8 alone would lose this distinction')
    for(const source of [a,b])assert.equal(vm.runInThisContext(source,{filename:'same-filename'})(p.value),'private-metadata-value')
    assert.deepEqual(p.metadata.report().events.map(e=>e.source.code.sha256),[sha(a),sha(b)])
    assert.notEqual(sha(a),sha(b))
  }finally{p.close()}
})

test('unavailable source identity remains a gap while the metadata read still works',()=>{
  class Graph {ensureModule(){} invalidateModule(){}}
  const p=observeModuleMetadata(Graph,{sourceForScript:()=>({available:false,reason:'fixture missing source'})})
  try {
    const value=p.observeMetadata({tmp:'value'},'fixture')
    assert.equal(value.tmp,'value')
    assert.equal(p.report().events[0].source.available,false)
    assert(p.report().gaps.includes('metadata reader source unavailable'))
    assert.equal(p.report().replayEligible,false)
  }finally{p.close()}
})
