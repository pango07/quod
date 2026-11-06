// Diagnostic only. Preserve the original object while observing a scalar
// property through its existing aliases. The descriptor changes, earlier
// copies remain invisible, and native descriptor reads are not excluded.
import {types,getCallSites} from 'node:util'
const isProxy=types.isProxy

const descriptor=Object.getOwnPropertyDescriptor,define=Object.defineProperty
const scalar=v=>v===null||!['object','function'].includes(typeof v)
export function observeScalarProperty(target,key,{sourceForScript=null,onOperation=null}={}) {
  const events=new Map(),gaps=new Set()
  let installed=false,overflow=false,original,value,get,set
  function note(operation) {
    try {
      const sites=getCallSites(8,{sourceMap:false}).slice(2).map(s=>({scriptId:s.scriptId,
        script:s.scriptName,function:s.functionName,line:s.lineNumber,column:s.columnNumber}))
      const source=sourceForScript?sourceForScript(sites[0]?.scriptId):null
      if(sourceForScript&&!source?.available)gaps.add('scalar reader source unavailable')
      const id=JSON.stringify([operation,sites])
      if(events.has(id))events.get(id).count++
      else if(events.size<10000)events.set(id,{operation,sites,source,count:1})
      else overflow=true
      if(onOperation)onOperation({operation,sites,source})
    }catch{gaps.add('scalar read observation unavailable')}
  }
  try {
    if(!target||typeof target!=='object'||isProxy(target))throw Error('unsupported scalar owner')
    if(typeof key!=='string')throw Error('unsupported scalar key')
    original=descriptor(target,key)
    if(!original||!Object.hasOwn(original,'value')||!original.configurable||!original.writable)throw Error('unsupported scalar descriptor')
    value=original.value
    if(!scalar(value))throw Error('property is not scalar')
    get=function(){note('get');return value}
    set=function(next){
      note('set')
      if(this!==target) {
        // An accessor cannot preserve every Reflect.set return value for
        // alternate/frozen receivers. Keep that case explicitly unsupported.
        gaps.add('alternate scalar write receiver')
        define(this,key,{value:next,writable:true,enumerable:true,configurable:true})
        return
      }
      value=next
      if(!scalar(next))gaps.add('scalar replaced by object')
    }
    define(target,key,{get,set,enumerable:original.enumerable,configurable:original.configurable})
    installed=true
  }catch(e){gaps.add(e.message)}
  const intact=()=>{
    if(!installed)return false
    const d=descriptor(target,key)
    return d?.get===get&&d?.set===set&&d?.enumerable===original.enumerable&&d?.configurable===original.configurable
  }
  return {value:target,
    // Read our own closed-over value, never an application accessor. Used
    // only to reconstruct diagnostic fingerprints after instrumentation.
    currentValue:()=>intact()?{available:true,value}:{available:false},
    report:()=>({replayEligible:false,completeOwnership:false,installed,intact:intact(),overflow,
      descriptorObservable:true,earlierCopiesExcluded:false,nativeReadsExcluded:false,
      events:[...events.values()],gaps:[...gaps],
      limitations:['The data property becomes an observable accessor',
        'Values copied before installation and intermediate descriptor replacement can escape observation',
        'Native descriptor access and arbitrary alternate write receivers are not certified']}),
    close(){if(intact())define(target,key,{...original,value});installed=false}}
}
