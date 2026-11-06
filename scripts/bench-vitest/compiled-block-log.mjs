// Lossless second-level compression for alternating compilation records.
// A block contains consecutive [record ID, count] runs. Exact equal blocks
// share storage; the outer run index retains their order and multiplicity.
// Limits bound stored pairs, not the number of events represented by them.
const FloatArray=Float64Array,MapType=Map,stringify=JSON.stringify,parse=JSON.parse,
  safe=Number.isSafeInteger,floor=Math.floor
const mapGet=Function.prototype.call.bind(Map.prototype.get),mapSet=Function.prototype.call.bind(Map.prototype.set)
export function blockCompiledScriptLog({recordLimit=20000,pairLimit=4000000,blockSize=256,ownedRecordKey=null}={}) {
  if(!safe(recordLimit)||recordLimit<1||!safe(pairLimit)||pairLimit<1||
    !safe(blockSize)||blockSize<1||blockSize>4096||
    ownedRecordKey!==null&&typeof ownedRecordKey!=='function')throw Error('invalid compilation block limits')
  const scriptKeys=[],counts=[],recordIds=new MapType(),blocks=[],blockIds=new MapType(),chunks=[],chunkRuns=4096
  let tail=[],events=0,runs=0,overflow=false,storedRuns=0,blockRunCount=0
  function lastBlockRun() {
    const index=blockRunCount-1
    return index<0?null:{chunk:chunks[floor(index/chunkRuns)],offset:index%chunkRuns*2}
  }
  function flush() {
    // These are owned safe integers. Avoid an inherited Array.toJSON when
    // interning a block; delimiters make every ID/count sequence unambiguous.
    let key=''
    for(let i=0;i<tail.length;i++)key+=tail[i]+','
    const known=mapGet(blockIds,key),id=known??blocks.length,last=lastBlockRun(),
      merge=last&&last.chunk[last.offset]===id,nextStored=storedRuns+(known===undefined?blockSize:0),
      nextOuter=blockRunCount+(merge?0:1)
    if(nextStored+nextOuter>pairLimit){overflow=true;return false}
    if(known===undefined) {
      blocks.push(new FloatArray(tail));mapSet(blockIds,key,id);storedRuns=nextStored
    }
    if(merge)last.chunk[last.offset+1]++
    else {
      const offset=blockRunCount%chunkRuns,index=floor(blockRunCount/chunkRuns)
      if(offset===0)chunks.push(new FloatArray(chunkRuns*2))
      chunks[index][offset*2]=id;chunks[index][offset*2+1]=1;blockRunCount++
    }
    tail=[];return true
  }
  function outerRuns() {
    return Array.from({length:blockRunCount},(_,i)=>{
      const chunk=chunks[floor(i/chunkRuns)],offset=i%chunkRuns*2
      return [chunk[offset],chunk[offset+1]]
    })
  }
  return {
    append(record) {
      if(events===Number.MAX_SAFE_INTEGER){overflow=true;return}
      events++
      if(overflow)return
      const key=ownedRecordKey?.(record)??stringify(record)
      let id=mapGet(recordIds,key)
      if(id===undefined) {
        if(scriptKeys.length>=recordLimit){overflow=true;return}
        id=scriptKeys.length;mapSet(recordIds,key,id);scriptKeys.push(key);counts.push(0)
      }
      if(tail.length&&tail[tail.length-2]===id)tail[tail.length-1]++
      else {
        if(tail.length===blockSize*2&&!flush())return
        if(storedRuns+blockRunCount+tail.length/2+1>pairLimit){overflow=true;return}
        tail.push(id,1);runs++
      }
      counts[id]++
    },
    report:()=>({encoding:'block-dictionary-v1',blockSize,events,overflow,
      scripts:scriptKeys.map((key,id)=>({...parse(key),count:counts[id]})),blocks:blocks.map(block=>Array.from(block)),blockRuns:outerRuns(),tail:[...tail]}),
    counts:()=>({events,runs,records:scriptKeys.length,overflow,runStorage:'block-dictionary-f64',
      blocks:blocks.length,blockRuns:blockRunCount,storedPairs:storedRuns+blockRunCount+tail.length/2,pairLimit,
      runStorageBytes:storedRuns*16+chunks.length*chunkRuns*16+tail.length*8}),
  }
}

// Iterate exact recorded runs without materializing the expanded event list.
// Callers must validate an external report before trusting its references.
export function* blockCompilationRuns(report) {
  for(const [id,repetitions] of report.blockRuns) {
    const block=report.blocks[id]
    for(let n=0;n<repetitions;n++)for(let i=0;i<block.length;i+=2)yield [block[i],block[i+1]]
  }
  for(let i=0;i<report.tail.length;i+=2)yield [report.tail[i],report.tail[i+1]]
}

export function validateBlockCompilationCounts(report) {
  if(!safe(report.blockSize)||report.blockSize<1||report.blockSize>4096||
    !Array.isArray(report.blocks)||!Array.isArray(report.blockRuns)||!Array.isArray(report.tail)||
    report.tail.length>report.blockSize*2)throw Error('invalid compilation blocks')
  const totals=report.scripts.map(()=>0),uses=report.blocks.map(()=>0)
  function add(a,b){const sum=a+b;if(!safe(sum))throw Error('compilation count overflow');return sum}
  function addPairs(pairs,repetitions) {
    if(!Array.isArray(pairs)||pairs.length%2)throw Error('invalid compilation block pairs')
    for(let i=0;i<pairs.length;i+=2) {
      const id=pairs[i],count=pairs[i+1]
      if(!safe(id)||id<0||id>=totals.length||!safe(count)||count<1||!safe(count*repetitions))
        throw Error('invalid compilation block reference')
      totals[id]=add(totals[id],count*repetitions)
    }
  }
  for(const run of report.blockRuns) {
    if(!Array.isArray(run)||run.length!==2||!safe(run[0])||run[0]<0||run[0]>=uses.length||!safe(run[1])||run[1]<1)
      throw Error('invalid compilation block run')
    uses[run[0]]=add(uses[run[0]],run[1])
  }
  for(let i=0;i<report.blocks.length;i++) {
    if(!Array.isArray(report.blocks[i])||report.blocks[i].length!==report.blockSize*2||uses[i]===0)
      throw Error('invalid compilation stored block')
    addPairs(report.blocks[i],uses[i])
  }
  addPairs(report.tail,1)
  if(totals.some((n,i)=>n!==report.scripts[i].count))throw Error('compilation block count differs')
}
