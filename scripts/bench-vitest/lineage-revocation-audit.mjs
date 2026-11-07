// Remote binary regression. Inject a witness-signature fault, not a hidden
// input or a fabricated promotion, to exercise the actual plain-audit path.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const [projectArg,binaryArg,outArg]=process.argv.slice(2)
const project=path.resolve(projectArg),binary=path.resolve(binaryArg),out=path.resolve(outArg)
assert.equal(fs.existsSync(out),false);fs.mkdirSync(out,{recursive:true})
const input=path.join(out,'lineage-input'),state=path.join(out,'state')
const ledgerPath=path.join(state,'ledger.json'),rows=[]
const digest=data=>crypto.createHash('sha256').update(data).digest('hex')
const ledger=()=>JSON.parse(fs.readFileSync(ledgerPath,'utf8'))
fs.writeFileSync(input,'revision:0')
for(const name of ['a','b'])fs.writeFileSync(path.join(project,`lineage-${name}.test.mjs`),`
import fs from 'node:fs';import {test,expect} from 'vitest';
test('lineage ${name}',()=>expect(fs.readFileSync(${JSON.stringify(input)},'utf8').startsWith('revision:')).toBe(true));
`)
fs.writeFileSync(path.join(project,'lineage-control.test.mjs'),`
import {test,expect} from 'vitest';test('independent control',()=>expect(2+2).toBe(4));
`)
fs.writeFileSync(path.join(project,'vitest.config.mjs'),`export default {
 css:{postcss:{}},test:{include:['lineage-a.test.mjs','lineage-b.test.mjs','lineage-control.test.mjs'],maxWorkers:1}
};\n`)
const env={...process.env,QUOD_VITEST_LINEAGE:'1',QUOD_VITEST_TIMING:'off',
 QUOD_VITEST_PLAIN_WITNESS:'1',QUOD_VITEST_WITNESSES:'2',QUOD_VITEST_RECORDING:'off',
 QUOD_VITEST_NAMES_PROOF:'0',QUOD_VITEST_BOOLEAN_PROOF:'0'}
delete env.QUOD_VITEST_JS_DIR
function run(label,{plain=false,lineage='1'}={}) {
 const report=path.join(out,label+'-report.json'),summary=path.join(out,label+'-summary.json')
 const args=plain?[process.execPath,'node_modules/vitest/vitest.mjs','run','--reporter=json','--outputFile.json='+report]:
  [binary,'vitest','--dir',state,'--no-fast','--no-narrow','--audits','0','--report',report,'--summary',summary,'--','run']
 const result=spawnSync(args[0],args.slice(1),{cwd:project,env:{...env,QUOD_VITEST_LINEAGE:lineage},
  encoding:'utf8',timeout:90000,maxBuffer:8*1024*1024})
 fs.writeFileSync(path.join(out,label+'.log'),(result.stdout??'')+(result.stderr??''))
 assert.equal(result.error,undefined);assert.equal(result.status,0)
 const bytes=fs.readFileSync(report),data=JSON.parse(bytes)
 const states=data.testResults.map(f=>[path.basename(f.name),f.assertionResults.map(a=>[a.fullName??a.title,a.status]).sort()]).sort()
 assert.equal(states.length,3);assert.equal(states.reduce((n,s)=>n+s[1].length,0),3)
 assert.ok(states.every(([,tests])=>tests.every(([,s])=>s==='passed')))
 if(rows.length)assert.deepEqual(states,rows[0].states)
 if(!plain)fs.copyFileSync(ledgerPath,path.join(out,label+'-ledger.json'))
 const row={label,exit:result.status,states,reportSha256:digest(bytes),
  summary:plain?null:JSON.parse(fs.readFileSync(summary,'utf8'))}
 rows.push(row);fs.writeFileSync(path.join(out,'runs.json'),JSON.stringify(rows,null,2)+'\n')
 console.log(JSON.stringify({label,ran:row.summary?.ran,replayed:row.summary?.replayed,
  promoted:row.summary?.lineage_promoted,audits:row.summary?.lineage_audits,
  disagreed:row.summary?.lineage_disagreed,revoked:row.summary?.lineage_revoked}))
 return row
}
run('plain-before',{plain:true})
run('record');run('witness')
assert.equal(run('warm').summary.replayed,3)
assert.ok(Object.values(ledger().files).every(r=>r.plain_n>=1&&r.w.n>=2))
fs.writeFileSync(input,'revision:1')
const promoted=run('promote')
assert.equal(promoted.summary.lineage_promoted,2)
const promotion=ledger(),batch=Object.entries(promotion.files).filter(([,r])=>r.lineage)
assert.equal(batch.length,2)
const stamp=batch[0][1].promotion_stamp
assert.ok(stamp);assert.ok(batch.every(([,r])=>r.lineage===stamp&&r.promotion_stamp===stamp&&r.w.n===2&&r.plain_n===0))
const [controlId,control]=Object.entries(promotion.files).find(([,r])=>r.file.endsWith('lineage-control.test.mjs'))
assert.equal(control.promotion_stamp,null)
const agreed=run('agreeing-audit')
assert.equal(agreed.summary.lineage_audits,1);assert.equal(agreed.summary.lineage_disagreed,0)
const before=ledger(),pending=batch.map(([id])=>id).filter(id=>before.files[id].lineage===stamp)
assert.equal(pending.length,1)
const pendingId=pending[0],auditedId=batch.find(([id])=>id!==pendingId)[0]
assert.equal(before.files[auditedId].lineage,null)
assert.ok(before.files[auditedId].plain_n>=1)
assert.equal(before.files[auditedId].promotion_stamp,stamp)

