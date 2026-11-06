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
import {validateCompilationCounts,compilationSequence} from './compiled-script-log.mjs'
import {validateNativeEntryLog,nativeEntryFrames} from './native-entry-log.mjs'

const [projectArg,outArg,phase]=process.argv.slice(2)
const project=path.resolve(projectArg),out=path.resolve(outArg)
const sha=b=>crypto.createHash('sha256').update(b).digest('hex')
const self=fileURLToPath(import.meta.url)
const moduleProbe=process.env.QUOD_WORKER_MODULE_PROBE==='1'
const boundaryProbe=process.env.QUOD_WORKER_BOUNDARY_PROBE==='1'
const moduleMetadataProbe=process.env.QUOD_WORKER_MODULE_METADATA_PROBE==='1'||boundaryProbe
function write(file,data){fs.writeFileSync(file,JSON.stringify(data,null,2)+'\n')}
function checkCompilationCounts(report) {
  validateCompilationCounts(report)
}
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
  if(moduleMetadataProbe&&(phase==='after'||boundaryProbe&&phase==='before')) {
    const directory=path.join(out,boundaryProbe?'worker-boundary/'+phase:'worker-module-metadata')
    fs.mkdirSync(directory,{recursive:true})
    const original=V.ForksPoolWorker.prototype.start
    V.ForksPoolWorker.prototype.start=function(...args) {
      this.execArgv=['--import',fileURLToPath(new URL('./worker-module-metadata-bootstrap.mjs',import.meta.url)),...this.execArgv]
      this.env={...this.env,QUOD_WORKER_MODULE_METADATA_REPORT_DIR:directory,QUOD_WORKER_BOUNDARY_PROBE:boundaryProbe?'1':'0',
        ...(boundaryProbe?{QUOD_NATIVE_CAPTURE_ADDON:process.env.QUOD_NATIVE_CAPTURE_ADDON}:{})}
      return Reflect.apply(original,this,args)
    }
  }
  if(moduleProbe&&phase!=='baseline') {
    const directory=path.join(out,'worker-modules',phase)
    fs.mkdirSync(directory,{recursive:true})
    const original=V.ForksPoolWorker.prototype.start
    V.ForksPoolWorker.prototype.start=function(...args) {
      this.execArgv=['--import',fileURLToPath(new URL('./worker-module-bootstrap.mjs',import.meta.url)),...this.execArgv]
      this.env={...this.env,QUOD_WORKER_MODULE_REPORT_DIR:directory}
      return Reflect.apply(original,this,args)
    }
  }
  if(phase==='after'&&process.env.QUOD_WORKER_CONFIG_PROBE==='1') {
    const directory=path.join(out,'worker-config')
    fs.mkdirSync(directory,{recursive:true})
    const original=V.ForksPoolWorker.prototype.start
    V.ForksPoolWorker.prototype.start=function(...args) {
      const native=process.env.QUOD_WORKER_CONFIG_NATIVE_PROBE==='1'
      this.execArgv=[native?'--import':'--require',fileURLToPath(new URL(native?'./worker-config-bootstrap.mjs':'./worker-config-probe.cjs',import.meta.url)),...this.execArgv]
      this.env={...this.env,QUOD_WORKER_CONFIG_REPORT_DIR:directory,QUOD_WORKER_CONFIG_NATIVE_PROBE:native?'1':'0'}
      return Reflect.apply(original,this,args)
    }
  }
  const observer=phase==='baseline'?null:observeWorkerInputs(V)
  try {
    await V.startVitest('test',[],{run:true,watch:false,reporters:['default','json'],outputFile:{json:path.join(out,phase+'-tests.json')}})
  }finally{
    write(path.join(out,phase+'-inputs.json'),{node:process.version,vitest:pkg.version,
      entrySha256:sha(fs.readFileSync(entry)),...(observer?.report()??{baseline:true})})
    observer?.close()
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
  const baseline=moduleProbe||boundaryProbe?run('baseline'):null
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
  if(moduleMetadataProbe) {
    const directory=path.join(out,boundaryProbe?'worker-boundary/after':'worker-module-metadata')
    const reports=fs.readdirSync(directory).filter(f=>f.endsWith('.json')).map(f=>JSON.parse(fs.readFileSync(path.join(directory,f))))
    assert.equal(reports.length,120,'every worker must report module metadata')
    assert.equal(new Set(reports.map(r=>JSON.stringify([r.project,r.files]))).size,120)
    assert(reports.every(r=>!r.replayEligible&&!r.unknown.length&&r.files.length===1))
    assert(reports.every(r=>!r.metadata.replayEligible&&!r.metadata.completeOwnership&&!r.metadata.overflow&&!r.metadata.errors.length&&r.metadata.nodes>0))
    assert(reports.every(r=>r.native.opaque.available&&r.native.inspector.available&&r.native.proxy.available&&!r.native.proxy.overflow))
    assert(reports.every(r=>r.native.hooks.available&&!r.native.hooks.overflow),'hook lifetime observation must be available in every worker')
    assert(reports.every(r=>!r.transport.replayEligible&&!r.transport.completeOwnership&&!r.transport.overflow&&
      !r.transport.errors.length&&r.transport.pending===0&&r.transport.unsettled===0),'transport observation must complete in every worker')
    const events=reports.flatMap(r=>r.metadata.events),invalidations=reports.flatMap(r=>r.metadata.invalidations)
    assert(events.every(e=>e.source?.available),'every metadata operation must have executable source evidence')
    const readerSources=new Map(),unmatchedReaderSources=[]
    let readerCalls=0,matchedReaderCalls=0
    for(const r of reports)for(const e of r.metadata.events) {
      const s=e.source,key=JSON.stringify([s.script,s.code])
      const matches=s.script.isModule&&!s.script.hasSourceURL&&r.expectedReaderSources.some(x=>x.url===s.script.url&&x.sha256===s.code.sha256)
      readerCalls+=e.count
      if(matches)matchedReaderCalls+=e.count
      else unmatchedReaderSources.push({project:r.project,files:r.files,operation:e.operation,key:e.key,source:s,count:e.count})
      if(!readerSources.has(key))readerSources.set(key,{script:s.script,code:s.code,count:0})
      readerSources.get(key).count+=e.count
    }
    const operations=Object.create(null),operationsByOrigin=Object.create(null),transportCounts=Object.create(null)
    for(const e of events){
      const k=e.operation+':'+e.key;operations[k]=(operations[k]??0)+e.count
      const origin=JSON.stringify(e.origins),group=operationsByOrigin[origin]??=Object.create(null)
      group[k]=(group[k]??0)+e.count
    }
    for(const r of reports)for(const [k,n] of Object.entries(r.transport.counts))transportCounts[k]=(transportCounts[k]??0)+n
    const entryCounts=Object.create(null),argumentCounts=Object.create(null),hookEntryCounts=Object.create(null)
    for(const r of reports)for(const e of r.native.opaque.entries)for(const name of e.entries)entryCounts[name]=(entryCounts[name]??0)+1
    for(const r of reports)for(const e of r.native.proxy.entries) {
      const k=e.argument.available?e.argument.kind:'unknown: '+e.argument.reason
      argumentCounts[k]=(argumentCounts[k]??0)+e.count
    }
    for(const r of reports)for(const e of r.native.hooks.entries)for(const name of e.entries)hookEntryCounts[name]=(hookEntryCounts[name]??0)+e.count
    const activeHooks=s=>Object.values(s.promiseHooks).some(n=>n!==0)||s.asyncHooks.kTotals!==0||s.pendingAsyncHookEdit
    const cold=x=>!x.evaluated&&!x.hasMeta&&!x.hasMap&&!x.hasPromise&&!x.hasExports&&x.imports===0
    const metadataResult={replayEligible:false,completeOwnership:false,workers:reports.length,
      fingerprint:first.fingerprint,plainAndProbeFingerprintEqual:first.fingerprint===second.fingerprint,
      nodes:reports.reduce((n,r)=>n+r.metadata.nodes,0),assignments:reports.reduce((n,r)=>n+r.metadata.assignments,0),
      metadataObjects:reports.reduce((n,r)=>n+r.metadata.metadata.length,0),operations,operationsByOrigin,transportCounts,
      readerCalls,matchedReaderCalls,readerSources:[...readerSources.values()],unmatchedReaderSources,
      invalidations:invalidations.length,coldInvalidations:invalidations.filter(cold).length,
      nonColdInvalidations:invalidations.filter(x=>!cold(x)),
      workersWithReads:reports.filter(r=>r.metadata.events.some(e=>['get','has','descriptor','keys'].includes(e.operation))).length,
      workersWithStoredReads:reports.filter(r=>r.metadata.events.some(e=>e.origins.includes('stored')&&['get','has','descriptor','keys'].includes(e.operation))).length,
      gaps:[...new Set(reports.flatMap(r=>r.metadata.gaps))],
      native:{entryCounts,argumentCounts,hookEntryCounts,
        workersWithInitialHooks:reports.filter(r=>activeHooks(r.native.hooks.initial)).length,
        workersWithFinalHooks:reports.filter(r=>activeHooks(r.native.hooks.current)).length,
        workersWithHookAccess:reports.filter(r=>r.native.hooks.entries.length>0).length,
        workersWithInspectorEntries:reports.filter(r=>r.native.inspector.entries.length>0).length,
        workersWithCachedNativeModules:reports.filter(r=>r.native.opaque.cachedNativeModules.length>0).length,
        workersWithProxyUnwrapEntries:reports.filter(r=>r.native.proxy.entries.some(e=>e.entries.includes('util.getProxyDetails'))).length},
      limitations:[...reports[0].metadata.limitations,...reports[0].transport.limitations]}
    write(path.join(out,'worker-module-metadata-result.json'),metadataResult)
    console.log(JSON.stringify(metadataResult,null,2))
    if(boundaryProbe) {
      const beforeDir=path.join(out,'worker-boundary/before')
      const previous=fs.readdirSync(beforeDir).filter(f=>f.endsWith('.json')).map(f=>JSON.parse(fs.readFileSync(path.join(beforeDir,f))))
      const key=r=>JSON.stringify([r.project,r.files]),byKey=new Map(previous.map(r=>[key(r),r]))
      assert.equal(previous.length,120);assert.equal(byKey.size,120)
      const code=events=>events.map(e=>({kind:e.kind,identity:e.identity,...(e.code?{code:e.code}:{})}))
      const comparisons=[]
      for(const r of reports) {
        const a=byKey.get(key(r));assert(a,'every combined worker must have a previous observation')
        for(const x of [a,r]) {
          assert(!x.unknown.length&&!x.metadata.errors.length&&!x.metadata.gaps.length&&!x.metadata.overflow)
          assert(!x.transport.errors.length&&!x.transport.overflow&&x.transport.pending===0&&x.transport.unsettled===0)
          assert(x.native.opaque.available&&x.native.inspector.available&&x.native.proxy.available&&x.native.hooks.available)
          assert(!x.native.proxy.overflow&&!x.native.hooks.overflow)
          assert(x.native.descriptors?.available&&!x.native.descriptors.overflow&&!x.native.descriptors.completeDescriptorCoverage,
            'listed descriptor APIs must be observed, without claiming complete descriptor coverage')
          assert.equal(x.native.descriptors.argumentDiagnostics?.mode,'native-entry-arguments',
            'the combined diagnostic must report actual native arguments')
          validateNativeEntryLog(x.native.descriptors);validateNativeEntryLog(x.native.proxy)
          assert.equal(x.native.identity?.targetAccessorsOnly,true,'the diagnostic must use the gated accessor-only observation')
          assert.equal(x.native.identity?.argumentMode,'direct','the combined diagnostic must use the gated direct argument transport')
          assert.equal(x.native.identity?.targetInspection,'native-watched','the combined diagnostic must use the gated native inspection')
          assert.equal(x.native.identity?.nativeCapture,true)
          assert.equal(x.native.identity?.handleMode,'native-private-capture')
          for(const entry of x.native.descriptors.entries)if(entry.argument.kind==='object') {
            const target=entry.argument.target
            assert(target&&['all-own-accessors','watched-properties'].includes(target.inspectionScope),
              'every object target must declare its descriptor inspection scope')
            assert(Array.isArray(target.watchedAccessorKeys)&&new Set(target.watchedAccessorKeys).size===target.watchedAccessorKeys.length&&
              target.watchedAccessorKeys.every(key=>['seed','tmp','invalidate'].includes(key)))
            if(target.inspectionScope==='watched-properties')assert(!Object.hasOwn(target,'ownAccessorCount')&&
              target.watchedAccessorCount===target.watchedAccessorKeys.length,'scoped inspection must not claim an all-accessor count')
          }
          const evidence=x.native.descriptors.sourceEvidence
          assert(evidence?.available&&evidence.scope==='frames-of-watched-accessor-targets',
            'watched target call stacks must retain actual V8 source evidence')
          const sourceIds=new Set(evidence.records.filter(r=>r.available&&r.code.encoding==='utf16le').map(r=>r.scriptId))
          for(const entry of x.native.descriptors.entries)if(entry.argument.target?.watchedAccessorKeys.length)
            assert(nativeEntryFrames(x.native.descriptors,entry).every(f=>sourceIds.has(f.location.scriptId)),
              'every watched target frame must resolve by its actual script identity')
          assert(x.native.identity?.available&&x.native.identity.mode==='native-entry-arguments'&&x.native.identity.snapshotsRequested===0,
            'native argument observation must remain available without collecting heap snapshots')
          assert.equal(x.native.identity.calls,x.native.identity.cleared,'every diagnostic exception wrapper must be cleared')
          assert.equal(x.native.identity.calls,[...x.native.descriptors.entries,...x.native.proxy.entries].reduce((n,e)=>n+e.count,0),
            'every native argument event must be attributed once')
          assert([...x.native.descriptors.entries,...x.native.proxy.entries].every(e=>e.argument.available&&e.argument.actualNativeEntry),
            'native argument evidence must not silently fall back to reconstructed caller expressions')
          assert.equal(x.metadata.observation,'accessor')
          for(const field of ['evaluation','compilation'])assert(!x.boundary[field].errors.length&&!x.boundary[field].overflow)
          assert(x.boundary.compilation.enabled&&!x.boundary.compilation.completeCompilationCoverage)
          assert(x.boundary.compilation.scriptMetadataIndex&&!x.boundary.compilation.scriptMetadataIndex.overflow)
          assert.equal(x.boundary.compilation.scriptMetadataIndex.events+x.boundary.compilation.failedToParse,
            x.boundary.compilation.events,'every parsed script must reach the exact metadata index')
          checkCompilationCounts(x.boundary.compilation)
          assert(x.boundary.config&&!x.boundary.config.overflow)
          assert(x.boundary.seed?.installed&&x.boundary.seed.intact&&!x.boundary.seed.overflow&&!x.boundary.seed.gaps.length,
            'scalar seed observation must remain installed and complete within its declared scope')
        }
        comparisons.push({key:key(r),codeSequenceEqual:JSON.stringify(code(a.boundary.evaluation.events))===JSON.stringify(code(r.boundary.evaluation.events)),
          compiledSequenceEqual:JSON.stringify(compilationSequence(a.boundary.compilation))===JSON.stringify(compilationSequence(r.boundary.compilation)),
          evaluatorBefore:a.boundary.evaluation.events.length,evaluatorAfter:r.boundary.evaluation.events.length,
          configSeedReadsBefore:a.boundary.config.events.filter(e=>e.path.join('.')==='sequence.seed').reduce((n,e)=>n+e.count,0),
          configSeedReadsAfter:r.boundary.config.events.filter(e=>e.path.join('.')==='sequence.seed').reduce((n,e)=>n+e.count,0),
          scalarSeedOperationsBefore:a.boundary.seed.events.reduce((n,e)=>n+e.count,0),
          scalarSeedOperationsAfter:r.boundary.seed.events.reduce((n,e)=>n+e.count,0),
          descriptorCallsBefore:a.native.descriptors.entries.reduce((n,e)=>n+e.count,0),
          descriptorCallsAfter:r.native.descriptors.entries.reduce((n,e)=>n+e.count,0),
          unknownDescriptorTargetsBefore:a.native.descriptors.entries.filter(e=>!e.argument.available).reduce((n,e)=>n+e.count,0),
          unknownDescriptorTargetsAfter:r.native.descriptors.entries.filter(e=>!e.argument.available).reduce((n,e)=>n+e.count,0)})
      }
      const combined={replayEligible:false,completeOwnership:false,baseline,first,second,workers:comparisons.length,
        equalEvaluatorCodeSequences:comparisons.filter(c=>c.codeSequenceEqual).length,
        equalCompiledSequences:comparisons.filter(c=>c.compiledSequenceEqual).length,comparisons,
        limitations:['Recorded code, configuration, transport and native observations coexist in these runs',
          'Earlier primitive copies, descriptor channels, full filesystem/environment inputs, parent effects and opaque channels remain outside complete coverage']}
      write(path.join(out,'worker-boundary-result.json'),combined)
      console.log(JSON.stringify({boundaryWorkers:combined.workers,equalEvaluatorCodeSequences:combined.equalEvaluatorCodeSequences,
        equalCompiledSequences:combined.equalCompiledSequences,replayEligible:false},null,2))
    }
  }
  if(moduleProbe) {
    function loadModules(phase) {
      const directory=path.join(out,'worker-modules',phase)
      const reports=fs.readdirSync(directory).filter(f=>f.endsWith('.json')).map(f=>JSON.parse(fs.readFileSync(path.join(directory,f))))
      assert.equal(reports.length,120,'all isolated workers must report modules')
      const entries=reports.map(r=>[JSON.stringify([r.project,r.files]),r])
      assert.equal(new Set(entries.map(([k])=>k)).size,120)
      assert(reports.every(r=>!r.replayEligible&&!r.unknown.length&&r.files.length===1))
      assert(reports.every(r=>!r.evaluation.errors.length&&!r.compilation.errors.length&&!r.evaluation.overflow&&!r.compilation.overflow))
      assert(reports.every(r=>r.evaluation.events.length>0&&r.compilation.scripts.length>0))
      reports.forEach(r=>checkCompilationCounts(r.compilation))
      return new Map(entries)
    }
    const left=loadModules('before'),right=loadModules('after'),comparisons=[]
    function group(rows) {
      const groups=new Map()
      for(const row of rows) {
        assert.equal(row.identity.available,true)
        const count=row.count??1
        assert(Number.isSafeInteger(count)&&count>0)
        const k=row.identity.hash,group=groups.get(k)??{total:0,bodies:new Map()}
        group.total+=count
        group.bodies.set(row.code.sha256,(group.bodies.get(row.code.sha256)??0)+count)
        groups.set(k,group)
      }
      return groups
    }
    function compare(a,b) {
      const left=group(a),right=group(b),result={paired:0,unpaired:0,differentBodies:0,differentMultiplicity:0,differentBodyMultisets:0}
      for(const key of new Set([...left.keys(),...right.keys()])) {
        const a=left.get(key),b=right.get(key)
        if(!a||!b){result.unpaired++;continue}
        result.paired++
        if(a.total!==b.total)result.differentMultiplicity++
        if(JSON.stringify([...a.bodies].sort())!==JSON.stringify([...b.bodies].sort()))result.differentBodyMultisets++
        if(JSON.stringify([...a.bodies.keys()].sort())!==JSON.stringify([...b.bodies.keys()].sort()))result.differentBodies++
      }
      return result
    }
    for(const [key,a] of left) {
      const b=right.get(key);assert(b,'workers must pair by project and file')
      const body=r=>r.evaluation.events.map(({kind,identity,code})=>({kind,identity,code}))
      const metadataComparable=[a,b].every(r=>r.evaluation.events.every(e=>e.identity.available&&(e.kind==='external'||e.meta.available)))
      const metadataFieldChanges=Object.create(null)
      let unknownMetadataFields=0
      for(let i=0;i<Math.max(a.evaluation.events.length,b.evaluation.events.length);i++) {
        const x=a.evaluation.events[i],y=b.evaluation.events[i]
        if(x?.kind!=='inline'&&y?.kind!=='inline')continue
        if(x?.kind!=='inline'||y?.kind!=='inline'||!x.identity.available||!y.identity.available||x.identity.hash!==y.identity.hash||!x.metaFields?.available||!y.metaFields?.available) {
          unknownMetadataFields++;continue
        }
        for(const name of new Set([...Object.keys(x.metaFields.fields),...Object.keys(y.metaFields.fields)])) {
          const l=Object.hasOwn(x.metaFields.fields,name)?x.metaFields.fields[name]:undefined
          const r=Object.hasOwn(y.metaFields.fields,name)?y.metaFields.fields[name]:undefined
          if(l?.available===false||r?.available===false){unknownMetadataFields++;continue}
          if(JSON.stringify(l)!==JSON.stringify(r))metadataFieldChanges[name]=(metadataFieldChanges[name]??0)+1
        }
      }
      comparisons.push({key,before:{evaluations:a.evaluation.events.length,scripts:a.compilation.events},
        after:{evaluations:b.evaluation.events.length,scripts:b.compilation.events},
        metadataComparable,metadataFieldChanges,unknownMetadataFields,
        evaluatorSequenceEqual:metadataComparable?JSON.stringify(a.evaluation.events)===JSON.stringify(b.evaluation.events):null,
        codeSequenceEqual:JSON.stringify(body(a))===JSON.stringify(body(b)),
        compiledSequenceEqual:JSON.stringify(compilationSequence(a.compilation))===JSON.stringify(compilationSequence(b.compilation)),
        inlined:compare(a.evaluation.events.filter(e=>e.kind==='inline'),b.evaluation.events.filter(e=>e.kind==='inline')),
        compiled:compare(a.compilation.scripts,b.compilation.scripts)})
    }
    const totals={}
    for(const kind of ['inlined','compiled'])totals[kind]=Object.fromEntries(['paired','unpaired','differentBodies','differentMultiplicity','differentBodyMultisets'].map(k=>[k,comparisons.reduce((n,c)=>n+c[kind][k],0)]))
    const moduleResult={replayEligible:false,completeCompilationCoverage:false,baselineFingerprint:baseline.fingerprint,workers:comparisons.length,
      equalEvaluatorSequences:comparisons.filter(c=>c.evaluatorSequenceEqual).length,
      equalCodeSequences:comparisons.filter(c=>c.codeSequenceEqual).length,
      equalCompiledSequences:comparisons.filter(c=>c.compiledSequenceEqual).length,totals,comparisons,
      unknownMetadataWorkers:comparisons.filter(c=>!c.metadataComparable).length,
      wasmBefore:[...left.values()].reduce((n,r)=>n+r.compilation.scripts.filter(s=>s.language==='WebAssembly').reduce((m,s)=>m+s.count,0),0),
      wasmAfter:[...right.values()].reduce((n,r)=>n+r.compilation.scripts.filter(s=>s.language==='WebAssembly').reduce((m,s)=>m+s.count,0),0),
      limitation:'Debugger coverage can omit unbound scripts; module code observations do not prove configuration, non-code data, Wasm imports, external inputs, parent RPC state or side effects'}
    write(path.join(out,'worker-module-result.json'),moduleResult)
    console.log(JSON.stringify({...moduleResult,comparisons:comparisons.length},null,2))
  }
  if(process.env.QUOD_WORKER_CONFIG_PROBE==='1') {
    const directory=path.join(out,'worker-config')
    const reports=fs.readdirSync(directory).filter(f=>f.endsWith('.json')).map(f=>JSON.parse(fs.readFileSync(path.join(directory,f))))
    const workers=reports.flatMap(r=>r.reports)
    assert.equal(workers.length,120,'every worker must provide diagnostic evidence')
    assert(reports.every(r=>r.unknown.length===0&&!r.replayEligible))
    assert(workers.every(r=>r.files.length===1&&!r.replayEligible&&!r.overflow))
    const seedReads=workers.filter(w=>w.events.some(e=>e.path.join('.')==='sequence.seed'))
    const configResult={replayEligible:false,workers:workers.length,workersObservingSeed:seedReads.length,
      observations:seedReads.flatMap(w=>w.events.filter(e=>e.path.join('.')==='sequence.seed').map(e=>({project:w.project,files:w.files,...e}))),
      plainAndProbeFingerprintEqual:first.fingerprint===second.fingerprint,
      limitation:'Proxy observations cannot exclude native reads or prove that unobserved fields are irrelevant'}
    if(process.env.QUOD_WORKER_CONFIG_NATIVE_PROBE==='1') {
      assert(reports.every(r=>r.native?.opaque.available&&r.native.inspector.available),'native observation must be available in every worker')
      const entries=reports.flatMap(r=>r.native.opaque.entries.flatMap(e=>e.entries))
      const counts=Object.fromEntries([...new Set(entries)].sort().map(k=>[k,entries.filter(e=>e===k).length]))
      assert(reports.every(r=>r.native.proxy.available&&!r.native.proxy.overflow),'proxy entry observation must be complete in every worker')
      const proxyCounts={}
      for(const r of reports)for(const e of r.native.proxy.entries)for(const name of e.entries)proxyCounts[name]=(proxyCounts[name]??0)+e.count
      const argumentCounts={}
      for(const r of reports)for(const e of r.native.proxy.entries) {
        const key=e.argument.available?e.argument.kind:'unknown: '+e.argument.reason
        argumentCounts[key]=(argumentCounts[key]??0)+e.count
      }
      configResult.native={workers:reports.length,entryCounts:counts,
        workersWithOpaqueEntries:reports.filter(r=>r.native.opaque.entries.length>0).length,
        workersWithInspectorEntries:reports.filter(r=>r.native.inspector.entries.length>0).length,
        workersWithCachedNativeModules:reports.filter(r=>r.native.opaque.cachedNativeModules.length>0).length,
        proxyEntryCounts:proxyCounts,
        nativeArgumentCounts:argumentCounts,
        workersWithProxyArguments:reports.filter(r=>r.native.proxy.entries.some(e=>e.argument.available&&e.argument.kind==='proxy')).length,
        workersWithUnknownArguments:reports.filter(r=>r.native.proxy.entries.some(e=>!e.argument.available)).length,
        workersWithProxyUnwrapEntries:reports.filter(r=>r.native.proxy.entries.some(e=>e.entries.includes('util.getProxyDetails'))).length,
        replayEligible:false}
    }
    write(path.join(out,'worker-config-result.json'),configResult)
    console.log(JSON.stringify(configResult,null,2))
  }
  console.log(JSON.stringify({...result,changes:changes.length,comparisons:comparisons.length},null,2))
}
