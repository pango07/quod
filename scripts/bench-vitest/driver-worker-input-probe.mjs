// Two plain Vitest runs, with a metadata-only edit between them. Inventory
// actual worker inputs; neither run uses Quod or skips any tests.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {fileURLToPath,pathToFileURL} from 'node:url'
import {spawnSync} from 'node:child_process'
import {observeWorkerInputs} from './worker-input-probe.mjs'

const [projectArg,outArg,phase]=process.argv.slice(2)
const project=path.resolve(projectArg),out=path.resolve(outArg)
const sha=b=>crypto.createHash('sha256').update(b).digest('hex')
const self=fileURLToPath(import.meta.url)
function write(file,data){fs.writeFileSync(file,JSON.stringify(data,null,2)+'\n')}
function states(file) {
  const report=JSON.parse(fs.readFileSync(file))
  const rows=report.testResults.map(f=>[path.relative(project,f.name),f.status,
    f.assertionResults.map(a=>[a.fullName,a.status]).sort((a,b)=>JSON.stringify(a).localeCompare(JSON.stringify(b)))])
    .sort((a,b)=>JSON.stringify(a).localeCompare(JSON.stringify(b)))
  return {files:rows.length,states:rows.reduce((n,r)=>n+r[2].length,0),fingerprint:sha(JSON.stringify(rows)),
    paths:[...new Set(report.testResults.map(f=>f.name))].sort()}
}
fs.mkdirSync(out,{recursive:true})
if(phase) {
  process.chdir(project)
  const require=createRequire(path.join(project,'package.json'))
  const packageFile=require.resolve('vitest/package.json'),pkg=JSON.parse(fs.readFileSync(packageFile))
  assert.equal(pkg.version,'5.0.0','review the boundary before changing Vitest versions')
  const nodeEntry=pkg.exports['./node'],relative=typeof nodeEntry==='string'?nodeEntry:nodeEntry.import??nodeEntry.default
  const entry=path.join(path.dirname(packageFile),relative)
  const V=await import(pathToFileURL(entry).href)
  const observer=observeWorkerInputs(V)
  try {
    await V.startVitest('test',[],{run:true,watch:false,reporters:['default','json'],outputFile:{json:path.join(out,phase+'-tests.json')}})
  }finally{
    write(path.join(out,phase+'-inputs.json'),{node:process.version,vitest:pkg.version,
      entrySha256:sha(fs.readFileSync(entry)),...observer.report()})
    observer.close()
  }
}else {
  function run(name) {
    const r=spawnSync(process.execPath,[self,project,out,name],{cwd:project,encoding:'utf8',timeout:420000,maxBuffer:32*1024*1024})
    fs.writeFileSync(path.join(out,name+'.log'),(r.stdout??'')+(r.stderr??''))
    assert.equal(r.error,undefined);assert.equal(r.status,0,'plain Vitest failed; see '+name+'.log')
    const s=states(path.join(out,name+'-tests.json'))
    assert.equal(s.fingerprint,'09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd')
    return s
  }
  const first=run('before'),changes=[]
  for(const file of first.paths) {
    assert.ok(file.startsWith(project+'/'),'only modify the disposable benchmark checkout')
    const content=sha(fs.readFileSync(file)),s=fs.statSync(file)
    fs.utimesSync(file,s.atime,new Date(s.mtimeMs+60000))
    assert.equal(sha(fs.readFileSync(file)),content)
    changes.push({file:path.relative(project,file),contentSha256:content,mtimeChanged:fs.statSync(file).mtimeMs!==s.mtimeMs})
  }
  assert.ok(changes.length>0&&changes.every(c=>c.mtimeChanged))
  const second=run('after')
  const before=JSON.parse(fs.readFileSync(path.join(out,'before-inputs.json')))
  const after=JSON.parse(fs.readFileSync(path.join(out,'after-inputs.json')))
  function workerKey(w) {
    return JSON.stringify([w.starts[0]?.config.name?.hash,w.runs.map(r=>r.files)])
  }
  function index(report) {
    const entries=report.workers.map(w=>[workerKey(w),w])
    assert.equal(new Set(entries.map(([k])=>k)).size,entries.length,'worker attribution must be unambiguous')
    return new Map(entries)
  }
  function diff(a,b) {
    return [...new Set([...Object.keys(a??{}),...Object.keys(b??{})])].filter(k=>JSON.stringify(a?.[k])!==JSON.stringify(b?.[k])).sort()
  }
  function rpc(w){return w.requests.map(r=>({method:r.method,args:r.args,optional:r.optional,response:r.response}))}
  function comparable(v) {
    if(!v||typeof v!=='object')return true
    if(v.available===false)return false
    return Object.values(v).every(comparable)
  }
  function summary(r) {
    const methods={},unsupported={}
    for(const w of r.workers)for(const q of w.requests)methods[q.method]=(methods[q.method]??0)+1
    function visit(v){if(!v||typeof v!=='object')return;if(v.available===false)unsupported[v.reason]=(unsupported[v.reason]??0)+1;for(const x of Object.values(v))visit(x)}
    visit(r)
    return {workers:r.workers.length,runs:r.workers.reduce((n,w)=>n+w.runs.length,0),methods,unsupported,
      pending:r.workers.reduce((n,w)=>n+w.pending,0),unknown:r.workers.reduce((n,w)=>n+w.unknown,0),
      errors:r.workers.flatMap(w=>w.errors),failures:r.failures}
  }
  const a=index(before),b=index(after),comparisons=[]
  for(const [key,w] of a) {
    const next=b.get(key)
    const canCompare=!!next&&comparable(rpc(w))&&comparable(rpc(next))
    comparisons.push({key,matched:!!next,configChanged:diff(w.starts[0]?.config,next?.starts[0]?.config),
      sequenceChanged:diff(w.starts[0]?.sequence,next?.starts[0]?.sequence),
      startPacketChanged:diff(w.starts[0]?.packet,next?.starts[0]?.packet),
      startContextChanged:diff(w.starts[0]?.context,next?.starts[0]?.context),
      runContextChanged:diff(w.runs[0]?.context,next?.runs[0]?.context),
      envEqual:!!next&&w.starts[0]?.env.available===true&&next.starts[0]?.env.available===true&&w.starts[0].env.hash===next.starts[0].env.hash,
      execArgvEqual:!!next&&w.starts[0]?.execArgv.available===true&&next.starts[0]?.execArgv.available===true&&w.starts[0].execArgv.hash===next.starts[0].execArgv.hash,
      rpcComparable:canCompare,rpcEqual:canCompare?JSON.stringify(rpc(w))===JSON.stringify(rpc(next)):null,
      requestsBefore:w.requests.length,requestsAfter:next?.requests.length??null})
  }
  const result={replayEligible:false,metadataOnly:true,changes,first,second,before:summary(before),after:summary(after),
    pairedWorkers:comparisons.filter(c=>c.matched).length,comparableRpcWorkers:comparisons.filter(c=>c.rpcComparable).length,
    equalRpcWorkers:comparisons.filter(c=>c.rpcEqual===true).length,
    comparisons,limitations:before.limitations}
  write(path.join(out,'result.json'),result)
  console.log(JSON.stringify({...result,changes:changes.length,comparisons:comparisons.length},null,2))
}
