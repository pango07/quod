// Diagnostic only: observe evaluator inputs and debugger-reported source bytes.
// Unbound vm.Script compilations may be absent; neither observation permits replay.
import {Session} from 'node:inspector'
import {compiledScriptLog} from './compiled-script-log.mjs'
import {createHash} from 'node:crypto'
import {types} from 'node:util'
const isProxy=types.isProxy
import {fingerprint} from './worker-input-probe.mjs'

const apply=Reflect.apply,descriptor=Object.getOwnPropertyDescriptor
const mapGet=Function.prototype.call.bind(Map.prototype.get),mapSet=Function.prototype.call.bind(Map.prototype.set),
  freeze=Object.freeze,charCodeAt=Function.prototype.call.bind(String.prototype.charCodeAt),
  weakGet=Function.prototype.call.bind(WeakMap.prototype.get),weakSet=Function.prototype.call.bind(WeakMap.prototype.set),
  stringify=JSON.stringify
const pinnedSourceHash=process.versions.node==='24.18.0'
// JS source is a sequence of UTF-16 code units. UTF-8 encoding replaces lone
// surrogates, which would conflate distinct source strings and test results.
const sourceDigest=value=>({encoding:'utf16le',bytes:Buffer.byteLength(value,'utf16le'),
  sha256:createHash('sha256').update(value,'utf16le').digest('hex')})
function metadataFields(value) {
  try {
    if(value===undefined)return {available:true,fields:{}}
    if(!value||typeof value!=='object'||isProxy(value))throw Error('unsupported module metadata')
    const fields={}
    for(const name of Reflect.ownKeys(value)) {
      if(typeof name!=='string')throw Error('symbol metadata field')
      const d=descriptor(value,name)
      if(!d||!Object.hasOwn(d,'value'))throw Error('metadata accessor')
      Object.defineProperty(fields,name,{value:fingerprint(d.value),enumerable:true})
    }
    return {available:true,fields}
  }catch(e){return {available:false,reason:e.message}}
}
function field(object,name) {
  if(!object||typeof object!=='object'||isProxy(object))throw Error('unsupported module object')
  const d=descriptor(object,name)
  if(!d)return undefined
  if(!Object.hasOwn(d,'value'))throw Error('module accessor')
  return d.value
}

export function observeModuleEvaluation(Evaluator,{metadataForNode=null}={}) {
  const events=[],errors=[],restores=[]
  let overflow=false
  function note(kind,args) {
    if(events.length>=10000){overflow=true;return}
    if(kind==='external') {
      if(typeof args[0]!=='string')throw Error('unsupported external module id')
      events.push({kind,identity:fingerprint(args[0])});return
    }
    const [,code,module]=args
    if(typeof code!=='string')throw Error('unsupported evaluator code')
    const id=field(module,'id'),url=field(module,'url'),file=field(module,'file')
    const owned=metadataForNode?metadataForNode(module):null
    if(metadataForNode&&!owned?.available)throw Error('module metadata ownership unavailable')
    const meta=metadataForNode?owned.value:field(module,'meta')
    events.push({kind,identity:fingerprint({id,url,file}),meta:fingerprint(meta),metaFields:metadataFields(meta),
      code:sourceDigest(code)})
  }
  for(const [method,kind] of [['runInlinedModule','inline'],['runExternalModule','external']]) {
    const p=Evaluator?.prototype,d=p&&descriptor(p,method),original=d?.value
    if(typeof original!=='function'){errors.push('missing evaluator method: '+method);continue}
    const wrapper=function(...args) {
      try{note(kind,args)}catch(e){errors.push(e.message)}
      return apply(original,this,args)
    }
    Object.defineProperty(p,method,{...d,value:wrapper})
    restores.push(()=>{if(descriptor(p,method)?.value===wrapper)Object.defineProperty(p,method,d)})
  }
  return {report:()=>({replayEligible:false,events,errors,overflow,
    limitation:'Evaluator inputs do not include all context values, external dependencies or side effects'}),
    close(){for(const restore of restores)restore()}}
}

