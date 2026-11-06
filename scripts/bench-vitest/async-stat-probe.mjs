// Diagnostic only. Observe native promise provenance without adding a
// .then callback, replacing Stats, or granting a narrower dependency key.
import fs from 'node:fs'
import {promiseHooks} from 'node:v8'
import path from 'node:path'
import crypto from 'node:crypto'
import {inspectPromiseObservers} from './promise-observers-probe.mjs'

export function observeAsyncStat(project,{startup=null}={}) {
  const observerRegistry=startup?.observerRegistry??inspectPromiseObservers()
  const initialObservers=observerRegistry.snapshot()
  const nativeStatProvenance=startup?.provenance??observerRegistry.nativeStatProvenance()
  const startupAtAttach=startup?.snapshot()??null
  const original=fs.promises.stat, NativeStats=fs.Stats
  const descriptors=Object.getOwnPropertyDescriptors, descriptor=Object.getOwnPropertyDescriptor
  const getPrototypeOf=Object.getPrototypeOf, ownKeys=Reflect.ownKeys, apply=Reflect.apply
  const prototypes=[]
  for(let p=NativeStats.prototype;p;p=getPrototypeOf(p)) prototypes.push({object:p,parent:getPrototypeOf(p),shape:descriptors(p)})
  const nativeConstructor=descriptor(prototypes[0].object,'constructor')?.value
  const pristine=()=>descriptor(fs,'Stats')?.value===NativeStats &&
    descriptor(NativeStats,'prototype')?.value===prototypes[0].object && prototypes.every(({object,parent,shape})=>
      getPrototypeOf(object)===parent && ownKeys(object).length===ownKeys(shape).length && ownKeys(shape).every(k=>{
        const a=shape[k],b=descriptor(object,k)
        return b && a.value===b.value && a.get===b.get && a.set===b.set && a.writable===b.writable && a.configurable===b.configurable && a.enumerable===b.enumerable
      })) && typeof nativeConstructor==='function' && descriptor(nativeConstructor,'prototype')?.value===prototypes[0].object
  let binding=null
  try {binding=process.binding('async_wrap')} catch {}
  const counts=()=>{
    const result={}
    for(const name of ['kInit','kBefore','kAfter','kDestroy','kPromiseResolve','kTotals']) {
      const i=binding?.constants?.[name]
      result[name]=Number.isInteger(i) && binding.async_hook_fields ? binding.async_hook_fields[i] : null
    }
    return result
  }
  const calls=[], promises=new WeakMap(), roots=new Map(), stack=[]
  // Referenced by the owned before hook and inspected through that
  // hook's private V8 context, never through an application-visible global.
  const checkpoint={promise:null}
  let scope=null, active=null, nextPromise=0, eventMismatch=false, consumerProof=null, inspectorAtConsumerProof=null
  const snapshot=()=>({pristine:pristine(),native:observerRegistry.nativeStatSnapshot(),
    startup:startup?.snapshot()??null,
    opaque:observerRegistry.opaqueSnapshot(),
    asyncHooks:counts(),observers:observerRegistry.snapshot()})
  const stop=promiseHooks.createHook({
    init(p,parent) {
      const ancestor=promises.get(parent)
      const op=scope ?? (ancestor ? calls[ancestor.op-1] : null)
      if(!op) return
      const state={id:++nextPromise,op:op.id,parent:ancestor?.id??null,nativeCall:scope===op,
        init:snapshot(),before:[],settled:null}
      promises.set(p,state);op.promises.push(state)
    },
    before(p) {
      stack.push({promise:p,prior:active});active=p
      const state=promises.get(p)
      if(state) {
        state.before.push(snapshot())
        const op=calls[state.op-1]
        if(state.parent===op.root && roots.has(op.id)) {
          checkpoint.promise=roots.get(op.id)
          try {op.outcome=observerRegistry.promiseOutcome('before')}
          finally {checkpoint.promise=null;roots.delete(op.id)}
        }
      }
    },
    after(p) {
      const frame=stack.pop()
      if(!frame || frame.promise!==p) eventMismatch=true
      active=frame?.prior??null
    },
    settled(p) {
      const state=promises.get(p)
      if(!state) return
      const creator=active && promises.get(active)
      state.settled={insidePromise:creator?.id??null,insideOperation:creator?.op??null,...snapshot()}
    },
  })
  const ownedCaptured=observerRegistry.captureOwned()
  fs.promises.stat=function stat(...args) {
    const op={id:calls.length+1,path:typeof args[0]==='string'?path.relative(project,path.resolve(args[0])):'<non-string>',
      sizeConsumer:consumerProof?.statSizeConsumer(args)===true,
      stack:new Error().stack.split('\n').slice(2,10).join('\n'),before:snapshot(),promises:[],root:null}
    calls.push(op)
    const prior=scope;scope=op
    try {
      const result=apply(original,this,args)
      op.root=promises.get(result)?.id??null
      if(op.root) roots.set(op.id,result)
      // Return the very same native promise. No fulfillment observer and
      // no additional promise reaction are inserted by this diagnostic.
      return result
    } finally {scope=prior}
  }
  return {
    setConsumerProof(proof){consumerProof=proof;inspectorAtConsumerProof=observerRegistry.inspectorAccess()},
    report() {
      return {replayEligible:false,eventMismatch,initialObservers,ownedCaptured,nativeStatProvenance,
        startupAtAttach,
        inspectorAccess:{atConsumerProof:inspectorAtConsumerProof,final:observerRegistry.inspectorAccess()},
        opaqueAccess:observerRegistry.opaqueAccess(),
        preloadModules:[...(process._preload_modules??[])],
        nativeStatSource:Function.prototype.toString.call(original),
        nativeStatSourceHash:crypto.createHash('sha256').update(Function.prototype.toString.call(original)).digest('hex'),
        calls:calls.map(op=>{
          const root=op.promises.find(p=>p.id===op.root)
          const creator=op.promises.find(p=>p.id===root?.settled?.insidePromise)
          const checkpoints=[op.before,...op.promises.flatMap(p=>[p.init,...p.before,...(p.settled?[p.settled]:[])])]
          return {...op,rootSettledFromOwnReaction:!!creator && creator.nativeCall && root.settled.insideOperation===op.id,
            observedStartupNativeUnchanged:startupAtAttach?.startup.available===true &&
              startupAtAttach.nativeBindingUnchanged && startupAtAttach.publicSyncUnchanged &&
              startupAtAttach.publicAsyncUnchanged && startupAtAttach.prototypeUnchanged &&
              checkpoints.every(x=>x.startup?.nativeBindingUnchanged && x.startup.prototypeUnchanged),
            observedNativeUnchanged:checkpoints.every(x=>x.native.available && x.native.unchanged),
            observedConstructionFieldsAbsent:checkpoints.every(x=>x.native.constructionFieldsAbsent===true),
            observedNoOpaqueEntries:checkpoints.every(x=>x.opaque.available &&
              Object.values(x.opaque.counts).every(n=>n===0)),
            observedOnlyOwnedHooks:ownedCaptured && checkpoints.every(x=>x.observers.onlyOwnedPromiseHooks===true &&
              x.observers.pendingAsyncHookEdit===false && Object.values(x.observers.asyncHooks).every(n=>n===0)),
            creatorBefore:creator?.before??[]}
        }),
        unvalidated:['non-registry observation channels','initial preload state','native binding origin',
          'all result consumers','production integration']}
    },
    close(){stop();observerRegistry.close()},
  }
}
