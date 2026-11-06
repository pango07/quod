// Diagnostic only. Observe native promise provenance without adding a
// .then callback, replacing Stats, or granting a narrower dependency key.
import fs from 'node:fs'
import {promiseHooks} from 'node:v8'
import path from 'node:path'
import crypto from 'node:crypto'

export function observeAsyncStat(project) {
  const original=fs.promises.stat, NativeStats=fs.Stats
  const descriptors=Object.getOwnPropertyDescriptors, descriptor=Object.getOwnPropertyDescriptor
  const getPrototypeOf=Object.getPrototypeOf, ownKeys=Reflect.ownKeys, apply=Reflect.apply
  const prototypes=[]
  for(let p=NativeStats.prototype;p;p=getPrototypeOf(p)) prototypes.push({object:p,parent:getPrototypeOf(p),shape:descriptors(p)})
  const pristine=()=>descriptor(fs,'Stats')?.value===NativeStats &&
    descriptor(NativeStats,'prototype')?.value===prototypes[0].object && prototypes.every(({object,parent,shape})=>
      getPrototypeOf(object)===parent && ownKeys(object).length===ownKeys(shape).length && ownKeys(shape).every(k=>{
        const a=shape[k],b=descriptor(object,k)
        return b && a.value===b.value && a.get===b.get && a.set===b.set && a.writable===b.writable && a.configurable===b.configurable && a.enumerable===b.enumerable
      }))
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
  const calls=[], promises=new WeakMap(), stack=[]
  let scope=null, active=null, nextPromise=0, eventMismatch=false
  const stop=promiseHooks.createHook({
    init(p,parent) {
      if(!scope) return
      const state={id:++nextPromise,op:scope.id,parent:promises.get(parent)?.id??null,before:[],settled:null}
      promises.set(p,state);scope.promises.push(state)
    },
    before(p) {
      stack.push({promise:p,prior:active});active=p
      const state=promises.get(p)
      if(state) state.before.push({pristine:pristine(),asyncHooks:counts()})
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
      state.settled={insidePromise:creator?.id??null,insideOperation:creator?.op??null,pristine:pristine(),asyncHooks:counts()}
    },
  })
  fs.promises.stat=function stat(...args) {
    const op={id:calls.length+1,path:typeof args[0]==='string'?path.relative(project,path.resolve(args[0])):'<non-string>',
      stack:new Error().stack.split('\n').slice(2,10).join('\n'),before:{pristine:pristine(),asyncHooks:counts()},promises:[],root:null}
    calls.push(op)
    const prior=scope;scope=op
    try {
      const result=apply(original,this,args)
      op.root=promises.get(result)?.id??null
      // Return the very same native promise. No fulfillment observer and
      // no additional promise reaction are inserted by this diagnostic.
      return result
    } finally {scope=prior}
  }
  return {
    report() {
      return {replayEligible:false,eventMismatch,
        preloadModules:[...(process._preload_modules??[])],
        nativeStatSource:Function.prototype.toString.call(original),
        nativeStatSourceHash:crypto.createHash('sha256').update(Function.prototype.toString.call(original)).digest('hex'),
        calls:calls.map(op=>{
          const root=op.promises.find(p=>p.id===op.root)
          const creator=op.promises.find(p=>p.id===root?.settled?.insidePromise)
          return {...op,rootSettledFromOwnReaction:!!creator && root.settled.insideOperation===op.id,
            creatorBefore:creator?.before??[]}
        }),
        unvalidated:['other promise observers','preload effects','native bindings','all result consumers','outcome and error projection']}
    },
    close(){stop()},
  }
}
