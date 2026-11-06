// Diagnostic only. Keep the original configuration identity when direct
// native scalar observation is selected. No absence-of-read result grants
// replay authority, and unrelated configuration access is not observed.
import {observeScalarProperty} from './scalar-property-probe.mjs'
const descriptor=Object.getOwnPropertyDescriptor,own=Object.hasOwn

export function observeStartConfiguration(config,{observation,sourceForScript,ownerGuard,observeConfig}={}) {
  if(!['accessor','native-data'].includes(observation))throw Error('unknown start configuration observation')
  const sequence=descriptor(config,'sequence')
  const seed=observeScalarProperty(sequence&&own(sequence,'value')?sequence.value:null,'seed',
    {sourceForScript,ownerGuard,observation})
  if(observation==='native-data')return {
    value:config,seed,
    report:()=>({observation:'unchanged-object',replayEligible:false,completeOwnership:false,
      proxyObservable:false,configurationAccessObserved:false,events:null,overflow:false,
      limitation:'Only the separate seed scalar observer records reads; other configuration accesses are not observed'}),
  }
  try {
    const wrapped=observeConfig(config)
    return {value:wrapped.value,seed,report:()=>({...wrapped.report(),observation:'proxy',configurationAccessObserved:true})}
  }catch(error){seed.close();throw error}
}
