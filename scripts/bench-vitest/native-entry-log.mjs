// Lossless compression of the diagnostic's grouped native-entry records.
// Full frame fields and their order stay significant. This does not extend
// the captured stack depth or native API coverage, or authorize replay.
export function nativeEntryLog({entryLimit=100000,frameLimit=20000}={}) {
  if(![entryLimit,frameLimit].every(n=>Number.isSafeInteger(n)&&n>0))throw Error('invalid native log limits')
  const entries=new Map(),frames=[],frameIndex=new Map()
  let calls=0,overflow=false
  return {
    append(apis,stack,argument) {
      if(calls===Number.MAX_SAFE_INTEGER){overflow=true;return}
      calls++
      if(overflow)return
      const frameIds=[]
      for(const frame of stack) {
        const key=JSON.stringify(frame)
        let id=frameIndex.get(key)
        if(id===undefined) {
          if(frames.length>=frameLimit){overflow=true;return}
          id=frames.length;frames.push(JSON.parse(key));frameIndex.set(key,id)
        }
        frameIds.push(id)
      }
      const key=JSON.stringify([apis,frameIds,argument])
      const row=entries.get(key)
      if(row)row.count++
      else if(entries.size<entryLimit)entries.set(key,{entries:[...apis],frameIds,argument,count:1})
      else overflow=true
    },
    counts:()=>({calls,entries:entries.size,frames:frames.length,overflow}),
    report:()=>({frameEncoding:'dictionary-v1',calls,overflow,frames:[...frames],
      entries:[...entries.values()].map(e=>({...e,frameIds:[...e.frameIds]}))}),
  }
}

export function nativeEntryFrames(report,entry) {
  if(report.frameEncoding!=='dictionary-v1'||!Array.isArray(report.frames)||!Array.isArray(entry.frameIds))throw Error('invalid native frame dictionary')
  return entry.frameIds.map(id=>{
    if(!Number.isSafeInteger(id)||id<0||id>=report.frames.length)throw Error('invalid native frame reference')
    return report.frames[id]
  })
}

export function validateNativeEntryLog(report) {
  if(!report||report.overflow!==false||report.frameEncoding!=='dictionary-v1'||
    !Number.isSafeInteger(report.calls)||report.calls<0||!Array.isArray(report.entries)||!Array.isArray(report.frames))throw Error('incomplete native entry log')
  let total=0
  for(const entry of report.entries) {
    if(!Number.isSafeInteger(entry.count)||entry.count<1)throw Error('invalid native entry count')
    total+=entry.count
    nativeEntryFrames(report,entry)
  }
  if(total!==report.calls)throw Error('native entry count differs')
}