export function compiledScriptRecord(params,source,failedParse=false) {
  const language=params.scriptLanguage??'JavaScript'
  if(language!=='JavaScript'&&language!=='WebAssembly')throw Error('unsupported compiled language')
  let code
  if(language==='WebAssembly') {
    if(typeof source.bytecode!=='string'||!source.bytecode.length)throw Error('compiled bytecode unavailable')
    const bytes=Buffer.from(source.bytecode,'base64')
    if(bytes.toString('base64')!==source.bytecode)throw Error('invalid compiled bytecode')
    code={encoding:'bytes',bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')}
  }else {
    if(typeof source.scriptSource!=='string')throw Error('compiled source unavailable')
    code=sourceDigest(source.scriptSource)
  }
  return compiledRecordWithCode(params,code,failedParse)
}

function compiledRecordWithCode(params,code,failedParse) {
  const language=params.scriptLanguage??'JavaScript'
  if(params.url!==undefined&&typeof params.url!=='string')throw Error('unsupported compiled URL')
  // The protocol supplies these primitives. Encode the same fingerprint
  // without calling the observed native isProxy from an Inspector event.
  const fields={url:params.url,language,isModule:params.isModule===true,hasSourceURL:params.hasSourceURL===true,failedParse}
  const identity=['object',0,Object.entries(fields).map(([key,value])=>[key,
    value===undefined?['undefined']:[typeof value,value]])]
  return {identity:{available:true,hash:createHash('sha256').update(JSON.stringify(identity)).digest('hex')},
    code,language,kind:language==='WebAssembly'?'wasm':params.url?.startsWith('node:')?'builtin':params.url?.startsWith('file:')?'file':params.url?'named':'anonymous'}
}

// Node 24.18's Script::GetScriptHash hashes String::ToCString(), which maps
// NUL to space and encodes UTF-16 as UTF-8. It is NOT an exact-source digest.
// For a source consisting only of non-NUL ASCII, with no spaces, that
// conversion has a unique preimage. Admit a cache entry only after fetching
// the complete source and checking this restriction, its length and hash.
// Every other source (including Wasm) keeps the complete-source path.
export function createCompiledScriptRecorder(readSource,{maxCachedSources=256}={}) {
  if(!Number.isSafeInteger(maxCachedSources)||maxCachedSources<0||maxCachedSources>4096)throw Error('invalid compiled source cache limit')
  const cache=new Map(),serializations=new WeakMap()
  let entries=0,events=0,sourceReads=0,cacheHits=0
  function key(params) {
    if(!pinnedSourceHash||params.isLiveEdit||(params.scriptLanguage??'JavaScript')!=='JavaScript'||
      typeof params.hash!=='string'||params.hash.length!==64||!Number.isSafeInteger(params.length)||params.length<0)return null
    return params.length+':'+params.hash
  }
  function memo(params,failedParse,record) {
    const owned=freeze({...record,identity:freeze(record.identity),code:freeze(record.code)})
    // Only our deeply frozen scalar record can reuse a serialized key.
    // Null-prototype mirrors prevent inherited toJSON from changing the
    // representation of these owned fields. Mutable records keep the full
    // serialization path in the log.
    weakSet(serializations,owned,stringify({__proto__:null,
      identity:{__proto__:null,available:owned.identity.available,hash:owned.identity.hash},
      code:{__proto__:null,encoding:owned.code.encoding,bytes:owned.code.bytes,sha256:owned.code.sha256},
      language:owned.language,kind:owned.kind}))
    return {url:params.url,isModule:params.isModule===true,hasSourceURL:params.hasSourceURL===true,failedParse,
      record:owned}
  }
  return {
    record(params,failedParse=false) {
      events++
      const id=key(params),cached=id===null?undefined:mapGet(cache,id)
      if(cached) {
        cacheHits++
        const last=cached.last
        if(last.url!==params.url||last.isModule!==(params.isModule===true)||
          last.hasSourceURL!==(params.hasSourceURL===true)||last.failedParse!==failedParse)
          cached.last=memo(params,failedParse,compiledRecordWithCode(params,cached.code,failedParse))
        return cached.last.record
      }
      sourceReads++
      const source=readSource(params.scriptId),record=compiledScriptRecord(params,source,failedParse)
      if(id!==null&&entries<maxCachedSources&&typeof source.scriptSource==='string'&&source.scriptSource.length===params.length) {
        let injective=true
        for(let i=0;i<source.scriptSource.length;i++) {
          const c=charCodeAt(source.scriptSource,i)
          if(c===0||c===32||c>127){injective=false;break}
        }
        if(injective&&createHash('sha256').update(source.scriptSource,'utf8').digest('hex')===params.hash) {
          const last=memo(params,failedParse,record)
          // Source acquisition may synchronously deliver another script event.
          // Recheck after that work so nested admission cannot duplicate a slot.
          if(entries<maxCachedSources&&mapGet(cache,id)===undefined) {
            mapSet(cache,id,{code:last.record.code,last});entries++
          }
          return last.record
        }
      }
      return record
    },
    cachedKey:record=>weakGet(serializations,record),
    counts:()=>({policy:'pinned-injective-ascii',enabled:pinnedSourceHash,events,sourceReads,cacheHits,entries,capacity:maxCachedSources}),
  }
}

export function observeCompiledScripts() {
  const session=new Session(),log=compiledScriptLog(),errors=[]
  let failedToParse=0
  function post(method,params={}) {
    let done=false,error,value
    session.post(method,params,(e,r)=>{done=true;error=e;value=r})
    if(!done||error)throw Error('compilation observation unavailable')
    return value
  }
  function observe(params,failedParse) {
    if(failedParse)failedToParse++
    try {
      const source=post('Debugger.getScriptSource',{scriptId:params.scriptId})
      log.append(compiledScriptRecord(params,source,failedParse))
    }catch(e){errors.push(e.message)}
  }
  session.on('Debugger.scriptParsed',({params})=>observe(params,false))
  session.on('Debugger.scriptFailedToParse',({params})=>observe(params,true))
  try{session.connect();post('Debugger.enable')}catch(e){errors.push(e.message)}
  return {report:()=>({replayEligible:false,completeCompilationCoverage:false,...log.report(),errors,failedToParse,
    limitation:'Debugger-reported scripts can omit unbound vm.Script compilations; this is neither an execution trace nor a complete module dependency certificate'}),
    close(){try{session.disconnect()}catch{}}}
}
