import test from 'node:test'
import assert from 'node:assert/strict'
import {inspect} from 'node:util'
import {observeModuleMetadata} from './worker-module-metadata-probe.mjs'

function graph() {
  return class Graph {
    nodes=new Map()
    ensureModule(id,url) {
      if(!this.nodes.has(id))this.nodes.set(id,{id,url,file:id,meta:undefined,
        evaluated:false,map:undefined,promise:undefined,exports:undefined,imports:new Set()})
      return this.nodes.get(id)
    }
    invalidateModule(node) {
      node.evaluated=false;node.meta=undefined;node.map=undefined
      node.promise=undefined;node.exports=undefined;node.imports.clear()
      return 17
    }
  }
}
test('observes stored metadata reads through the module graph and preserves node identity',()=>{
  const Graph=graph(),original=Graph.prototype.ensureModule,p=observeModuleMetadata(Graph),g=new Graph()
  const node=g.ensureModule('/one','/one'),raw={tmp:'/temporary',invalidate:true,code:'private-code'}
  node.meta=raw
  assert.equal(g.ensureModule('/one','/one'),node)
  assert.equal(node.meta.tmp,raw.tmp)
  assert.equal('invalidate' in node.meta,true)
  assert.equal(Object.getOwnPropertyDescriptor(node.meta,'invalidate').value,true)
  assert.deepEqual(Object.keys(node.meta),['tmp','invalidate','code'])
  const r=p.report()
  assert(r.events.some(e=>e.operation==='get'&&e.key==='tmp'))
  assert(r.events.some(e=>e.operation==='has'&&e.key==='invalidate'))
  assert(r.events.some(e=>e.operation==='descriptor'&&e.key==='invalidate'))
  assert(r.events.every(e=>e.count>0&&e.sites.length>0))
  assert(!JSON.stringify(r).includes('private-code'))
  assert.equal(r.replayEligible,false);assert.deepEqual(r.errors,[])
  p.close();assert.equal(node.meta,raw);assert.equal(Graph.prototype.ensureModule,original)
  assert(Object.hasOwn(Object.getOwnPropertyDescriptor(node,'meta'),'value'))
})
test('records cold and populated invalidations before preserving the original clearing behavior',()=>{
  const Graph=graph(),p=observeModuleMetadata(Graph),g=new Graph(),node=g.ensureModule('/one','/one')
  assert.equal(g.invalidateModule(node),17)
  node.meta={tmp:'/temporary',invalidate:false};node.evaluated=true
  node.promise=Promise.resolve();node.exports={x:1};node.map={};node.imports.add('/dep')
  assert.equal(g.invalidateModule(node),17)
  const [cold,warm]=p.report().invalidations
  assert.equal(cold.evaluated,false);assert.equal(cold.hasMeta,false);assert.equal(cold.imports,0)
  assert.equal(warm.evaluated,true);assert.equal(warm.hasMeta,true);assert.equal(warm.hasPromise,true)
  assert.equal(warm.hasExports,true);assert.equal(warm.hasMap,true);assert.equal(warm.imports,1)
  assert.equal(node.meta,undefined);assert.equal(node.imports.size,0);assert.equal(node.evaluated,false)
  p.close()
})
test('metadata aliases and mutations retain proxy identity while raw aliases remain an explicit gap',()=>{
  const Graph=graph(),p=observeModuleMetadata(Graph),g=new Graph()
  const a=g.ensureModule('/a','/a'),b=g.ensureModule('/b','/b'),raw={tmp:'/old',invalidate:true}
  a.meta=raw;b.meta=raw;assert.equal(a.meta,b.meta)
  a.meta.tmp='/new';assert.equal(raw.tmp,'/new');assert.equal(b.meta.tmp,'/new')
  delete a.meta.invalidate;assert.equal('invalidate' in b.meta,false)
  assert.equal(p.report().metadata[0].owners.length,2)
  assert.equal(p.report().completeOwnership,false)
  const count=p.report().events.length
  assert.match(inspect(a.meta),/new/)
  assert.equal(raw.tmp,'/new')
  assert.equal(p.report().events.length,count,'native inspection and raw aliases need separate treatment')
  p.close()
})
test('unsupported metadata descriptors stay explicit and original exceptions propagate',()=>{
  const Graph=graph(),original=Graph.prototype.ensureModule,failure=Error('original')
  Graph.prototype.ensureModule=function(...args) {
    if(args[0]==='throw')throw failure
    const node=Reflect.apply(original,this,args)
    Object.defineProperty(node,'meta',{value:undefined,configurable:false,writable:false})
    return node
  }
  const p=observeModuleMetadata(Graph),g=new Graph()
  assert(g.ensureModule('/fixed','/fixed'))
  assert(p.report().errors.includes('unsupported module metadata descriptor'))
  assert.throws(()=>g.ensureModule('throw','throw'),e=>e===failure)
  assert.equal(p.report().replayEligible,false)
  p.close()
})
