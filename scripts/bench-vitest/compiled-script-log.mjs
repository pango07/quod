// Lossless run-length encoding of the existing compiled-script records.
// Adjacent equal records merge; ordering, multiplicity and every field in
// each record remain significant. This does not improve Inspector coverage.
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
