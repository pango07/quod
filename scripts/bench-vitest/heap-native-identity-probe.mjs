// Diagnostic only. Compare actual V8 object identities without compiling a
// JavaScript comparison for every native entry. Tracking affects heap/GC
// behavior; it is not a production equivalence certificate. No heap snapshot
// or heap contents are requested, retained, printed, or uploaded.
export function observeHeapNativeIdentity(post) {
  let active=false,failure=null,comparisons=0,unknown=0
  try {
    post('HeapProfiler.enable')
    post('HeapProfiler.startTrackingHeapObjects',{trackAllocations:false})
    active=true
  }catch(e){failure=e.message}
  function id(objectId) {
    if(typeof objectId!=='string')throw Error('missing native identity handle')
    const value=post('HeapProfiler.getHeapObjectId',{objectId}).heapSnapshotObjectId
    // V8 returns zero for an object with no assigned ID. Two such results
    // never establish equality. Do not cache IDs across profiler resets.
    if(typeof value!=='string'||!/^\d+$/.test(value)||BigInt(value)===0n)throw Error('native heap identity unavailable')
    return value
  }
  return {
    compare(left,right) {
      comparisons++
      try {
        if(!active||failure)throw Error(failure??'native heap identity closed')
        return {available:true,same:id(left)===id(right)}
      }catch(e){unknown++;return {available:false,reason:e.message}}
    },
    report:()=>({available:active&&!failure,comparisons,unknown,reason:failure,replayEligible:false,
      mode:'heap-object-id',snapshotsRequested:0,
      limitation:'Heap tracking and garbage collection can be observable; this comparison does not prove complete input ownership'}),
    close() {
      // stopTrackingHeapObjects requests a heap snapshot. disable stops the
      // tracking timer and clears IDs without requesting snapshot contents.
      try{post('HeapProfiler.disable')}catch(e){failure=e.message}
      active=false
    }
  }
}
