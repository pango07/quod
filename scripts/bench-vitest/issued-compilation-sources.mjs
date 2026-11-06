// Exact source matching for diagnostics. A script can share source with an
// Inspector command without having been created by that command. This records
// that distinction and never confers ownership, equivalence or replay rights.
import {createHash} from 'node:crypto'
import {validateCompilationCounts} from './compiled-script-log.mjs'
import {blockCompilationRuns} from './compiled-block-log.mjs'

const supported=new Set(['Runtime.evaluate','Runtime.callFunctionOn','Debugger.evaluateOnCallFrame','Debugger.setBreakpointOnFunctionCall'])
const mapGet=Function.prototype.call.bind(Map.prototype.get),mapSet=Function.prototype.call.bind(Map.prototype.set)
const sourceCode=source=>({encoding:'utf16le',bytes:source.length*2,sha256:createHash('sha256').update(source,'utf16le').digest('hex')})
const codeKey=code=>`${code.encoding}:${code.bytes}:${code.sha256}`

export function issuedCompilationSources({sourceLimit=4096,runLimit=20000,byteLimit=16*1024*1024}={}) {
  if(![sourceLimit,runLimit,byteLimit].every(n=>Number.isSafeInteger(n)&&n>0))throw Error('invalid issued-source limits')
  const ids=new Map(),textIds=new Map(),sources=[],runs=[]
  let overflow=false,issuedCalls=0,anonymousEvents=0,matchedEvents=0,unmatchedEvents=0,retainedBytes=0
  function rowFor(code) {
    const key=codeKey(code),existing=mapGet(ids,key)
    if(existing!==undefined)return existing
    if(sources.length>=sourceLimit){overflow=true;return null}
    const id=sources.length
    sources.push({code:{...code},issued:[],matchedEvents:0,unmatchedEvents:0});mapSet(ids,key,id)
    return id
  }
  return {
    issue(method,source) {
      if(!supported.has(method))throw Error('unsupported Inspector source command')
      if(typeof source!=='string')throw Error('issued source must be a string')
      issuedCalls++
      if(overflow)return
      let id=mapGet(textIds,source)
      if(id===undefined) {
        if(retainedBytes+source.length*2>byteLimit){overflow=true;return}
        id=rowFor(sourceCode(source));if(id===null)return
        mapSet(textIds,source,id);retainedBytes+=source.length*2
      }
      const row=sources[id],origin=row.issued.find(v=>v.method===method)
      if(origin)origin.calls++
      else row.issued.push({method,calls:1})
    },
    observe(record) {
      if(record.kind!=='anonymous')return
      anonymousEvents++
      if(overflow)return
      if(record.code?.encoding!=='utf16le'||!Number.isSafeInteger(record.code.bytes)||record.code.bytes<0||
        !/^[a-f0-9]{64}$/.test(record.code.sha256))throw Error('anonymous script source unavailable')
      const id=rowFor(record.code);if(id===null)return
      const row=sources[id],matched=row.issued.length>0
      if(matched){matchedEvents++;row.matchedEvents++}else{unmatchedEvents++;row.unmatchedEvents++}
      const last=runs[runs.length-1]
      if(last&&last[0]===id&&last[1]===matched)last[2]++
      else if(runs.length<runLimit)runs.push([id,matched,1])
      else overflow=true
    },
    report:()=>({replayEligible:false,ownershipEstablished:false,match:'exact-utf16le-sha256-issued-before-event',
      anonymousEvents,matchedEvents,unmatchedEvents,issuedCalls,overflow,
      sources:sources.map(s=>({code:{...s.code},issued:s.issued.map(v=>({...v})),matchedEvents:s.matchedEvents,unmatchedEvents:s.unmatchedEvents})),
      runs:runs.map(r=>[...r]),limits:{sourceLimit,runLimit,byteLimit},retainedBytes,
      limitation:'Matching source text may also be compiled by application code; neither origin nor harmlessness follows from a match. Ordering and all unmatched events are retained.'}),
  }
}

