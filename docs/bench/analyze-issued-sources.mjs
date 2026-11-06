// Reproduce: node docs/bench/analyze-issued-sources.mjs /path/to/unpacked-artifact
// Diagnostic partition only; no source match grants replay or source ownership.
import fs from 'node:fs'
import path from 'node:path'
import {validateIssuedCompilationSources} from '../../scripts/bench-vitest/issued-compilation-sources.mjs'
import {blockCompilationRuns} from '../../scripts/bench-vitest/compiled-block-log.mjs'
const root=process.argv[2],result={replayEligible:false,ownershipEstablished:false,phases:{},comparisons:[]}
const key=r=>JSON.stringify([r.project,r.files]),codeKey=c=>JSON.stringify(c)
const rows=new Map()
function append(out,key,count){const last=out.at(-1);if(last?.[0]===key)last[1]+=count;else out.push([key,count])}
function multiset(runs){const m=new Map();for(const [k,n]of runs)m.set(k,(m.get(k)??0)+n);return JSON.stringify([...m].sort((a,b)=>a[0].localeCompare(b[0])))}
function firstDifference(a,b){
  const expand=r=>r.flatMap(([record,n])=>Array(n).fill(record)),x=expand(a),y=expand(b)
  const i=x.findIndex((v,i)=>v!==y[i]);if(i<0&&x.length===y.length)return null
  const at=i<0?x.length:i
  return {event:at,before:x[at]?JSON.parse(x[at]):null,after:y[at]?JSON.parse(y[at]):null}
}
for(const phase of ['before','after']){
  const totals={workers:0,anonymousEvents:0,matchedEvents:0,unmatchedEvents:0,issuedCalls:0,unmatchedSources:[]}
  const seen=new Set(),sources=new Map(),dir=path.join(root,'worker-boundary',phase)
  for(const name of fs.readdirSync(dir).filter(n=>n.endsWith('.json'))){
    const r=JSON.parse(fs.readFileSync(path.join(dir,name))),c=r.boundary.compilation,s=c.issuedSources
    validateIssuedCompilationSources(s,c)
    if(seen.has(key(r)))throw Error('duplicate worker');seen.add(key(r))
    totals.workers++
    for(const k of ['anonymousEvents','matchedEvents','unmatchedEvents','issuedCalls'])totals[k]+=s[k]
    for(const row of s.sources)if(row.unmatchedEvents){
      const k=codeKey(row.code),value=sources.get(k)??{code:row.code,events:0,workers:0,alsoIssued:row.issued.length>0}
      value.events+=row.unmatchedEvents;value.workers++;sources.set(k,value)
    }
    const unmatched=[],remainingAllKinds=[]
    let i=0,left=s.runs[0]?.[2]??0
    for(const [id,count] of blockCompilationRuns(c)){
      const script={...c.scripts[id]};delete script.count
      const record=JSON.stringify(script)
      if(script.kind!=='anonymous'){append(remainingAllKinds,record,count);continue}
      let n=count
      while(n){
        const run=s.runs[i],take=Math.min(n,left)
        if(!run[1]){append(unmatched,record,take);append(remainingAllKinds,record,take)}
        n-=take;left-=take;if(!left){i++;left=s.runs[i]?.[2]??0}
      }
    }
    if(phase==='before')rows.set(key(r),{unmatched,remainingAllKinds})
    else{
      const a=rows.get(key(r));if(!a)throw Error('unpaired worker')
      result.comparisons.push({project:r.project,files:r.files,unmatchedAnonymousEqual:JSON.stringify(a.unmatched)===JSON.stringify(unmatched),
        remainingAllKindsEqual:JSON.stringify(a.remainingAllKinds)===JSON.stringify(remainingAllKinds),
        remainingNonWasmEqual:JSON.stringify(a.remainingAllKinds.filter(([k])=>JSON.parse(k).kind!=='wasm'))===JSON.stringify(remainingAllKinds.filter(([k])=>JSON.parse(k).kind!=='wasm')),
        remainingAllKindsMultisetEqual:multiset(a.remainingAllKinds)===multiset(remainingAllKinds),
        firstDifference:firstDifference(a.remainingAllKinds,remainingAllKinds),
        beforeUnmatched:a.unmatched.reduce((n,e)=>n+e[1],0),afterUnmatched:unmatched.reduce((n,e)=>n+e[1],0)})
    }
  }
  if(phase==='after'&&(seen.size!==rows.size||[...rows.keys()].some(k=>!seen.has(k))))throw Error('worker sets differ')
  totals.unmatchedSources=[...sources.values()].sort((a,b)=>b.events-a.events)
  result.phases[phase]=totals
}
result.equalUnmatchedAnonymous=result.comparisons.filter(r=>r.unmatchedAnonymousEqual).length
result.equalRemainingAllKinds=result.comparisons.filter(r=>r.remainingAllKindsEqual).length
result.equalRemainingAllKindsMultiset=result.comparisons.filter(r=>r.remainingAllKindsMultisetEqual).length
result.equalRemainingNonWasm=result.comparisons.filter(r=>r.remainingNonWasmEqual).length
result.limitation='Exploratory exact-source partition only. Matching issued source is not ownership or permission to omit it. Remaining events preserve exact records, multiplicity and order, including cross-kind interleaving.'
console.log(JSON.stringify(result,null,2))
