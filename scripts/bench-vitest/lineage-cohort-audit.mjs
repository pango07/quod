// Correlated recorder-fault recovery: omit one dependency from two genuinely
// promoted records, then change that input so both tests fail. No natural
// recorder omission is claimed; the fault is deliberately injected.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const [projectArg,binaryArg,outArg,expected='correct']=process.argv.slice(2)
assert.ok(['stale','correct','recovery-failure'].includes(expected))
const project=path.resolve(projectArg),binary=path.resolve(binaryArg),out=path.resolve(outArg)
assert.equal(fs.existsSync(out),false);fs.mkdirSync(out,{recursive:true})
const input=path.join(out,'cohort-input'),state=path.join(out,'state'),ledgerPath=path.join(state,'ledger.json')
const digest=data=>crypto.createHash('sha256').update(data).digest('hex')
const ledger=()=>JSON.parse(fs.readFileSync(ledgerPath,'utf8')),rows=[]
fs.writeFileSync(input,'revision:0')
for(const name of ['a','b'])fs.writeFileSync(path.join(project,`cohort-${name}.test.mjs`),`
import fs from 'node:fs';import {test,expect} from 'vitest';
test('cohort ${name}',()=>expect(fs.readFileSync(${JSON.stringify(input)},'utf8').startsWith('revision:')).toBe(true));
`)
fs.writeFileSync(path.join(project,'cohort-control.test.mjs'),`
import {test,expect} from 'vitest';test('independent control',()=>expect(2+2).toBe(4));
`)
fs.writeFileSync(path.join(project,'vitest.config.mjs'),`export default {
 css:{postcss:{}},test:{include:['cohort-a.test.mjs','cohort-b.test.mjs','cohort-control.test.mjs'],maxWorkers:1}
};\n`)
const env={...process.env,QUOD_VITEST_LINEAGE:'1',QUOD_VITEST_TIMING:'off',
 QUOD_VITEST_PLAIN_WITNESS:'1',QUOD_VITEST_WITNESSES:'2',QUOD_VITEST_RECORDING:'off',
 QUOD_VITEST_NAMES_PROOF:'0',QUOD_VITEST_BOOLEAN_PROOF:'0'}
