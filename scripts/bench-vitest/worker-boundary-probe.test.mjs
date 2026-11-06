import test from 'node:test'
import assert from 'node:assert/strict'
import vm from 'node:vm'
import {createHash} from 'node:crypto'
import {observeModuleEvaluation} from './worker-module-probe.mjs'
import {observeModuleMetadata} from './worker-module-metadata-probe.mjs'
import {inspectPromiseObservers} from './promise-observers-probe.mjs'

test('code observation reads only its own metadata accessor and rejects a replacement without invoking it',()=>{
  class Graph {
    ensureModule(){return {id:'/module',url:'/module',file:'/module',meta:undefined}}
    invalidateModule(){}
  }
  class Evaluator {runInlinedModule(context,code){return code}runExternalModule(){}}
  const metadata=observeModuleMetadata(Graph),evaluation=observeModuleEvaluation(Evaluator,{metadataForNode:metadata.metadataForNode})
  try {
    const node=new Graph().ensureModule(),raw={tmp:'/private-metadata',invalidate:true}
    node.meta=raw
    const evaluator=new Evaluator(),code='private-evaluated-source'
    assert.equal(evaluator.runInlinedModule({},code,node),code)
    assert.deepEqual(evaluation.report().errors,[])
    assert.equal(evaluation.report().events[0].code.sha256,createHash('sha256').update(code,'utf16le').digest('hex'))
    let calls=0
    Object.defineProperty(node,'meta',{get(){calls++;return raw},configurable:true})
    assert.equal(evaluator.runInlinedModule({},code,node),code)
    assert.equal(calls,0)
    assert.deepEqual(evaluation.report().errors,['module metadata ownership unavailable'])
    assert(!JSON.stringify(evaluation.report()).includes(code))
    assert(!JSON.stringify(evaluation.report()).includes('/private-metadata'))
  }finally{evaluation.close();metadata.close()}
})

test('one Inspector session observes native entries and exact JS/Wasm bytes without extra connections',()=>{
  const p=inspectPromiseObservers({proxyAccess:true,hookAccess:true,compiledScripts:true})
  const code='41 + 1',bytes=Buffer.from([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,7,5,1,1,102,0,0,10,6,1,4,0,65,41,11])
  try {
    assert.equal(vm.runInThisContext(code,{filename:'quod-combined-code-fixture'}),42)
    assert.equal(new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports.f(),41)
    const report=p.compiledScriptReport()
    assert.equal(report.enabled,true);assert.equal(report.overflow,false);assert.deepEqual(report.errors,[])
    assert(report.scripts.some(s=>s.code.encoding==='utf16le'&&s.code.sha256===createHash('sha256').update(code,'utf16le').digest('hex')))
    assert(report.scripts.some(s=>s.code.encoding==='bytes'&&s.code.sha256===createHash('sha256').update(bytes).digest('hex')))
    assert.deepEqual(p.inspectorAccess().entries,[])
    assert.equal(p.proxyAccess().available,true);assert.equal(p.hookAccess().available,true)
    assert.equal(report.completeCompilationCoverage,false);assert.equal(report.replayEligible,false)
  }finally{p.close()}
})
