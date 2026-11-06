// Diagnostic only: observe evaluator inputs and actual V8 compilation bytes.
// Compilation need not execute code; neither observation permits replay.
import {Session} from 'node:inspector'
import {createHash} from 'node:crypto'
import {types} from 'node:util'
import {fingerprint} from './worker-input-probe.mjs'

const apply=Reflect.apply,descriptor=Object.getOwnPropertyDescriptor
const digest=value=>createHash('sha256').update(value).digest('hex')
function field(object,name) {
  if(!object||typeof object!=='object'||types.isProxy(object))throw Error('unsupported module object')
  const d=descriptor(object,name)
  if(!d)return undefined
  if(!Object.hasOwn(d,'value'))throw Error('module accessor')
  return d.value
}

export function observeModuleEvaluation(Evaluator) {
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
    events.push({kind,identity:fingerprint({id,url,file}),meta:fingerprint(field(module,'meta')),
      code:{bytes:Buffer.byteLength(code),sha256:digest(code)}})
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
      if(params.scriptLanguage&&params.scriptLanguage!=='JavaScript')throw Error('unsupported compiled language')
      const source=post('Debugger.getScriptSource',{scriptId:params.scriptId}).scriptSource
      if(typeof source!=='string')throw Error('compiled source unavailable')
      scripts.push({identity:fingerprint({url:params.url,isModule:params.isModule===true,hasSourceURL:params.hasSourceURL===true,failedParse}),
        code:{bytes:Buffer.byteLength(source),sha256:digest(source)},
        kind:params.url?.startsWith('node:')?'builtin':params.url?.startsWith('file:')?'file':params.url?'named':'anonymous'})
    }catch(e){errors.push(e.message)}
  }
  session.on('Debugger.scriptParsed',({params})=>observe(params,false))
  session.on('Debugger.scriptFailedToParse',({params})=>observe(params,true))
  try{session.connect();post('Debugger.enable')}catch(e){errors.push(e.message)}
  return {report:()=>({replayEligible:false,scripts,errors,overflow,failedToParse,
    limitation:'Compiled scripts are not an execution trace or complete module dependency certificate'}),
    close(){try{session.disconnect()}catch{}}}
}
