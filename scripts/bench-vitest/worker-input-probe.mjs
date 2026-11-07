// Diagnostic only: inventory the messages entering isolated Vitest workers.
// Hash values rather than publishing configuration, environment or source text.
// This is not a complete dependency certificate and never permits replay.
import {createHash} from 'node:crypto'
import {types} from 'node:util'
const isProxy=types.isProxy
import fs from 'node:fs'

const {openSync,closeSync,fstatSync,readFileSync}=fs
const {O_RDONLY,O_NOATIME,O_NOFOLLOW}=fs.constants

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
    if(isProxy(v))throw Error('proxy value')
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
  if(!object||typeof object!=='object'||isProxy(object))throw Error('unknown packet')
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

// Read the exact module bytes behind a transport path. Unknown platforms,
// symlinks, changed files and access-time changes remain explicit failures.
// This does not prove that the worker later read those same bytes.
export function moduleArtifact(file) {
  let fd
  try {
    if(process.platform!=='linux'||!Number.isInteger(O_NOATIME)||!Number.isInteger(O_NOFOLLOW))throw Error('unsupported artifact reader')
    if(typeof file!=='string'||!file.startsWith('/'))throw Error('non-absolute module artifact')
    fd=openSync(file,O_RDONLY|O_NOATIME|O_NOFOLLOW)
    const before=fstatSync(fd,{bigint:true})
    if(!before.isFile()||before.size>33554432n)throw Error('unsupported module artifact size or kind')
    const content=readFileSync(fd),after=fstatSync(fd,{bigint:true})
    const stable=['dev','ino','mode','size','nlink','mtimeNs','ctimeNs']
    if(!stable.every(k=>before[k]===after[k])||BigInt(content.length)!==before.size)throw Error('module artifact changed while reading')
    if(before.atimeNs!==after.atimeNs)throw Error('module artifact access time changed')
    return {available:true,bytes:content.length,sha256:hash(content),noAtimeRead:true}
  }catch(e){return {available:false,reason:e.code??e.message}}
  finally{if(fd!==undefined)try{closeSync(fd)}catch{}}
}

export function moduleDetails(value) {
  try {
    const v=values(value),body=typeof v.tmp==='string'?moduleArtifact(v.tmp):
      typeof v.code==='string'?{available:true,bytes:Buffer.byteLength(v.code),sha256:hash(v.code),inline:true}:null
    return {available:true,fields:fields(value),body,
      identity:fingerprint({id:v.id,file:v.file,url:v.url,externalize:v.externalize,type:v.type,moduleType:v.moduleType}),
      kind:typeof v.externalize==='string'?'external':body?'transformed':v.cache===true?'cache-ack':'unknown',
      replayEligible:false}
  }catch(e){return {available:false,reason:e.message,replayEligible:false}}
}

// Compare diagnostic phases without conflating their explicitly assigned report
// directories with application environment differences. Retain the full digest;
// this comparison must never be used as a replay key or an environment policy.
const reportDirectoryNames=new Set(['QUOD_WORKER_MODULE_METADATA_REPORT_DIR',
  'QUOD_WORKER_MODULE_REPORT_DIR','QUOD_WORKER_CONFIG_REPORT_DIR'])
export function diagnosticEnvironment(env,expected={}) {
  const full=fingerprint(env)
  try {
    if(!full.available)throw Error('environment fingerprint unavailable')
    if(!fingerprint(expected).available)throw Error('diagnostic directory assignment unavailable')
    const original=values(env),owned=values(expected),copy=Object.create(null),excluded=[]
    for(const [name,value] of Object.entries(owned)) {
      if(!reportDirectoryNames.has(name)||typeof value!=='string'||!value.startsWith('/')||original[name]!==value)
        throw Error('diagnostic report directory does not match its assigned value')
      excluded.push(name)
    }
    for(const [name,value] of Object.entries(original))if(!own(owned,name))copy[name]=value
    return {available:true,full,comparison:fingerprint(copy),excluded:excluded.sort(),replayEligible:false}
  }catch(error){return {available:false,full,reason:error.message,replayEligible:false}}
}

export function observeWorkerInputs(V,{reportDirectories={}}={}) {
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
            config:fields(values(m.context).config),sequence:fields(values(values(m.context).config).sequence??{}),
            env:fingerprint(w.env),diagnosticEnvironment:diagnosticEnvironment(w.env,reportDirectories),execArgv:fingerprint(w.execArgv)})
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
        if(m.e===undefined&&r.method==='fetch')r.response.modules=[moduleDetails(m.r)]
        if(m.e===undefined&&r.method==='fetchWarmModules') {
          r.response.modules=Object.entries(values(m.r)).map(([url,v])=>({url:fingerprint(url),...moduleDetails(v)}))
        }
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
          'artifact bytes observed before sending do not prove what a worker later reads',
          'RPC validation must preserve ordering and stateful parent behavior',
          'matching packets alone do not prove worker dependency completeness']}
    },
    close(){for(const restore of restores)restore()},
  }
}
