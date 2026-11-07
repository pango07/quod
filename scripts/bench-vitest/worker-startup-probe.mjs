// Synchronously required by the first CJS preload. Keep this graph free of
// top-level await: application code must not run before the guards are armed.
import {inspectPromiseObservers} from './promise-observers-probe.mjs'

export function createWorkerStartup(startup) {
  const combined=process.env.QUOD_WORKER_BOUNDARY_PROBE==='1'
  const scalarObservation=process.env.QUOD_WORKER_SCALAR_OBSERVATION??'accessor'
  if(!['accessor','native-data'].includes(scalarObservation))throw Error('unknown startup scalar observation mode')
  let transport=null,prepare=null,checkpoint=null,handoffs=0,prematureMessages=0,closed=false
  const failures=[]
  const registry=inspectPromiseObservers({proxyAccess:true,hookAccess:true,compiledScripts:true,
    descriptorAccess:combined&&scalarObservation==='accessor',nativeEntries:combined,
    inspectorTiming:combined,ordinaryOwners:combined,
    nativeTransport:combined?(process.env.QUOD_WORKER_NATIVE_CAPTURE_TRANSPORT??'protocol'):'protocol',
    onDescriptorCheckpoint:combined?value=>checkpoint?.(value):null})
  function prepareWorkerMessage(message) {
    if(!prepare){prematureMessages++;return false}
    return prepare(message)
  }
  function observeWorkerMessage(message) {
    // This exact lexical binding also supplies the transport continuation
    // callback after the one-time handoff; no global bridge is published.
    if(transport)transport.received(message)
    else if(!prematureMessages)prepareWorkerMessage(message)
  }
  process.prependListener('message',observeWorkerMessage)
  registry.captureMessageObserver()
  registry.captureIpcDelivery()
  const initialDispatch=registry.messageDispatch(),initialIpc=registry.ipcDelivery()
  if(!initialDispatch.available||!initialIpc.available)failures.push('startup guards unavailable')
  function report() {
    return {...startup,installedBeforeApplicationImports:true,handoffs,prematureMessages,closed,
      available:!closed&&!failures.length&&prematureMessages===0&&handoffs===1&&registry.messageDispatch().available&&registry.ipcDelivery().available,
      failures:[...failures],completeStartupIntegrity:false,completeOwnership:false,replayEligible:false}
  }
  return Object.freeze({registry,report,
    handoff({transport:nextTransport,prepare:nextPrepare,onCheckpoint=null}={}) {
      if(closed||handoffs||typeof nextPrepare!=='function'||typeof nextTransport?.received!=='function'||typeof nextTransport?.delivered!=='function') {
        failures.push('invalid or repeated startup handoff');return false
      }
      // Refresh only the later warning wrapper. Never replace the observer,
      // reset counters, disconnect Inspector or discard earlier failures.
      registry.captureMessageWrapper()
      transport=nextTransport;prepare=nextPrepare;checkpoint=onCheckpoint;handoffs++
      return report().available
    },
    close() {
      if(closed)return
      closed=true;registry.close();process.removeListener('message',observeWorkerMessage)
      transport=null;prepare=null;checkpoint=null
    },
  })
}
