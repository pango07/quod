// Diagnostic only: run the real Rust planner, production driver and worker
// setup with passive main-process stat observations. Every dependency stays
// complete. Use a fresh, disposable state directory; never benchmark savings
// or reuse this diagnostic ledger as a production record.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'
import {readNativeSymbolAudit} from './native-symbol-report.mjs'

const [projectArg,binaryArg,outArg]=process.argv.slice(2)
const project=path.resolve(projectArg),binary=path.resolve(binaryArg),out=path.resolve(outArg)
const source=fileURLToPath(new URL('../../crates/quod-cli/src/vitest/',import.meta.url))
const observer=new URL('./async-stat-probe.mjs',import.meta.url)
const registry=new URL('./promise-observers-probe.mjs',import.meta.url)
const startup=new URL('./startup-stat-probe.cjs',import.meta.url)
const state=path.join(out,'state'),staged=path.join(out,'js'),observations=path.join(out,'async-stats.json')
assert.equal(fs.existsSync(state),false,'diagnostic state must be fresh')
fs.mkdirSync(staged,{recursive:true})
const auditLibrary=process.env.QUOD_NATIVE_AUDIT_LIBRARY
const nativeAuditDirectory=path.join(out,'native-loader')
if(auditLibrary) {
  assert.ok(path.isAbsolute(auditLibrary)&&fs.statSync(auditLibrary).isFile())
  fs.mkdirSync(nativeAuditDirectory)
}
for(const name of fs.readdirSync(source)) if(/\.(mjs|cjs)$/.test(name)||name==='acorn-LICENSE')
  fs.copyFileSync(path.join(source,name),path.join(staged,name))
let driver=fs.readFileSync(path.join(staged,'driver.mjs'),'utf8')
function replaceOnce(before,after) {
  assert.equal(driver.split(before).length,2,'production driver anchor changed: '+before)
  driver=driver.replace(before,after)
}
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex')
replaceOnce("const namesProof = hooks && process.env.QUOD_VITEST_NAMES_PROOF === '1'",
  `// Diagnostic module hashes: ${hash(observer)} ${hash(registry)} ${hash(startup)}\n`+
  `const statDiagnostics=hooks?(await import(${JSON.stringify(observer.href)})).observeAsyncStat(process.cwd(),{startup:nodeModule.createRequire(import.meta.url)(${JSON.stringify(fileURLToPath(startup))})}):null;\n`+
  `process.on('exit',()=>{if(statDiagnostics){try{real.writeFileSync(${JSON.stringify(observations)},JSON.stringify(statDiagnostics.report()))}finally{statDiagnostics.close()}}});\n`+
  "const namesProof = hooks && process.env.QUOD_VITEST_NAMES_PROOF === '1'")
replaceOnce("internalFiles:[new URL('./record.mjs', import.meta.url).href]",
  `internalFiles:[new URL('./record.mjs', import.meta.url).href,${JSON.stringify(observer.href)}]`)
replaceOnce('const booleanProof = booleanEnabled ? statProof : null',
  'const booleanProof = booleanEnabled ? statProof : null\nstatDiagnostics?.setConsumerProof(statProof)')
fs.writeFileSync(path.join(staged,'driver.mjs'),driver)
const preload=path.join(out,'bench-vitest-report.cjs'),report=path.join(out,'vitest-report.json')
fs.writeFileSync(preload,`const entry=process.argv[1]||'';
if(/(?:^|[\\/])vitest(?:\\.m?js)?$/.test(entry)&&process.argv.includes('run')){
 process.argv.push('--reporter=default','--reporter=json','--outputFile.json='+process.env.BENCH_VITEST_REPORT);
}
`)
const args=['vitest','--dir',state,'--audits','0','--report',path.join(out,'internal-report.json'),
  '--summary',path.join(out,'summary.json'),'--','run','--reporter=default','--reporter=json','--outputFile.json='+report]
