// Diagnostic only: cover metadata delivery before assignment into the graph.
// Proxies and an extra Promise reaction are observable, and earlier raw aliases
// are not excluded. This never grants replay authority.
import {types} from 'node:util'

const descriptor=Object.getOwnPropertyDescriptor,define=Object.defineProperty
const apply=Reflect.apply,then=Promise.prototype.then
function data(object,key) {
  if(!object||typeof object!=='object'||types.isProxy(object))throw Error('unsupported transport object')
  const d=descriptor(object,key)
  if(d&&!Object.hasOwn(d,'value'))throw Error('transport accessor')
  return d?.value
}
function replace(object,key,value) {
  const d=descriptor(object,key)
  if(!d||!Object.hasOwn(d,'value')||!d.writable)throw Error('unwritable transport metadata')
  define(object,key,{...d,value})
}

export function observeTransportMetadata(VitestTransport,metadata) {
  const pending=new Map(),errors=[],counts={requests:0,responses:0,errorResponses:0,
    rpcMetadata:0,invocations:0,settled:0,transportMetadata:0,rejections:0}
  let overflow=false,closed=false
  const p=VitestTransport?.prototype,d=p&&descriptor(p,'invoke'),original=d?.value
  function sent(message) {
    try {
      if(data(message,'t')!=='q')return
      const method=data(message,'m')
      if(method!=='fetch'&&method!=='fetchWarmModules')return
      const id=data(message,'i')
      if(typeof id!=='string')throw Error('module request has no identifier')
      if(pending.has(id))throw Error('duplicate module request identifier')
      if(pending.size>=10000){overflow=true;return}
      pending.set(id,method);counts.requests++
    }catch(e){errors.push(e.message)}
  }
  function received(message) {
    try {
      if(data(message,'t')!=='s')return
      const id=data(message,'i'),method=pending.get(id)
      if(!method)return
      pending.delete(id);counts.responses++
      if(data(message,'e')!==undefined){counts.errorResponses++;return}
      const value=data(message,'r')
      if(method==='fetch') {
        replace(message,'r',metadata.observeMetadata(value,'rpc:fetch'));counts.rpcMetadata++
      } else {
        if(!value||typeof value!=='object'||types.isProxy(value))throw Error('unsupported warm module map')
        const proto=Object.getPrototypeOf(value)
        if(proto!==null&&proto!==Object.prototype)throw Error('unsupported warm module prototype')
        for(const key of Reflect.ownKeys(value)) {
          if(typeof key!=='string')throw Error('symbol warm module key')
          replace(value,key,metadata.observeMetadata(data(value,key),'rpc:warm'));counts.rpcMetadata++
        }
      }
    }catch(e){errors.push(e.message)}
  }
  const wrapper=function(...args) {
    let fetch=false
    try {
      const event=args[0]
      fetch=data(event,'type')==='custom'&&data(event,'event')==='vite:invoke'&&data(data(event,'data'),'name')==='fetchModule'
    }catch(e){errors.push(e.message)}
    const result=apply(original,this,args)
    if(!fetch||closed)return result
    counts.invocations++
    if(!types.isPromise(result)){errors.push('transport did not return a native Promise');return result}
    // Register before the caller receives the original promise. Preserve its
    // identity, receiver, rejection and thrown-error behavior.
    apply(then,result,[value=>{
      counts.settled++
      if(closed)return
      try {
        if(data(value,'error')!==undefined)return
        replace(value,'result',metadata.observeMetadata(data(value,'result'),'transport'))
        counts.transportMetadata++
      }catch(e){errors.push(e.message)}
    },()=>{counts.settled++;counts.rejections++}])
    return result
  }
  if(typeof original!=='function')errors.push('missing Vitest transport invoke')
  else define(p,'invoke',{...d,value:wrapper})
  return {sent,received,
    report:()=>({replayEligible:false,completeOwnership:false,counts:{...counts},pending:pending.size,
      unsettled:counts.invocations-counts.settled,overflow,errors:[...errors],
      limitations:['An earlier IPC listener or a retained raw alias can bypass the observer',
        'The observer adds a Promise reaction and changes metadata descriptors or identity before delivery',
        'Transport callback ownership and worker side effects are not certified']}),
    close(){closed=true;if(p&&descriptor(p,'invoke')?.value===wrapper)define(p,'invoke',d)}}
}
