// Diagnostic only. Native data callbacks retain ordinary descriptors while
// observing value reads; callback loss rejects incomplete evidence. Earlier
// copies and native/debugger capability integrity remain unproven.
import path from 'node:path'
import {createRequire} from 'node:module'
import {getCallSites} from 'node:util'
const require=createRequire(import.meta.url)

export function observeNativeScalarProperty(target,key,{sourceForScript=null,onOperation=null,ownerGuard=null}={}) {
  const events=new Map(),gaps=new Set()
  let observer=null,overflow=false,ownerGuardPassed=false
  function onRead() {
    try {
      const sites=getCallSites(100,{sourceMap:false}).slice(1).map(s=>({scriptId:s.scriptId,
        script:s.scriptName,function:s.functionName,line:s.lineNumber,column:s.columnNumber}))
      const sources=sourceForScript?sites.map(s=>sourceForScript(s.scriptId)):null
      const source=sources?.[0]??null
      if(!sites.length||!sources||sources.some(s=>!s?.available))gaps.add('native scalar caller source unavailable')
      const operation='get',id=JSON.stringify([operation,sites])
      if(events.has(id))events.get(id).count++
      else if(events.size<10000)events.set(id,{operation,sites,source,sources,count:1})
      else overflow=true
      if(onOperation)onOperation({operation,sites,source,sources})
    }catch{gaps.add('native scalar read observation unavailable')}
  }
  try {
    const addon=process.env.QUOD_NATIVE_CAPTURE_ADDON
    if(process.versions.node!=='24.18.0'||!addon||!path.isAbsolute(addon))throw Error('native scalar requires pinned helper')
    if(ownerGuard&&ownerGuard(target)!==true)throw Error('unsupported native scalar owner')
    // The native factory validates own data and rejects native callbacks
    // before reading the initial field. No JS descriptor read precedes it.
    observer=require(addon).createScalarReadObserver(target,key,onRead)
    if(!observer||typeof observer.report!=='function'||typeof observer.currentValue!=='function')throw Error('native scalar setup unavailable')
    ownerGuardPassed=true
  }catch{gaps.add('native scalar setup failed')}
  function status() {
    const native=observer?.report()??null
    const recordedReads=[...events.values()].reduce((n,e)=>n+e.count,0)
    if(native&&(!native.intact||!native.available))gaps.add('native scalar callback unavailable')
    if(native&&native.calls!==recordedReads)gaps.add('native scalar read accounting mismatch')
    return {native,recordedReads}
  }
  return {value:target,
    currentValue() {
      const {native}=status()
      return native?.available&&!overflow&&!gaps.size?observer.currentValue():{available:false}
    },
    report() {
      const {native,recordedReads}=status()
      return {replayEligible:false,completeOwnership:false,observation:'native-data',
        installed:native?.installed??false,intact:native?.intact??false,overflow,
        ownerGuardRequired:true,ownerGuardPassed,ordinaryDescriptorPreserved:true,nativeCallbackObservable:true,
        earlierCopiesExcluded:false,nativeReadsExcluded:false,native,recordedReads,
        events:[...events.values()],gaps:[...gaps],
        limitations:['A native callback is observable through debugger/native/heap capabilities',
          'Values copied before installation and initial runtime integrity are not certified',
          'Writes or redefinition that remove the callback invalidate observation',
          'Frozen owners retain a closed native callback after cleanup']}
    },
    close(){observer?.close()},
  }
}