export function validateIssuedCompilationSources(report,compilation) {
  validateCompilationCounts(compilation)
  if(!report||report.replayEligible!==false||report.ownershipEstablished!==false||report.overflow!==false||
    report.match!=='exact-utf16le-sha256-issued-before-event'||!Array.isArray(report.sources)||!Array.isArray(report.runs))
    throw Error('issued-source accounting unavailable')
  const safe=n=>Number.isSafeInteger(n)&&n>=0
  for(const name of ['anonymousEvents','matchedEvents','unmatchedEvents','issuedCalls','retainedBytes'])
    if(!safe(report[name]))throw Error('invalid issued-source total')
  if(!report.limits||!['sourceLimit','runLimit','byteLimit'].every(k=>safe(report.limits[k])&&report.limits[k]>0)||
    report.sources.length>report.limits.sourceLimit||report.runs.length>report.limits.runLimit||report.retainedBytes>report.limits.byteLimit)
    throw Error('issued-source capacity differs')
  const seen=new Set(),totals=report.sources.map(()=>[0,0]),expected=new Map()
  let issued=0
  for(const row of report.sources) {
    const c=row.code,key=codeKey(c)
    if(c.encoding!=='utf16le'||!safe(c.bytes)||c.bytes%2||!/^[a-f0-9]{64}$/.test(c.sha256)||seen.has(key)||
      !safe(row.matchedEvents)||!safe(row.unmatchedEvents)||!Array.isArray(row.issued))throw Error('invalid issued source')
    seen.add(key);const methods=new Set()
    for(const origin of row.issued) {
      if(!supported.has(origin.method)||methods.has(origin.method)||!safe(origin.calls)||origin.calls===0)throw Error('invalid issued origin')
      methods.add(origin.method);issued+=origin.calls
    }
    if(row.matchedEvents&&!row.issued.length)throw Error('matched source has no prior issue')
    expected.set(key,row.matchedEvents+row.unmatchedEvents)
  }
  for(const run of report.runs) {
    if(!Array.isArray(run)||run.length!==3||!safe(run[0])||run[0]>=totals.length||
      typeof run[1]!=='boolean'||!safe(run[2])||run[2]===0)throw Error('invalid issued-source run')
    totals[run[0]][run[1]?0:1]+=run[2]
  }
  if(totals.some((n,i)=>n[0]!==report.sources[i].matchedEvents||n[1]!==report.sources[i].unmatchedEvents)||
    totals.reduce((n,r)=>n+r[0],0)!==report.matchedEvents||totals.reduce((n,r)=>n+r[1],0)!==report.unmatchedEvents||
    report.matchedEvents+report.unmatchedEvents!==report.anonymousEvents||issued!==report.issuedCalls)
    throw Error('issued-source counts differ')
  const actual=new Map()
  for(const script of compilation.scripts)if(script.kind==='anonymous') {
    const key=codeKey(script.code);actual.set(key,(actual.get(key)??0)+script.count)
  }
  if([...expected].some(([k,n])=>(actual.get(k)??0)!==n)||[...actual].some(([k,n])=>expected.get(k)!==n))
    throw Error('issued-source accounting differs from complete compilation')
  const rawRuns=compilation.encoding==='block-dictionary-v1'?blockCompilationRuns(compilation):
    compilation.encoding==='dictionary-v1'?compilation.runs:compilation.scripts.map((s,i)=>[i,s.count])
  let index=0,remaining=report.runs[0]?.[2]??0
  for(const [id,count] of rawRuns) {
    const script=compilation.scripts[id]
    if(script.kind!=='anonymous')continue
    let pending=count
    while(pending) {
      const run=report.runs[index]
      if(!run||codeKey(script.code)!==codeKey(report.sources[run[0]].code))throw Error('issued-source order differs')
      const take=Math.min(pending,remaining);pending-=take;remaining-=take
      if(!remaining){index++;remaining=report.runs[index]?.[2]??0}
    }
  }
  if(index!==report.runs.length)throw Error('unmatched issued-source runs remain')
}