delete env.QUOD_VITEST_JS_DIR
const shim=path.join(out,'node-recovery-failure')
// This executable only fails the named recovery launch, while forwarding
// every ordinary invocation (including node --version) unchanged.
if(expected==='recovery-failure')fs.writeFileSync(shim,`#!/usr/bin/env python3
import os,sys
if any(os.path.basename(x)=='plan-lineage-recover.json' for x in sys.argv[1:]):sys.exit(75)
os.execv(${JSON.stringify(process.execPath)},[${JSON.stringify(process.execPath)},*sys.argv[1:]])
`,{mode:0o755})
function run(label,{plain=false,failRecovery=false}={}) {
 const report=path.join(out,label+'-report.json'),summary=path.join(out,label+'-summary.json')
 const args=plain?[process.execPath,'node_modules/vitest/vitest.mjs','run','--reporter=json','--outputFile.json='+report]:
  [binary,'vitest','--dir',state,'--no-fast','--no-narrow','--audits','0','--report',report,'--summary',summary,
   ...(failRecovery?['--node',shim]:[]),'--','run']
 const result=spawnSync(args[0],args.slice(1),{cwd:project,env,encoding:'utf8',timeout:90000,maxBuffer:8*1024*1024})
 fs.writeFileSync(path.join(out,label+'.log'),(result.stdout??'')+(result.stderr??''))
 assert.equal(result.error,undefined)
 if(failRecovery) {
  assert.notEqual(result.status,0);assert.equal(fs.existsSync(report),false,'a failed recovery must remove the stale combined report')
  fs.copyFileSync(ledgerPath,path.join(out,label+'-ledger.json'))
  const data=JSON.parse(fs.readFileSync(summary))
  assert.equal(data.lineage_recovery_failed,true);assert.equal(data.exit,result.status)
  return {exit:result.status,states:null,summary:data}
 }
 assert.ok(result.status===0||result.status===1)
 const bytes=fs.readFileSync(report),data=JSON.parse(bytes)
 const states=data.testResults.map(f=>[path.basename(f.name),f.assertionResults.map(a=>[a.fullName??a.title,a.status]).sort()]).sort()
 assert.equal(states.length,3);assert.equal(states.reduce((n,s)=>n+s[1].length,0),3)
 if(!plain)fs.copyFileSync(ledgerPath,path.join(out,label+'-ledger.json'))
 const row={label,exit:result.status,states,reportSha256:digest(bytes),summary:plain?null:JSON.parse(fs.readFileSync(summary,'utf8'))}
 rows.push(row);fs.writeFileSync(path.join(out,'runs.json'),JSON.stringify(rows,null,2)+'\n')
 console.log(JSON.stringify({label,exit:row.exit,states,ran:row.summary?.ran,replayed:row.summary?.replayed,
  audits:row.summary?.lineage_audits,disagreed:row.summary?.lineage_disagreed,revoked:row.summary?.lineage_revoked,
  recovery:row.summary?.lineage_recovery}))
 return row
}
const beforePlain=run('plain-before',{plain:true})
assert.equal(beforePlain.exit,0)
for(const label of ['record','witness','warm'])assert.deepEqual(run(label).states,beforePlain.states)
assert.equal(rows.at(-1).summary.replayed,3)
fs.writeFileSync(input,'revision:1')
assert.equal(run('promote').summary.lineage_promoted,2)
const promoted=ledger(),batch=Object.entries(promoted.files).filter(([,r])=>r.lineage)
assert.equal(batch.length,2)
const stamp=batch[0][1].promotion_stamp
assert.ok(stamp);assert.ok(batch.every(([,r])=>r.promotion_stamp===stamp))
assert.equal(run('agreeing-audit').summary.lineage_audits,1)
const before=ledger(),pending=batch.map(([id])=>id).filter(id=>before.files[id].lineage===stamp)
assert.equal(pending.length,1)
const auditedId=batch.find(([id])=>id!==pending[0])[0]
assert.equal(before.files[auditedId].lineage,null);assert.ok(before.files[auditedId].plain_n>=1)
const keyId=before.keys.indexOf('f:'+input)
assert.ok(keyId>=0);assert.equal(before.global.includes(keyId),false,'shared inputs must not mask the injected per-file fault')
const original=fs.readFileSync(ledgerPath,'utf8');let injected=original
for(const [id] of batch) {
 const deps=before.files[id].deps
 assert.ok(deps.includes(keyId))
 const field='"deps":'+JSON.stringify(deps),parts=injected.split(field)
 assert.equal(parts.length,2,'each dependency vector must occur exactly once')
 injected=parts.join('"deps":'+JSON.stringify(deps.filter(k=>k!==keyId)))
}
fs.writeFileSync(ledgerPath,injected);fs.copyFileSync(ledgerPath,path.join(out,'fault-injected-ledger.json'))
fs.writeFileSync(path.join(out,'fault-injection.json'),JSON.stringify({batch:batch.map(([id])=>id),pendingId:pending[0],auditedId,stamp,keyId,
 originalSha256:digest(original),injectedSha256:digest(injected),scope:'One input dependency omitted from each of two promoted records; all other ledger bytes retained'},null,2)+'\n')
fs.writeFileSync(input,'defect')
const bad=run('fault-quod',{failRecovery:expected==='recovery-failure'}),plain=run('fault-plain',{plain:true})
assert.equal(plain.exit,1)
assert.equal(plain.states.flatMap(([,s])=>s).filter(([,s])=>s==='failed').length,2)
const after=ledger()
for(const [id] of batch) {
 assert.equal(after.files[id].w.n,1);assert.equal(after.files[id].plain_n,0)
 assert.equal(after.files[id].promotion_stamp,null);assert.equal(after.files[id].lineage,null)
}
const wrong=bad.states?bad.states.filter((s,i)=>JSON.stringify(s)!==JSON.stringify(plain.states[i])).length:null
if(expected==='stale') {
 assert.equal(wrong,1);assert.equal(bad.exit,1)
 assert.equal(bad.summary.lineage_disagreed,1);assert.equal(bad.summary.lineage_revoked,2)
} else if(expected==='correct') {
 assert.equal(wrong,0);assert.equal(bad.exit,plain.exit)
 assert.equal(bad.summary.replayed,1);assert.equal(bad.summary.ran,2)
 assert.equal(bad.summary.lineage_revoked,2)
 for(const [id] of batch)assert.equal(after.files[id].quarantined,true)
} else {
 assert.notEqual(bad.exit,0);assert.equal(bad.states,null)
}
const result={scope:'Injected correlated dependency omission; actual same-job results compared with plain',expected,
 binarySha256:digest(fs.readFileSync(binary)),sourceOverride:false,node:process.version,
 image:[process.env.ImageOS,process.env.ImageVersion].filter(Boolean).join('/'),wrong,
 revoked:2,auditedPeerIncluded:true,quodExit:bad.exit,plainExit:plain.exit,benchmarkSavingsClaim:false}
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify(result))
