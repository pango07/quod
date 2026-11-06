// Diagnostic only: inventory the messages entering isolated Vitest workers.
// Hash values rather than publishing configuration, environment or source text.
// This is not a complete dependency certificate and never permits replay.
import {createHash} from 'node:crypto'
import {types} from 'node:util'

const descriptors=Object.getOwnPropertyDescriptors,proto=Object.getPrototypeOf
const own=Object.hasOwn,apply=Reflect.apply
const methods=new Set(['fetch','fetchWarmModules','prewarmModuleGraph','resolve',
  'snapshotSaved','resolveSnapshotPath','transform','onQueued','onCollected',
  'onAfterSuiteRun','onTestBenchmark','readBenchmarkResult','writeBenchmarkResult',
  'onTaskArtifactRecord','onTaskUpdate','onUserConsoleLog','onUnhandledError',
  'onAsyncLeaks','onCancel','getCountOfFailedTests','ensureModuleGraphEntry'])
const hash=text=>createHash('sha256').update(text).digest('hex')

// Deliberately limited. Do not invoke getters, Proxy traps, toJSON, custom
// iterators or constructors just to inspect a packet. Unknown shapes are
// reported, rather than silently omitted from the digest.
export function fingerprint(value) {
  const seen=new Map()
  function visit(v) {
    if(v===null)return ['null']
    if(typeof v==='undefined')return ['undefined']
    if(typeof v==='string'||typeof v==='boolean')return [typeof v,v]
    if(typeof v==='number')return ['number',Object.is(v,-0)?'-0':String(v)]
    if(typeof v==='bigint')return ['bigint',v.toString()]
    if(typeof v!=='object')throw Error('unsupported value type')
    if(types.isProxy(v))throw Error('proxy value')
    if(seen.has(v))return ['ref',seen.get(v)]
    const id=seen.size;seen.set(v,id)
    const p=proto(v),ds=descriptors(v)
    if(Reflect.ownKeys(ds).some(k=>typeof k!=='string'))throw Error('symbol property')
    if(Object.values(ds).some(d=>!own(d,'value')))throw Error('accessor property')
    let kind=Array.isArray(v)&&p===Array.prototype?'array':p===null?'null-object':p===Object.prototype?'object':null
    if(!kind)throw Error('unsupported prototype')
    return [kind,id,Object.entries(ds).map(([k,d])=>[k,visit(d.value)])]
  }
  try{return {available:true,hash:hash(JSON.stringify(visit(value)))}}
  catch(e){return {available:false,reason:e.message}}
}

function values(object) {
  if(!object||typeof object!=='object'||types.isProxy(object))throw Error('unknown packet')
  const ds=descriptors(object),out=Object.create(null)
  for(const [k,d] of Object.entries(ds)) {
    if(!own(d,'value'))throw Error('accessor packet')
    out[k]=d.value
  }
  return out
}
function fields(object) {
  return Object.fromEntries(Object.entries(values(object)).map(([k,v])=>[k,fingerprint(v)]))
}

export function observeWorkerInputs(V) {
  const records=[],byWorker=new WeakMap(),restores=[]
  const failures=[]
  function state(worker,pool) {
    let s=byWorker.get(worker)
    if(!s) {
      s={pool,starts:[],runs:[],requests:[],pending:new Map(),responses:0,unknown:0,errors:[]}
      byWorker.set(worker,s);records.push(s)
    }
    return s
  }
  function observe(worker,pool,direction,message) {
    const s=state(worker,pool)
    try {
      const m=values(message)
      if(direction==='out'&&m.__vitest_worker_request__===true) {
        if(m.type==='start') {
          const w=values(worker)
          s.starts.push({packet:fields(message),context:fields(m.context),
            config:fields(values(m.context).config),env:fingerprint(w.env),execArgv:fingerprint(w.execArgv)})
        }
        else if(m.type==='run'||m.type==='collect') {
          const c=values(m.context)
          const files=Array.isArray(c.files)?c.files.map(f=>values(f).filepath):[]
          s.runs.push({type:m.type,files,packet:fields(message),context:fields(m.context)})
        }else if(m.type!=='stop')s.unknown++
        return
      }
      if(direction==='in'&&m.__vitest_worker_response__===true)return
      if(direction==='in'&&m.t==='q') {
        const method=methods.has(m.m)?m.m:typeof m.m==='string'?'unknown:'+hash(m.m):'unknown:invalid'
        const r={method,args:fingerprint(m.a),optional:m.o===true,response:null}
        s.requests.push(r)
        if(typeof m.i==='string') {
          if(s.pending.has(m.i))throw Error('duplicate pending request')
          s.pending.set(m.i,r)
        }
        return
      }
      if(direction==='out'&&m.t==='s') {
        const r=s.pending.get(m.i)
        if(!r)throw Error('unmatched response')
        r.response={value:fingerprint(m.r),error:fingerprint(m.e),fields:Object.keys(m)}
        s.pending.delete(m.i);s.responses++
        return
      }
      s.unknown++
    }catch(e){s.errors.push(e.message)}
  }
  const patches=[]
  for(const [name,pool] of [['ForksPoolWorker','forks'],['ThreadsPoolWorker','threads']]) {
    const p=V[name]?.prototype
    if(!p||typeof p.send!=='function'||typeof p.deserialize!=='function') {
      failures.push('missing '+name+' interface');continue
    }
    for(const [method,direction] of [['send','out'],['deserialize','in']]) {
      patches.push({p,pool,method,direction,original:p[method],owned:own(p,method)})
    }
  }
  for(const {p,pool,method,direction,original,owned} of patches) {
      const note=(worker,message)=>{
        try{observe(worker,pool,direction,message)}catch{failures.push('packet observation failed')}
      }
      const wrapper=function(...args) {
        if(direction==='out')note(this,args[0])
        const result=apply(original,this,args)
        if(direction==='in')note(this,result)
        return result
      }
      p[method]=wrapper
      restores.push(()=>{if(p[method]===wrapper){if(owned)p[method]=original;else delete p[method]}})
  }
  return {
    report() {
      return {replayEligible:false,failures,workers:records.map(({pending,...s})=>({...s,pending:pending.size})),
        limitations:['unobserved worker channels and side effects are not excluded',
          'hashing a temporary-module pathname does not hash its contents',
          'RPC validation must preserve ordering and stateful parent behavior',
          'matching packets alone do not prove worker dependency completeness']}
    },
    close(){for(const restore of restores)restore()},
  }
}