// Model one erroneous stored signature. All input states, actual results,
// witness counts, promotion stamps and file contents remain unchanged.
const oldTests=before.files[pendingId].w.tests
const injected='0'.repeat(64);assert.notEqual(oldTests,injected)
// Keep every other byte, including integer nanosecond identities that a
// JSON.parse/stringify round trip would round through JavaScript Numbers.
const original=fs.readFileSync(ledgerPath,'utf8'),parts=original.split(JSON.stringify(oldTests))
assert.equal(parts.length,2,'the selected signature must occur exactly once')
const corrupted=parts.join(JSON.stringify(injected))
fs.writeFileSync(ledgerPath,corrupted)
fs.copyFileSync(ledgerPath,path.join(out,'fault-injected-ledger.json'))
fs.writeFileSync(path.join(out,'fault-injection.json'),JSON.stringify({pendingId,auditedId,controlId,stamp,
 oldTests,injected,originalSha256:digest(original),injectedSha256:digest(corrupted),
 scope:'Only the pending record witness signature was deliberately corrupted'},null,2)+'\n')
const bad=run('disagreeing-audit')
assert.equal(bad.summary.lineage_audits,1)
assert.equal(bad.summary.lineage_disagreed,1)
assert.equal(bad.summary.lineage_revoked,2)
const revoked=ledger()
for(const [id] of batch) {
 const r=revoked.files[id]
 assert.equal(r.w.n,1);assert.equal(r.plain_n,0)
 assert.equal(r.lineage,null);assert.equal(r.promotion_stamp,null)
}
assert.equal(revoked.files[pendingId].quarantined,true)
assert.equal(revoked.files[auditedId].quarantined,false)
assert.equal(revoked.files[controlId].w.n,before.files[controlId].w.n)
assert.equal(revoked.files[controlId].plain_n,before.files[controlId].plain_n)
assert.equal(revoked.files[controlId].quarantined,false)
const next=run('after-revocation')
assert.equal(next.summary.replayed,1,'only the independent control may replay after batch revocation')
assert.equal(next.summary.ran,2)
run('plain-after',{plain:true})
const result={scope:'Synthetic witness-signature fault; real binary promotion, audit and batch revocation',
 sourceOverride:false,binarySha256:digest(fs.readFileSync(binary)),node:process.version,
 image:[process.env.ImageOS,process.env.ImageVersion].filter(Boolean).join('/'),
 promoted:2,agreeingAudits:1,disagreeingAudits:1,revoked:2,previouslyAuditedPeerRevoked:true,
 disagreedFileQuarantined:true,independentRecordPreserved:true,revokedFilesExecute:true,
 observedTestStatesMatch:true,benchmarkSavingsClaim:false}
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(result,null,2)+'\n')
console.log(JSON.stringify(result))
