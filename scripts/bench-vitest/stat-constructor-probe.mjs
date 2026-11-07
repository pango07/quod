// Diagnostic only. Observe actual Node Stats before constructor return.
// Startup/native/debugger integrity and query attribution are not certified.
import {Session} from 'node:inspector'
import {createHash,randomBytes} from 'node:crypto'
import path from 'node:path'

const sourceHash='a47d9b1f38bd21771eb4a93b4e1730b8f158ec0fb07097d638fb5436fb945770'
const commonFields=['dev','mode','nlink','uid','gid','rdev','blksize','ino','size','blocks',
  'atimeMs','mtimeMs','ctimeMs','birthtimeMs']
const nsFields=['atimeNs','mtimeNs','ctimeNs','birthtimeNs']

export function createStatConstructorProbe() {
  if(process.versions.node!=='24.18.0')throw Error('constructor probe requires pinned Node')
  const addon=process.env.QUOD_NATIVE_CAPTURE_ADDON
  if(!addon||!path.isAbsolute(addon))throw Error('constructor probe requires native helper')
  const session=new Session(),scripts=new Map(),breakpoints=new Map(),failures=[]
  const binding='__quod_stat_'+randomBytes(16).toString('hex')
  let closed=false,initialized=false,pauses=0,source=null
  function post(method,params={}) {
    let done=false,error,result
    session.post(method,params,(e,r)=>{done=true;error=e;result=r})
    if(!done||error)throw Error('constructor inspector request failed: '+method)
    return result
  }
  function evaluate(expression) {
    const response=post('Runtime.evaluate',{expression,returnByValue:true,silent:true})
    if(response.exceptionDetails)throw Error('constructor diagnostic evaluation failed')
    return response.result?.value
  }
  function invoke(frame,method,kind) {
    const response=post('Runtime.callFunctionOn',{objectId:frame.this.objectId,
      functionDeclaration:`function(){return ${binding}.${method}(this,${JSON.stringify(kind)})}`,
      returnByValue:true,silent:true})
    if(response.exceptionDetails||response.result?.value!==true)throw Error('constructor '+method+' callback failed')
  }
  session.connect()
  session.on('Debugger.scriptParsed',({params})=>scripts.set(params.scriptId,params.url))
  session.on('Debugger.paused',({params})=>{
    try {
      const hits=(params.hitBreakpoints??[]).filter(id=>breakpoints.has(id))
      if(hits.length!==1)throw Error('unexpected constructor breakpoint set')
      const target=breakpoints.get(hits[0]),frame=params.callFrames?.[0]
      if(!frame?.this?.objectId||frame.functionName!==target.kind||frame.location.scriptId!==source?.scriptId)
        throw Error('unexpected constructor frame')
      pauses++
      invoke(frame,target.method,target.kind)
    }catch(error){failures.push(error.message)}
    finally {try{post('Debugger.resume')}catch{failures.push('constructor resume failed')}}
  })
  try {
    post('Runtime.enable');post('Debugger.enable')
    const matches=[...scripts].filter(([,url])=>url==='node:internal/fs/utils')
    if(matches.length!==1)throw Error('native fs utils script is ambiguous')
    const scriptId=matches[0][0],text=post('Debugger.getScriptSource',{scriptId}).scriptSource
    const actual=createHash('sha256').update(text).digest('hex')
    if(actual!==sourceHash)throw Error('native fs utils source differs: '+actual)
    source={scriptId,sha256:actual}
    // This private global lexical binding is a diagnostic bridge, not a
    // production ownership boundary. It is cleared on close. WeakMap values
    // keep observer handles alive only while their Stats owner is alive.
    evaluate(`let ${binding}=(()=>{
      const addon=process.getBuiltinModule('module').createRequire(${JSON.stringify(import.meta.url)})(${JSON.stringify(addon)});
      const ownKeys=Reflect.ownKeys,proto=Object.getPrototypeOf,descriptor=Object.getOwnPropertyDescriptor;
      const fields=${JSON.stringify(commonFields)},ns=${JSON.stringify(nsFields)};
      const pending=new WeakMap(),installed=new WeakMap(),rows=[],rejections=[];
      function reject(kind,reason){rejections.push({kind,reason});return true}
      return {
        enter(target,kind){
          if(!addon.isInspectableOwner(target)||ownKeys(target).length)return reject(kind,'nonempty or unsupported constructor owner');
          let p=proto(target),depth=0;
          while(p!==null){
            if(++depth>8||!addon.isInspectableOwner(p))return reject(kind,'unsupported constructor prototype');
            for(const field of fields.concat(ns))if(descriptor(p,field))return reject(kind,'inherited constructor field: '+field);
            p=proto(p);
          }
          pending.set(target,kind);return true;
        },
        leave(target,kind){
          if(pending.get(target)!==kind)return true;
          pending.delete(target);
          const expected=fields.concat(kind==='BigIntStats'?ns:[]),keys=ownKeys(target);
          if(keys.length!==expected.length||keys.some(k=>!expected.includes(k)))return reject(kind,'unexpected constructed fields');
          for(const key of keys){const d=descriptor(target,key);if(!d||!Object.hasOwn(d,'value')||
            !d.writable||!d.enumerable||!d.configurable||!['number','bigint'].includes(typeof d.value))return reject(kind,'unexpected constructed descriptor');}
          const row={kind,owner:new WeakRef(target),reads:new Map()},observers=[];
          try{for(const key of keys)observers.push(addon.createDataReadObserver(target,key,()=>row.reads.set(key,(row.reads.get(key)??0)+1)))}
          catch{for(const o of observers)o.close();return reject(kind,'native field installation failed')}
          installed.set(target,observers);rows.push(row);return true;
        },
        report(){return {rejections,rows:rows.map(row=>{const owner=row.owner.deref(),observers=owner&&installed.get(owner);return {
          kind:row.kind,fields:[...row.reads.keys()].sort(),calls:[...row.reads.values()].reduce((a,b)=>a+b,0),
          collected:!owner,intact:observers?observers.every(o=>o.report().available):null};})}},
        close(){for(const row of rows){const owner=row.owner.deref();if(owner)for(const o of installed.get(owner)||[])o.close();}rows.length=0;}
      };
    })();`)
    initialized=true
    for(const kind of ['Stats','BigIntStats']) {
      const start=text.indexOf('function '+kind+'('),end=text.indexOf('\n}',start)
      if(start<0||end<start)throw Error('constructor source extent unavailable')
      const line=offset=>text.slice(0,offset).split('\n').length-1
      const locations=post('Debugger.getPossibleBreakpoints',{start:{scriptId,lineNumber:line(start),columnNumber:0},
        end:{scriptId,lineNumber:line(end)+2,columnNumber:0},restrictToFunction:true}).locations
      const returns=locations.filter(location=>location.type==='return')
      const entries=locations.filter(location=>location.type!=='return')
      if(returns.length!==1||!entries.length)throw Error('constructor entry/return points unavailable: '+kind)
      for(const [method,location] of [['enter',entries[0]],['leave',returns[0]]]) {
        const {type,...position}=location
        const result=post('Debugger.setBreakpoint',{location:position})
        if(result.actualLocation.lineNumber!==position.lineNumber||result.actualLocation.columnNumber!==position.columnNumber)
          throw Error('constructor breakpoint moved')
        breakpoints.set(result.breakpointId,{kind,method})
      }
    }
  }catch(error){
    for(const id of breakpoints.keys())try{post('Debugger.removeBreakpoint',{breakpointId:id})}catch{}
    if(initialized)try{evaluate(`${binding}.close(); ${binding}=undefined`)}catch{}
    session.disconnect();throw error
  }
  return {
    report(){
      if(closed)throw Error('constructor probe closed')
      return {source,pauses,failures:[...failures],...evaluate(`${binding}.report()`),
        replayEligible:false,completeOwnership:false,
        limitations:['startup, other native/debugger observers and query identity are not certified',
          'the diagnostic bridge is observable through debugger capabilities']}
    },
    close(){
      if(closed)return
      for(const id of breakpoints.keys())post('Debugger.removeBreakpoint',{breakpointId:id})
      evaluate(`${binding}.close(); ${binding}=undefined`)
      session.disconnect();closed=true
    },
  }
}
