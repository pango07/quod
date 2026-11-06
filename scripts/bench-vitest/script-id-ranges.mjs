// Exact numeric-ID lookup. Adjacent IDs with identical interned metadata
// share an interval; gaps and overwritten IDs remain distinct. No events
// are sampled or dropped. The caller separately records every occurrence.
const FloatArray=Float64Array,floor=Math.floor,safe=Number.isSafeInteger,MapType=Map
const get=Function.prototype.call.bind(Map.prototype.get),set=Function.prototype.call.bind(Map.prototype.set)
export function scriptIdRanges(limit=65536) {
  if(!safe(limit)||limit<1)throw Error('invalid script range limit')
  const chunks=new MapType(),chunkSize=4096
  let count=0,allocated=0
  function at(index,field) {return get(chunks,floor(index/chunkSize))[index%chunkSize*3+field]}
  function put(index,lo,hi,value) {
    const id=floor(index/chunkSize),offset=index%chunkSize*3
    let chunk=get(chunks,id)
    if(!chunk){chunk=new FloatArray(chunkSize*3);set(chunks,id,chunk);allocated++}
    chunk[offset]=lo;chunk[offset+1]=hi;chunk[offset+2]=value
  }
  function leftOf(number) {
    if(!count)return -1
    if(at(count-1,0)<=number)return count-1
    let lo=0,hi=count
    while(lo<hi){const mid=floor((lo+hi)/2);if(at(mid,0)<=number)lo=mid+1;else hi=mid}
    return lo-1
  }
  return {
    get(number) {
      const index=leftOf(number)
      return index>=0&&number<=at(index,1)?at(index,2):undefined
    },
    set(number,value) {
      if(!safe(number)||number<0||!safe(value)||value<1)throw Error('invalid script range entry')
      const index=leftOf(number),exists=index>=0&&number<=at(index,1)
      if(exists&&at(index,2)===value)return true
      // The common monotonically increasing equal-metadata case extends
      // one interval without allocating or shifting an entry.
      if(!exists&&index===count-1&&index>=0&&number===at(index,1)+1&&at(index,2)===value) {
        put(index,at(index,0),number,value);return true
      }
      // Typed temporary storage keeps even inherited numeric Array setters
      // out of the commit path; no application callback may interleave here.
      const pieces=new FloatArray(9)
      let length=0
      function piece(lo,hi,id){pieces[length++]=lo;pieces[length++]=hi;pieces[length++]=id}
      let start=exists?index:index+1,remove=exists?1:0
      if(exists&&at(index,0)<number)piece(at(index,0),number-1,at(index,2))
      piece(number,number,value)
      if(exists&&number<at(index,1))piece(number+1,at(index,1),at(index,2))
      const last=length-3,right=start+remove
      if(start>0&&at(start-1,1)+1===pieces[0]&&at(start-1,2)===pieces[2]) {
        pieces[0]=at(start-1,0);start--;remove++
      }
      if(right<count&&pieces[last+1]+1===at(right,0)&&pieces[last+2]===at(right,2)) {
        pieces[last+1]=at(right,1);remove++
      }
      const next=count-remove+length/3
      if(next>limit)return false
      const delta=length/3-remove
      if(delta>0)for(let i=count-1;i>=start+remove;i--)put(i+delta,at(i,0),at(i,1),at(i,2))
      else if(delta<0)for(let i=start+remove;i<count;i++)put(i+delta,at(i,0),at(i,1),at(i,2))
      for(let i=0;i<length;i+=3)put(start+i/3,pieces[i],pieces[i+1],pieces[i+2])
      count=next;return true
    },
    counts:()=>({ranges:count,rangeLimit:limit,rangeStorageBytes:allocated*chunkSize*24}),
  }
}
