// Diagnostic only: cover metadata delivery before assignment into the graph.
// Metadata instrumentation is observable, and earlier raw aliases are not
// excluded. Legacy reaction mode also adds a Promise reaction; consumer mode
// uses the loader's existing continuation. Neither grants replay authority.
import {types} from 'node:util'
const isProxy=types.isProxy

const descriptor=Object.getOwnPropertyDescriptor,define=Object.defineProperty
const apply=Reflect.apply,then=Promise.prototype.then
function data(object,key) {
  if(!object||typeof object!=='object'||isProxy(object))throw Error('unsupported transport object')
  const d=descriptor(object,key)
  if(d&&!Object.hasOwn(d,'value'))throw Error('transport accessor')
  return d?.value
}
function replace(object,key,value) {
  const d=descriptor(object,key)
  if(!d||!Object.hasOwn(d,'value')||!d.writable)throw Error('unwritable transport metadata')
  define(object,key,{...d,value})
}

export function observeTransportMetadata(VitestTransport,metadata,{delivery='reaction'}={}) {
  if(!['reaction','consumer'].includes(delivery))throw Error('unknown transport delivery observation')
  const pending=new Map(),errors=[],counts={requests:0,responses:0,errorResponses:0,
    rpcMetadata:0,invocations:0,settled:0,transportMetadata:0,rejections:0}
  let overflow=false,closed=false
  const p=VitestTransport?.prototype,d=p&&descriptor(p,'invoke'),original=d?.value
  function delivered(value) {
    counts.settled++
    if(closed)return false
    try {
      if(data(value,'error')!==undefined)return true
      replace(value,'result',metadata.observeMetadata(data(value,'result'),'transport'))
      counts.transportMetadata++
      return true
    }catch(e){errors.push(e.message);return false}
  }
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
        if(!value||typeof value!=='object'||isProxy(value))throw Error('unsupported warm module map')
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
    if(delivery==='consumer')return result
    // Legacy diagnostic: retain the returned Promise identity, but add a
    // reaction that can affect ordering and unhandled-rejection reporting.
    apply(then,result,[value=>{
      delivered(value)
    },()=>{counts.settled++;counts.rejections++}])
    return result
  }
  if(typeof original!=='function')errors.push('missing Vitest transport invoke')
  else define(p,'invoke',{...d,value:wrapper})
  return {sent,received,delivered,
    report:()=>({replayEligible:false,completeOwnership:false,delivery,addsPromiseReaction:delivery==='reaction',
      counts:{...counts,rejections:delivery==='consumer'?null:counts.rejections},pending:pending.size,
      unsettled:counts.invocations-counts.settled,
      settledMeaning:delivery==='consumer'?'observed consumer deliveries':'observed Promise settlements',
      overflow,errors:[...errors],
      limitations:['An earlier IPC listener or a retained raw alias can bypass the observer',
        delivery==='reaction'?'The observer adds a Promise reaction and changes metadata descriptors or identity before delivery':
          'Only the pinned consumer continuation is observed; unsettled counts missing deliveries, not pending Promises; rejected Promises and other consumers remain unobserved',
        'Transport callback ownership and worker side effects are not certified']}),
    close(){closed=true;if(p&&descriptor(p,'invoke')?.value===wrapper)define(p,'invoke',d)}}
}
