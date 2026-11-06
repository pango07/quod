import test from 'node:test'
import assert from 'node:assert/strict'
import {inspect,types} from 'node:util'
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

test('accessor metadata mode observes earlier object aliases without substituting their identity',()=>{
  const Graph=graph(),p=observeModuleMetadata(Graph,{observation:'accessor'}),g=new Graph()
  const raw={tmp:'/private-original',invalidate:false,code:'private-code'},alias=raw,copied=raw.tmp
  class Holder {constructor(value){this.value=value}read(){return this.value.tmp}}
  const holder=new Holder(raw),a=g.ensureModule('/a','/a'),b=g.ensureModule('/b','/b')
  assert.equal(p.observeMetadata(raw,'rpc:fetch'),raw)
  assert.equal(alias.tmp,'/private-original');assert.equal(holder.read(),'/private-original')
  assert.equal(copied,'/private-original')
  a.meta=raw;b.meta=raw
  assert.equal(a.meta,raw);assert.equal(b.meta,raw);assert.equal(types.isProxy(raw),false)
  raw.tmp='/private-next';assert.equal(a.meta.tmp,'/private-next')
  const r=p.report()
  assert.equal(r.observation,'accessor');assert.deepEqual(r.gaps,[])
  assert.equal(r.events.reduce((n,e)=>n+e.count,0),4)
  assert(r.events.filter(e=>!e.origins.includes('stored')).length>0,'read-time origins precede later storage')
  assert(r.events.some(e=>e.origins.includes('stored')))
  assert(!JSON.stringify(r).includes('/private-'));assert(!JSON.stringify(r).includes('private-code'))
  assert.equal(r.completeOwnership,false);assert.equal(r.replayEligible,false)
  assert.deepEqual(p.metadataForNode(a).value,{tmp:'/private-next',invalidate:false,code:'private-code'})
  p.close();assert.equal(a.meta,raw)
  assert.deepEqual(Object.getOwnPropertyDescriptor(raw,'tmp'),{value:'/private-next',writable:true,enumerable:true,configurable:true})
})

test('accessor metadata mode preserves missing properties and refuses ownership after deletion or replacement',()=>{
  const Graph=graph(),p=observeModuleMetadata(Graph,{observation:'accessor'}),node=new Graph().ensureModule('/a','/a')
  const raw={tmp:'/tmp/original'};node.meta=raw
  assert.equal('invalidate' in raw,false);assert.deepEqual(Object.keys(raw),['tmp'])
  const installed=Object.getOwnPropertyDescriptor(raw,'tmp')
  delete raw.tmp
  assert.equal(p.metadataForNode(node).available,false)
  Object.defineProperty(raw,'tmp',installed)
  assert.equal(p.metadataForNode(node).available,true,'restoration alone cannot exclude an intermediate deletion')
  let getters=0
  Object.defineProperty(raw,'tmp',{get(){getters++;return 'changed'},configurable:true})
  assert.equal(p.metadataForNode(node).available,false);assert.equal(getters,0)
  raw.invalidate=true
  const report=p.report()
  assert(report.gaps.includes('metadata scalar descriptor unavailable: tmp'))
  assert(report.gaps.includes('unobserved metadata field added: invalidate'))
  p.close();assert.equal(getters,0,'cleanup must not invoke or overwrite the replacement getter')
})

test('accessor metadata rejects pre-existing accessors and fixed properties without evaluating them',()=>{
  const Graph=graph(),p=observeModuleMetadata(Graph,{observation:'accessor'})
  let calls=0
  const raw={get tmp(){calls++;return '/private'}}
  Object.defineProperty(raw,'invalidate',{value:true,configurable:false})
  assert.equal(p.observeMetadata(raw,'rpc:fetch'),raw)
  const report=p.report()
  assert.equal(calls,0);assert(report.gaps.length>0)
  assert(report.metadata[0].scalars.every(s=>!s.installed));p.close();assert.equal(calls,0)
})
