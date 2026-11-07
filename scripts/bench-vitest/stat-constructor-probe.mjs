// Diagnostic only. Observe actual Node Stats from their constructor entry.
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
    if(!done||error)throw Error('constructor inspector request failed: '+method+
      (error?': '+error.message:': asynchronous response'))
    return result
  }
  function evaluate(expression) {
    const response=post('Runtime.evaluate',{expression,returnByValue:true,silent:true})
    if(response.exceptionDetails)throw Error('constructor diagnostic evaluation failed')
    return response.result?.value
  }
  function properties(objectId) {
    if(!objectId)throw Error('constructor provenance object unavailable')
    return post('Runtime.getProperties',{objectId,ownProperties:true})
  }
  function lexicalFunction(objectId,name) {
    const scopes=properties(objectId).internalProperties?.find(p=>p.name==='[[Scopes]]')?.value?.objectId
    for(const scope of properties(scopes).result) {
      if(!/^\d+$/.test(scope.name)||scope.value?.description==='Global')continue
      const value=properties(scope.value?.objectId).result.find(p=>p.name===name)?.value
      if(value?.type==='function'&&value.objectId)return value.objectId
    }
    throw Error('constructor provenance binding unavailable: '+name)
  }
  function queryFor(frames) {
    for(const frame of frames.slice(1,8)) {
      const url=scripts.get(frame.location?.scriptId)
      if(!['node:fs','node:internal/fs/promises'].includes(url)||
        !['stat','lstat','statSync','lstatSync'].includes(frame.functionName))continue
      const local=frame.scopeChain.find(scope=>scope.type==='local')?.object?.objectId
      if(!local)continue
      const value=properties(local).result.find(p=>p.name==='path')?.value
      if(value?.type!=='string')continue
      return {kind:frame.functionName.startsWith('lstat')?'lstat':'stat',path:value.value,
        function:frame.functionName,url,certified:false}
    }
    return null
  }
  function invoke(frame,kind,primitiveArguments,query) {
    const response=post('Runtime.callFunctionOn',{objectId:frame.this.objectId,
      functionDeclaration:`function(){return ${binding}.enter(this,${JSON.stringify(kind)},${primitiveArguments},${JSON.stringify(query)})}`,
      returnByValue:true,silent:true})
    if(response.exceptionDetails||response.result?.value!==true)throw Error('constructor entry callback failed')
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
      const local=frame.scopeChain.find(scope=>scope.type==='local')?.object?.objectId
      if(!local)throw Error('constructor parameters unavailable')
      const parameters=post('Runtime.getProperties',{objectId:local,ownProperties:true}).result
      const names=commonFields.slice(0,10).concat(target.kind==='BigIntStats'?nsFields:commonFields.slice(10))
      const expectedType=target.kind==='BigIntStats'?'bigint':'number'
      const primitiveArguments=names.every(name=>parameters.find(p=>p.name===name)?.value?.type===expectedType)
      invoke(frame,target.kind,primitiveArguments,queryFor(params.callFrames))
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
      const define=Object.defineProperty,extensible=Object.isExtensible;
      const fields=${JSON.stringify(commonFields)},ns=${JSON.stringify(nsFields)};
      const installed=new WeakMap(),rows=[],rejections=[];
      function reject(kind,reason){rejections.push({kind,reason});return true}
      return {
        enter(target,kind,primitiveArguments,query){
          if(!primitiveArguments)return reject(kind,'nonprimitive constructor arguments');
          if(!addon.isInspectableOwner(target)||ownKeys(target).length)return reject(kind,'nonempty or unsupported constructor owner');
          if(!extensible(target))return reject(kind,'nonextensible constructor owner');
          let p=proto(target),depth=0;
          while(p!==null){
            if(++depth>8||!addon.isInspectableOwner(p))return reject(kind,'unsupported constructor prototype');
            for(const field of fields.concat(ns))if(descriptor(p,field))return reject(kind,'inherited constructor field: '+field);
            p=proto(p);
          }
          // The source-pinned constructors only assign these fields when
          // their parameters are primitive. An inherited setter is rejected
          // above before any field is installed. Each temporary setter turns
          // itself into an ordinary-looking native data field on its first
          // constructor assignment, before callers can receive the Stats.
          const expected=fields.concat(kind==='BigIntStats'?ns:[]);
          const row={kind,query,owner:new WeakRef(target),reads:new Map(),initialized:0,expected:expected.length,failed:false};
          const observers=[];installed.set(target,observers);rows.push(row);
          for(const key of expected)define(target,key,{enumerable:true,configurable:true,set(value){
            define(this,key,{value,writable:true,enumerable:true,configurable:true});
            if(this!==target){row.failed=true;return;}
            row.initialized++;
            try{observers.push(addon.createDataReadObserver(target,key,()=>row.reads.set(key,(row.reads.get(key)??0)+1)))}
            catch{row.failed=true;}
          }});
          return true;
        },
        report(){return {rejections,rows:rows.map(row=>{const owner=row.owner.deref(),observers=owner&&installed.get(owner);return {
          kind:row.kind,query:row.query,fields:[...row.reads.keys()].sort(),calls:[...row.reads.values()].reduce((a,b)=>a+b,0),
          collected:!owner,initialized:row.initialized,expected:row.expected,
          intact:observers?!row.failed&&row.initialized===row.expected&&observers.every(o=>o.report().available):null};})}},
        close(){for(const row of rows){const owner=row.owner.deref();if(owner)for(const o of installed.get(owner)||[])o.close();}rows.length=0;}
      };
    })();`)
    initialized=true
    // fs.Stats is a deprecation wrapper. Resolve the actual constructors
    // through the native promise stat function's lexical conversion helper.
    const stat=post('Runtime.evaluate',{expression:"process.getBuiltinModule('fs').promises.stat",silent:true})
    if(stat.exceptionDetails)throw Error('native stat function unavailable')
    const convert=lexicalFunction(stat.result?.objectId,'getStatsFromBinding')
    for(const kind of ['Stats','BigIntStats']) {
      const objectId=lexicalFunction(convert,kind)
      const location=properties(objectId).internalProperties?.find(p=>p.name==='[[FunctionLocation]]')?.value?.value
      if(location?.scriptId!==scriptId)throw Error('constructor function source differs')
      const result=post('Debugger.setBreakpointOnFunctionCall',{objectId})
      breakpoints.set(result.breakpointId,{kind})
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
