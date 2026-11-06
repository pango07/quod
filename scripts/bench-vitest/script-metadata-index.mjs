// Exact script-ID lookup with interned metadata and bounded dense storage.
// This compresses the observer's lookup table, never compilation events.
export function scriptMetadataIndex({entryLimit=4000000,metadataLimit=20000,denseLimit=4194304}={}) {
  if(![entryLimit,metadataLimit,denseLimit].every(n=>Number.isSafeInteger(n)&&n>0)||metadataLimit>=0xffffffff||denseLimit>4194304)
    throw Error('invalid script metadata limits')
  const chunks=new Map(),sparse=new Map(),dictionary=new Map(),values=[undefined],chunkSize=4096
  let entries=0,events=0,overflow=false
  function denseId(id) {
    const number=Number(id)
    return Number.isSafeInteger(number)&&number>=0&&number<denseLimit&&String(number)===id?number:null
  }
  function lookup(id) {
    const number=denseId(id)
    return number===null?sparse.get(id):chunks.get(Math.floor(number/chunkSize))?.[number%chunkSize]
  }
  return {
    set(id,metadata) {
      if(events===Number.MAX_SAFE_INTEGER){overflow=true;return false}
      events++
      if(overflow)return false
      if(typeof id!=='string'||!metadata||typeof metadata.url!=='string'||typeof metadata.language!=='string'||
        typeof metadata.isModule!=='boolean'||typeof metadata.hasSourceURL!=='boolean'||
        Reflect.ownKeys(metadata).length!==4||!['url','language','isModule','hasSourceURL'].every(key=>Object.hasOwn(metadata,key)))
        throw Error('invalid script metadata')
      const exists=!!lookup(id)
      if(!exists&&entries>=entryLimit){overflow=true;return false}
      const key=JSON.stringify([metadata.url,metadata.isModule,metadata.hasSourceURL,metadata.language])
      let value=dictionary.get(key)
      if(value===undefined) {
        if(dictionary.size>=metadataLimit){overflow=true;return false}
        value=values.length;dictionary.set(key,value)
        values.push(Object.freeze({...metadata}))
      }
      const number=denseId(id)
      if(number===null)sparse.set(id,value)
      else {
        const chunkId=Math.floor(number/chunkSize)
        let chunk=chunks.get(chunkId)
        if(!chunk){chunk=new Uint32Array(chunkSize);chunks.set(chunkId,chunk)}
        chunk[number%chunkSize]=value
      }
      if(!exists)entries++
      return true
    },
    get(id){return typeof id==='string'?values[lookup(id)]:undefined},
    has(id){return typeof id==='string'&&!!lookup(id)},
    counts:()=>({events,entries,metadata:dictionary.size,chunks:chunks.size,
      denseBytes:chunks.size*chunkSize*4,sparseEntries:sparse.size,overflow}),
  }
}
