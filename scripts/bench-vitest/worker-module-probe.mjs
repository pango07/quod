// Diagnostic only: observe evaluator inputs and debugger-reported source bytes.
// Unbound vm.Script compilations may be absent; neither observation permits replay.
import {Session} from 'node:inspector'
import {createHash} from 'node:crypto'
import {types} from 'node:util'
import {fingerprint} from './worker-input-probe.mjs'

const apply=Reflect.apply,descriptor=Object.getOwnPropertyDescriptor
// JS source is a sequence of UTF-16 code units. UTF-8 encoding replaces lone
// surrogates, which would conflate distinct source strings and test results.
const sourceDigest=value=>({encoding:'utf16le',bytes:Buffer.byteLength(value,'utf16le'),
  sha256:createHash('sha256').update(value,'utf16le').digest('hex')})
function metadataFields(value) {
  try {
    if(value===undefined)return {available:true,fields:{}}
    if(!value||typeof value!=='object'||types.isProxy(value))throw Error('unsupported module metadata')
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
  if(!object||typeof object!=='object'||types.isProxy(object))throw Error('unsupported module object')
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
  if(params.url!==undefined&&typeof params.url!=='string')throw Error('unsupported compiled URL')
  // The protocol supplies these primitives. Encode the same fingerprint
  // without calling the observed native isProxy from an Inspector event.
  const fields={url:params.url,language,isModule:params.isModule===true,hasSourceURL:params.hasSourceURL===true,failedParse}
  const identity=['object',0,Object.entries(fields).map(([key,value])=>[key,
    value===undefined?['undefined']:[typeof value,value]])]
  return {identity:{available:true,hash:createHash('sha256').update(JSON.stringify(identity)).digest('hex')},
    code,language,kind:language==='WebAssembly'?'wasm':params.url?.startsWith('node:')?'builtin':params.url?.startsWith('file:')?'file':params.url?'named':'anonymous'}
}

export function observeCompiledScripts() {
  const session=new Session(),scripts=[],errors=[]
  let overflow=false,failedToParse=0
  function post(method,params={}) {
    let done=false,error,value
    session.post(method,params,(e,r)=>{done=true;error=e;value=r})
    if(!done||error)throw Error('compilation observation unavailable')
    return value
  }
  function observe(params,failedParse) {
    if(failedParse)failedToParse++
    if(scripts.length>=20000){overflow=true;return}
    try {
      const source=post('Debugger.getScriptSource',{scriptId:params.scriptId})
      scripts.push(compiledScriptRecord(params,source,failedParse))
    }catch(e){errors.push(e.message)}
  }
  session.on('Debugger.scriptParsed',({params})=>observe(params,false))
  session.on('Debugger.scriptFailedToParse',({params})=>observe(params,true))
  try{session.connect();post('Debugger.enable')}catch(e){errors.push(e.message)}
  return {report:()=>({replayEligible:false,completeCompilationCoverage:false,scripts,errors,overflow,failedToParse,
    limitation:'Debugger-reported scripts can omit unbound vm.Script compilations; this is neither an execution trace nor a complete module dependency certificate'}),
    close(){try{session.disconnect()}catch{}}}
}
