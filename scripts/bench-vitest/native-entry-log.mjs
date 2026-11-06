// Lossless compression of the diagnostic's grouped native-entry records.
// Full frame fields and their order stay significant. This does not extend
// the captured stack depth or native API coverage, or authorize replay.
import {isOwnedScriptMetadata} from './script-metadata-index.mjs'
const knownFrameKeys=new WeakMap(),weakGet=Function.prototype.call.bind(WeakMap.prototype.get),
  weakSet=Function.prototype.call.bind(WeakMap.prototype.set),freeze=Object.freeze,
  stringify=JSON.stringify,parse=JSON.parse,safeInteger=Number.isSafeInteger,
  mapGet=Function.prototype.call.bind(Map.prototype.get),mapSet=Function.prototype.call.bind(Map.prototype.set),
  mapSize=Function.prototype.call.bind(Object.getOwnPropertyDescriptor(Map.prototype,'size').get)

// Map the same five native call-site fields as the original observer. Only
// scriptMetadataIndex-owned scalar records can participate in this cache.
// Unknown/mutable metadata and capacity exhaustion use fresh mapped frames.
export function nativeFrameMapper(scripts,{cacheLimit=20000}={}) {
  if(!safeInteger(cacheLimit)||cacheLimit<0||cacheLimit>65536)throw Error('invalid native frame cache limit')
  const cache=new Map(),metadataIds=new WeakMap()
  let nextMetadataId=1
  function mapped(f,script) {
    return {name:f.name,location:{scriptId:f.scriptId,lineNumber:f.line-1,columnNumber:f.column-1},script}
  }
  function serialized(f,script) {
    // Serialize only checked primitives. Inherited toJSON hooks cannot
    // change an owned frame's key after its first observation.
    const metadata=script===null?'null':'{'+(script.url===undefined?'':'"url":'+stringify(script.url)+',')+
      '"isModule":'+script.isModule+',"hasSourceURL":'+script.hasSourceURL+',"language":'+stringify(script.language)+'}'
    return '{"name":'+stringify(f.name)+',"location":{"scriptId":'+stringify(f.scriptId)+
      ',"lineNumber":'+(f.line-1)+',"columnNumber":'+(f.column-1)+'},"script":'+metadata+'}'
  }
  return {
    map(stack) {
      return stack.map(f=>{
        const script=scripts.get(f.scriptId)??null
        if(!cacheLimit||typeof f.name!=='string'||typeof f.scriptId!=='string'||
          !safeInteger(f.line)||!safeInteger(f.column)||script!==null&&!isOwnedScriptMetadata(script))return mapped(f,script)
        let metadataId=script===null?0:weakGet(metadataIds,script)
        if(metadataId===undefined) {
          if(mapSize(cache)>=cacheLimit||nextMetadataId>cacheLimit)return mapped(f,script)
          metadataId=nextMetadataId++;weakSet(metadataIds,script,metadataId)
        }
        const key=f.name.length+':'+f.name+f.scriptId.length+':'+f.scriptId+'|'+(f.line-1)+','+(f.column-1)+','+metadataId
        let frame=mapGet(cache,key)
        if(frame)return frame
        frame=mapped(f,script)
        if(mapSize(cache)<cacheLimit) {
          freeze(frame.location);freeze(frame)
          // Freeze can synchronously emit Inspector notifications. Recheck
          // the cache after it; a nested mapper must not exceed this bound.
          const nested=mapGet(cache,key)
          if(nested)return nested
          if(mapSize(cache)<cacheLimit) {
            weakSet(knownFrameKeys,frame,serialized(f,script));mapSet(cache,key,frame)
          }
        }
        return frame
      })
    },
    counts:()=>({entries:mapSize(cache),limit:cacheLimit,metadataIdentities:nextMetadataId-1}),
  }
}

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
        const key=weakGet(knownFrameKeys,frame)??stringify(frame)
        let id=frameIndex.get(key)
        if(id===undefined) {
          if(frames.length>=frameLimit){overflow=true;return}
          id=frames.length;frames.push(parse(key));frameIndex.set(key,id)
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
