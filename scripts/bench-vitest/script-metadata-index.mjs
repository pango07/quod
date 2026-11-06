// Exact script-ID lookup with interned metadata and bounded dense/range storage.
// This compresses the observer's lookup table, never compilation events.
import {types} from 'node:util'
import {scriptIdRanges} from './script-id-ranges.mjs'
const call=Function.prototype.call.bind(Function.prototype.call)
const mapGet=Map.prototype.get,mapSet=Map.prototype.set,
  mapSize=Object.getOwnPropertyDescriptor(Map.prototype,'size').get,MapType=Map
const freeze=Object.freeze,keys=Reflect.ownKeys,descriptor=Object.getOwnPropertyDescriptor,own=Object.hasOwn,
  isProxy=types.isProxy,safeInteger=Number.isSafeInteger,numberOf=Number,stringOf=String,
  floor=Math.floor,Uint32=Uint32Array,ErrorType=Error
const get=(map,key)=>call(mapGet,map,key),set=(map,key,value)=>call(mapSet,map,key,value),
  size=map=>call(mapSize,map)
const ownedMetadata=new WeakSet(),weakAdd=Function.prototype.call.bind(WeakSet.prototype.add),
  weakHas=Function.prototype.call.bind(WeakSet.prototype.has)
// Only these records have a checked four-scalar shape and an immutable,
// owned copy. Consumers must not infer this contract from Object.isFrozen.
export const isOwnedScriptMetadata=value=>weakHas(ownedMetadata,value)
export function scriptMetadataIndex({storage='dense',entryLimit=storage==='ranges'?9007199254740991:4000000,
  metadataLimit=20000,denseLimit=4194304,rangeLimit=65536,sparseLimit=65536,
  lookupLimit=storage==='ranges'?2048:0}={}) {
  if(!safeInteger(entryLimit)||entryLimit<=0||!safeInteger(metadataLimit)||metadataLimit<=0||
    !safeInteger(denseLimit)||denseLimit<=0||metadataLimit>=0xffffffff||denseLimit>4194304||
    !['dense','ranges'].includes(storage)||!safeInteger(sparseLimit)||sparseLimit<1||
    !safeInteger(lookupLimit)||lookupLimit<0||lookupLimit>65536)
    throw new ErrorType('invalid script metadata limits')
  const chunks=new MapType(),sparse=new MapType(),dictionary=new MapType(),values=new MapType(),chunkSize=4096
  // Cache only successful public lookups. Millions of newly compiled IDs
  // do not displace the small set of caller IDs read on every native entry.
  // Each value is the same immutable owned metadata object as in the index.
  const hot=new MapType()
  const ranges=storage==='ranges'?scriptIdRanges(rangeLimit):null
  const failureStages={validation:0,copy:0,lookup:0,commit:0}
  let entries=0,events=0,overflow=false,failures=0
  function denseId(id) {
    const number=numberOf(id)
    return safeInteger(number)&&number>=0&&(ranges||number<denseLimit)&&stringOf(number)===id?number:null
  }
  function lookup(id) {
    const number=denseId(id)
    return number===null?get(sparse,id):ranges?ranges.get(number):get(chunks,floor(number/chunkSize))?.[number%chunkSize]
  }
  function field(metadata,key,type,optional=false) {
    const d=descriptor(metadata,key)
    if(!d||!own(d,'value')||(typeof d.value!==type&&!(optional&&d.value===undefined)))
      throw new ErrorType('invalid script metadata')
    return d.value
  }
  return {
    set(id,metadata) {
      if(events===9007199254740991){overflow=true;return false}
      events++
      if(overflow)return false
      let stage='validation'
      try {
        if(typeof id!=='string'||!metadata||typeof metadata!=='object'||isProxy(metadata)||keys(metadata).length!==4)
          throw new ErrorType('invalid script metadata')
        const url=field(metadata,'url','string',true),language=field(metadata,'language','string'),
          isModule=field(metadata,'isModule','boolean'),hasSourceURL=field(metadata,'hasSourceURL','boolean')
        // Length prefixes preserve every UTF-16 string without consulting
        // application toJSON hooks on Array.prototype or Object.prototype.
        const key=(url===undefined?'u':'s'+url.length+':'+url)+'|'+(isModule?'1':'0')+
          (hasSourceURL?'1':'0')+'|'+language.length+':'+language
        let value=get(dictionary,key),copy
        if(value===undefined) {
          stage='copy'
          copy=freeze({url,isModule,hasSourceURL,language})
        }
        // Descriptor queries and freeze can synchronously emit Inspector
        // notifications. Commit only after those calls, then re-read every
        // decision that a nested insertion could have changed. These captured
        // Map/typed-array operations call no application JavaScript.
        stage='lookup'
        const exists=!!lookup(id),number=denseId(id)
        if(!exists&&entries>=entryLimit){overflow=true;return false}
        if(ranges&&number===null&&!exists&&size(sparse)>=sparseLimit){overflow=true;return false}
        if(overflow)return false
        value=get(dictionary,key)
        if(value===undefined&&size(dictionary)>=metadataLimit){overflow=true;return false}
        stage='commit'
        let chunk
        if(number!==null&&!ranges) {
          const chunkId=floor(number/chunkSize)
          chunk=get(chunks,chunkId)
          if(!chunk){chunk=new Uint32(chunkSize);set(chunks,chunkId,chunk)}
        }
        const fresh=value===undefined
        if(fresh)value=size(values)+1
        if(number!==null&&ranges&&!ranges.set(number,value)){overflow=true;return false}
        if(fresh) {
          weakAdd(ownedMetadata,copy)
          set(values,value,copy);set(dictionary,key,value)
        }
        if(number===null)set(sparse,id,value)
        else if(!ranges)chunk[number%chunkSize]=value
        if(!exists)entries++
        // A nested descriptor/freeze notification may have cached this ID
        // during validation. Update it only after the full index commit.
        if(lookupLimit&&get(hot,id)!==undefined)set(hot,id,get(values,value))
        return true
      }catch(error) {
        failures++;failureStages[stage]++
        throw error
      }
    },
    get(id){
      if(typeof id!=='string')return undefined
      const cached=lookupLimit?get(hot,id):undefined
      if(cached!==undefined)return cached
      const value=get(values,lookup(id))
      if(value!==undefined&&size(hot)<lookupLimit)set(hot,id,value)
      return value
    },
    has(id){return typeof id==='string'&&!!lookup(id)},
    counts:()=>({events,entries,metadata:size(dictionary),storage,...(ranges?ranges.counts():{}),chunks:size(chunks),
      denseBytes:size(chunks)*chunkSize*4,sparseEntries:size(sparse),lookupEntries:size(hot),lookupLimit,
      overflow,failures,failureStages:{...failureStages}}),
  }
}