const child=spawnSync(binary,args,{cwd:project,encoding:'utf8',timeout:420000,maxBuffer:64*1024*1024,
  env:{...process.env,QUOD_SHIM_ACTIVE:'1',QUOD_VITEST_JS_DIR:staged,QUOD_VITEST_NAMES_PROOF:'1',
    ...(auditLibrary?{LD_AUDIT:auditLibrary,QUOD_NATIVE_AUDIT_DIR:nativeAuditDirectory}:{}),
    QUOD_VITEST_BOOLEAN_PROOF:'0',QUOD_VITEST_LINEAGE:'0',QUOD_VITEST_TIMING:'off',
    NODE_OPTIONS:'--require="'+fileURLToPath(startup).replace(/\\/g,'\\\\').replace(/"/g,'\\"')+'" --require='+preload,BENCH_VITEST_REPORT:report}})
fs.writeFileSync(path.join(out,'driver.log'),(child.stdout??'')+(child.stderr??''))
assert.equal(child.error,undefined)
assert.equal(child.status,0,'diagnostic command failed; see driver.log')
const suite=JSON.parse(fs.readFileSync(report)),summary=JSON.parse(fs.readFileSync(path.join(out,'summary.json')))
const trace=JSON.parse(fs.readFileSync(observations)),ledger=JSON.parse(fs.readFileSync(path.join(state,'ledger.json')))
const rows=suite.testResults.map(f=>[path.relative(project,f.name),f.status,f.assertionResults.map(a=>[a.fullName,a.status])
  .sort((a,b)=>JSON.stringify(a).localeCompare(JSON.stringify(b)))])
  .sort((a,b)=>JSON.stringify(a).localeCompare(JSON.stringify(b)))
const fingerprint=crypto.createHash('sha256').update(JSON.stringify(rows)).digest('hex')
assert.equal(fingerprint,'09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd')
assert.equal(summary.replayed,0)
assert.equal(trace.replayEligible,false)
assert.equal(trace.startupAtAttach?.startup.available,true,'early origin must precede the benchmark report preload')
const fileCalls=trace.calls.filter(c=>c.sizeConsumer)
let nativeSymbols=null
if(auditLibrary) {
  nativeSymbols=readNativeSymbolAudit(nativeAuditDirectory,{pid:trace.pid})
  assert.equal(nativeSymbols.processes,1)
  const cached=trace.opaqueAccess.cachedNativeModules
  assert.equal(cached.length,1,'review the native boundary if this workload loads other addons')
  assert.deepEqual(nativeSymbols.addons.map(a=>a.path),cached)
  assert.equal(nativeSymbols.addons[0].postRunSha256,
    '09efa62dc8d03fb62f06ab2aadbbc26bde416acb7da20a927f7d53b2bd440322',
    'the loaded path must still contain the exact locked registry artifact after the run')
  fs.writeFileSync(path.join(out,'native-symbols.json'),JSON.stringify(nativeSymbols,null,2))
}
const metadata=Object.fromEntries(['sm','sl','st'].map(kind=>[kind,ledger.keys.filter(k=>k.startsWith(kind+':')).length]))
const result={measurementScope:'Real planner/driver/workers with passive main-process diagnostics and the benchmark report preload; no replay',
  files:rows.length,states:rows.reduce((n,r)=>n+r[2].length,0),fingerprint,replayed:summary.replayed,
  metadata,calls:trace.calls.length,sizeConsumers:fileCalls.length,
  consumersWithSizeOutcome:fileCalls.filter(c=>c.outcome?.available&&c.outcome.projection==='size').length,
  consumersWithOnlyOwnedHooks:fileCalls.filter(c=>c.observedOnlyOwnedHooks).length,
  consumersWithNativeUnchanged:fileCalls.filter(c=>c.observedNativeUnchanged).length,
  consumersWithConstructionFieldsAbsent:fileCalls.filter(c=>c.observedConstructionFieldsAbsent).length,
  consumersWithStartupNativeUnchanged:fileCalls.filter(c=>c.observedStartupNativeUnchanged).length,
  consumersWithNoOpaqueEntries:fileCalls.filter(c=>c.observedNoOpaqueEntries).length,
  startupAtAttach:trace.startupAtAttach,
  initialObservers:trace.initialObservers,nativeProvenance:trace.nativeStatProvenance,
  inspectorAccess:trace.inspectorAccess,
  opaqueAccess:trace.opaqueAccess,
  nativeSymbols,
  preloadModules:trace.preloadModules,replayEligible:false,unvalidated:trace.unvalidated}
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(result,null,2))
console.log(JSON.stringify({...result,opaqueAccess:{...result.opaqueAccess,entries:result.opaqueAccess.entries.length},
  nativeSymbols:nativeSymbols?{...nativeSymbols,addons:nativeSymbols.addons.map(({bindings,...a})=>a)}:null},null,2))
