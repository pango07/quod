// Lossless run-length encoding of the existing compiled-script records.
// Adjacent equal records merge; ordering, multiplicity and every field in
// each record remain significant. This does not improve Inspector coverage.
const RunArray=Float64Array,ArrayType=Array,floor=Math.floor
export function compiledScriptLog(limit=20000) {
  if(!Number.isSafeInteger(limit)||limit<1)throw Error('invalid compilation run limit')
  const scripts=[]
  let events=0,overflow=false,lastKey=null
  return {
    append(record) {
      if(events===Number.MAX_SAFE_INTEGER){overflow=true;return}
      events++
      if(overflow)return
      const key=JSON.stringify(record)
      if(key===lastKey){scripts[scripts.length-1].count++;return}
      if(scripts.length>=limit){overflow=true;return}
      scripts.push({...record,count:1});lastKey=key
    },
    report:()=>({encoding:'rle-v1',events,overflow,scripts:scripts.map(row=>({...row}))}),
    counts:()=>({events,runs:scripts.length,overflow}),
  }
}

// Alternating diagnostic conditions and cleanup scripts defeat adjacent
// RLE. Store each complete record once, plus an ordered run index. Counts
// on dictionary rows are totals; `runs` preserves the exact event order.
export function dictionaryCompiledScriptLog(recordLimit=20000,runLimit=1000000) {
  if(!Number.isSafeInteger(recordLimit)||recordLimit<1||!Number.isSafeInteger(runLimit)||runLimit<1)throw Error('invalid compilation dictionary limit')
  const scripts=[],chunks=[],ids=new Map(),chunkRuns=4096
  let events=0,overflow=false,runCount=0
  // Float64 holds every safe-integer ID and count exactly. Keep dense chunks
  // while observing; materialize the existing JSON tuples only for a report.
  // No event, record field, occurrence count, or ordering is discarded.
  function appendRun(id) {
    const offset=runCount%chunkRuns,index=floor(runCount/chunkRuns)
    if(offset===0)chunks.push(new RunArray(chunkRuns*2))
    chunks[index][offset*2]=id;chunks[index][offset*2+1]=1;runCount++
  }
  function reportRuns() {
    const result=new ArrayType(runCount)
    for(let i=0;i<runCount;i++) {
      const chunk=chunks[floor(i/chunkRuns)],offset=i%chunkRuns*2
      result[i]=[chunk[offset],chunk[offset+1]]
    }
    return result
  }
  return {
    append(record) {
      if(events===Number.MAX_SAFE_INTEGER){overflow=true;return}
      events++
      if(overflow)return
      const key=JSON.stringify(record)
      let id=ids.get(key)
      if(id===undefined) {
        if(scripts.length>=recordLimit){overflow=true;return}
        id=scripts.length;ids.set(key,id);scripts.push({...record,count:0})
      }
      const lastIndex=runCount-1,lastChunk=runCount?chunks[floor(lastIndex/chunkRuns)]:null,
        lastOffset=lastIndex%chunkRuns*2
      if(lastChunk&&lastChunk[lastOffset]===id)lastChunk[lastOffset+1]++
      else {
        if(runCount>=runLimit){overflow=true;return}
        appendRun(id)
      }
      scripts[id].count++
    },
    report:()=>({encoding:'dictionary-v1',events,overflow,scripts:scripts.map(row=>({...row})),runs:reportRuns()}),
    counts:()=>({events,runs:runCount,records:scripts.length,overflow,
      runStorage:'chunked-f64',runStorageBytes:chunks.length*chunkRuns*16}),
  }
}

export function validateCompilationCounts(report) {
  if(!report||report.overflow!==false||!['rle-v1','dictionary-v1'].includes(report.encoding)||!Number.isSafeInteger(report.events)||report.events<0||!Array.isArray(report.scripts))throw Error('invalid compilation log')
  if(report.scripts.some(s=>!Number.isSafeInteger(s.count)||s.count<1)||
    report.scripts.reduce((n,s)=>n+s.count,0)!==report.events)throw Error('compilation event count differs')
  if(report.encoding==='dictionary-v1') {
    if(!Array.isArray(report.runs))throw Error('compilation run index unavailable')
    const counts=report.scripts.map(()=>0)
    for(const run of report.runs) {
      if(!Array.isArray(run)||run.length!==2||!Number.isSafeInteger(run[0])||run[0]<0||run[0]>=counts.length||
        !Number.isSafeInteger(run[1])||run[1]<1)throw Error('invalid compilation run index')
      counts[run[0]]+=run[1]
    }
    if(counts.some((n,i)=>n!==report.scripts[i].count))throw Error('compilation dictionary count differs')
  }
}

export function compilationSequence(report) {
  return {encoding:report.encoding,scripts:report.scripts,...(report.encoding==='dictionary-v1'?{runs:report.runs}:{})}
}
