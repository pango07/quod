// Diagnostic only. Observe the metadata stored in Vite's module graph and
// the state cleared by invalidation. Original aliases/native reads can bypass
// these proxies, and accessor installation is observable: never replay authority.
import {types,getCallSites} from 'node:util'
import {fingerprint} from './worker-input-probe.mjs'
import {observeScalarProperty} from './scalar-property-probe.mjs'

const descriptor=Object.getOwnPropertyDescriptor,define=Object.defineProperty,apply=Reflect.apply
const setSize=descriptor(Set.prototype,'size').get
const watched=new Set(['tmp','invalidate'])
export function observeModuleMetadata(EvaluatedModules,{sourceForScript=null,observation='proxy'}={}) {
  if(!['proxy','accessor'].includes(observation))throw Error('unknown metadata observation mode')
  const nodes=new Map(),metadata=new Map(),proxyToRaw=new WeakMap(),events=new Map()
  const errors=[],gaps=new Set(),invalidations=[],restores=[]
  let overflow=false,assignments=0
  function record(m,operation,key,sites,source) {
    if(sourceForScript&&!source?.available)gaps.add('metadata reader source unavailable')
    const origins=[...m.origins],id=JSON.stringify([m.id,operation,key,sites,origins])
    if(events.has(id))events.get(id).count++
    else if(events.size<10000)events.set(id,{metadata:m.id,operation,key,sites,source,origins,count:1})
    else overflow=true
  }
  function ownData(value,key) {
    if(!value||typeof value!=='object'||types.isProxy(value))throw Error('unsupported module node')
    const d=descriptor(value,key)
    if(!d)return undefined
    if(!Object.hasOwn(d,'value'))throw Error('module node accessor')
    return d.value
  }
  function note(m,operation,key) {
    if(!watched.has(key)&&operation!=='keys')return
    try {
      const sites=getCallSites(8,{sourceMap:false}).slice(2).map(s=>({scriptId:s.scriptId,script:s.scriptName,
        function:s.functionName,line:s.lineNumber,column:s.columnNumber}))
      const source=sourceForScript?sourceForScript(sites[0]?.scriptId):null
      record(m,operation,key,sites,source)
    }catch{gaps.add('metadata call sites unavailable')}
  }
  function wrap(value,owner,origin='stored') {
    if(value===undefined||value===null)return value
    if(proxyToRaw.has(value))value=proxyToRaw.get(value)
    let m=metadata.get(value)
    if(m){m.owners.add(owner);m.origins.add(origin);return m.proxy}
    if(typeof value!=='object'||types.isProxy(value)){gaps.add('unsupported metadata value');return value}
    const proto=Object.getPrototypeOf(value)
    if(proto!==Object.prototype&&proto!==null){gaps.add('non-plain metadata');return value}
    if(metadata.size>=10000){overflow=true;return value}
    m={id:metadata.size,owners:new Set([owner]),origins:new Set([origin])}
    if(observation==='accessor') {
      // Keep the original object, including references retained before this
      // observer. Only existing scalar fields are changed: adding an absent
      // property would change `in`, ownKeys, and loader control flow.
      m.proxy=value;m.raw=value;m.scalars=new Map();metadata.set(value,m)
      for(const key of watched)if(descriptor(value,key)) {
        const scalar=observeScalarProperty(value,key,{sourceForScript,
          onOperation:e=>record(m,e.operation,key,e.sites,e.source)})
        m.scalars.set(key,scalar);restores.push(()=>scalar.close())
      }
      return value
    }
    const proxy=new Proxy(value,{
      get(target,key,receiver){note(m,'get',key);return Reflect.get(target,key,receiver)},
      has(target,key){note(m,'has',key);return Reflect.has(target,key)},
      ownKeys(target){note(m,'keys',null);return Reflect.ownKeys(target)},
      getOwnPropertyDescriptor(target,key){note(m,'descriptor',key);return Reflect.getOwnPropertyDescriptor(target,key)},
      set(target,key,value,receiver){note(m,'set',key);return Reflect.set(target,key,value,receiver)},
      defineProperty(target,key,d){note(m,'define',key);return Reflect.defineProperty(target,key,d)},
      deleteProperty(target,key){note(m,'delete',key);return Reflect.deleteProperty(target,key)}
    })
    m.proxy=proxy;metadata.set(value,m);proxyToRaw.set(proxy,value)
    return proxy
  }
  function observeNode(node) {
    if(nodes.has(node))return
    if(nodes.size>=10000){overflow=true;return}
    const id=fingerprint({id:ownData(node,'id'),url:ownData(node,'url'),file:ownData(node,'file')})
    if(!id.available)throw Error('unknown module identity')
    const d=descriptor(node,'meta')
    if(!d||!Object.hasOwn(d,'value')||!d.configurable||!d.writable)throw Error('unsupported module metadata descriptor')
    let raw=d.value,value=wrap(raw,id.hash)
    const get=()=>value,set=next=>{
      assignments++
      raw=proxyToRaw.get(next)??next
      value=wrap(raw,id.hash)
    }
    define(node,'meta',{get,set,enumerable:d.enumerable,configurable:true})
    nodes.set(node,{id,raw:()=>raw,get,set})
    restores.push(()=>{
      const now=descriptor(node,'meta')
      if(now?.get===get&&now?.set===set&&now.configurable)define(node,'meta',{...d,value:raw})
    })
  }
  function invalidation(node) {
    if(invalidations.length>=10000){overflow=true;return}
    const tracked=nodes.get(node)
    const raw=tracked?tracked.raw():ownData(node,'meta')
    const imports=ownData(node,'imports')
    if(!imports||types.isProxy(imports)||!types.isSet(imports))throw Error('unknown module imports')
    const evaluated=ownData(node,'evaluated')
    if(typeof evaluated!=='boolean')throw Error('unknown module evaluation state')
    invalidations.push({identity:tracked?.id??fingerprint({id:ownData(node,'id'),url:ownData(node,'url')}),
      evaluated,hasMeta:raw!==undefined,hasMap:ownData(node,'map')!==undefined,
      hasPromise:ownData(node,'promise')!==undefined,hasExports:ownData(node,'exports')!==undefined,
      imports:apply(setSize,imports,[])})
  }
  for(const [method,observeBefore] of [['ensureModule',false],['invalidateModule',true]]) {
    const p=EvaluatedModules?.prototype,d=p&&descriptor(p,method),original=d?.value
    if(typeof original!=='function'){errors.push('missing module graph method: '+method);continue}
    const wrapper=function(...args) {
      if(observeBefore)try{invalidation(args[0])}catch(e){errors.push(e.message)}
      const result=apply(original,this,args)
      if(!observeBefore)try{observeNode(result)}catch(e){errors.push(e.message)}
      return result
    }
    define(p,method,{...d,value:wrapper})
    restores.push(()=>{if(descriptor(p,method)?.value===wrapper)define(p,method,d)})
  }
  function scalarStatus(m) {
    if(!m.scalars)return []
    for(const key of watched)if(!m.scalars.has(key)&&descriptor(m.raw,key))gaps.add('unobserved metadata field added: '+key)
    return [...m.scalars].map(([key,p])=>{
      const r=p.report()
      if(!r.installed||!r.intact)gaps.add('metadata scalar descriptor unavailable: '+key)
      for(const gap of r.gaps)gaps.add(key+': '+gap)
      if(r.overflow)overflow=true
      return {key,installed:r.installed,intact:r.intact}
    })
  }
  function snapshot(raw) {
    const m=metadata.get(raw)
    if(!m?.scalars)return raw
    const copy=Object.create(Object.getPrototypeOf(raw))
    for(const key of Reflect.ownKeys(raw)) {
      const d=descriptor(raw,key),scalar=m.scalars.get(key)
      if(scalar) {
        const current=scalar.currentValue()
        if(!current.available)throw Error('metadata scalar ownership unavailable')
        define(copy,key,{value:current.value,enumerable:d.enumerable,configurable:true,writable:true})
      }else {
        if(!Object.hasOwn(d,'value'))throw Error('unowned metadata accessor')
        define(copy,key,d)
      }
    }
    // Deletion must not turn an owned field into an apparently valid snapshot.
    for(const scalar of m.scalars.values())if(!scalar.currentValue().available)throw Error('metadata scalar ownership unavailable')
    return copy
  }
  return {report:()=>{
    const records=[...metadata.values()].map(m=>({id:m.id,owners:[...m.owners],origins:[...m.origins],scalars:scalarStatus(m)}))
    return {replayEligible:false,completeOwnership:false,observation,overflow,assignments,
    nodes:nodes.size,metadata:records,
    events:[...events.values()],invalidations,errors,gaps:[...gaps],
    limitations:observation==='accessor'?[
      'Existing object aliases are observed, but earlier primitive copies are not',
      'Descriptors, native inspection, intermediate deletion and replacement remain observable gaps',
      'Only existing tmp and invalidate scalar properties are instrumented; this is not complete ownership']:
      ['Observation begins at explicitly wrapped metadata; earlier raw aliases remain outside it',
      'Original aliases, nested values and native target reads can bypass these proxies',
      'Installing metadata accessors and proxies is observable']}},
    observeMetadata(value,origin) {
      try{return wrap(value,origin,origin)}catch(e){errors.push(e.message);return value}
    },
    metadataForNode(node) {
      const tracked=nodes.get(node),d=tracked&&descriptor(node,'meta')
      if(!tracked||d?.get!==tracked.get||d?.set!==tracked.set)return {available:false}
      try{return {available:true,value:snapshot(tracked.raw())}}
      catch{return {available:false}}
    },
    close(){for(const restore of restores.reverse())restore()}}
}
