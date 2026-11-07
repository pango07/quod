// Remote integration gate: admission may avoid recording, never execution.
import fs from 'node:fs'
import path from 'node:path'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const [projectArg,binaryArg,outArg]=process.argv.slice(2)
const project=path.resolve(projectArg),binary=path.resolve(binaryArg),out=path.resolve(outArg)
assert.equal(fs.existsSync(out),false);fs.mkdirSync(out,{recursive:true})
const shared=path.join(out,'shared-input'),input=path.join(out,'test-input'),state=path.join(out,'state')
fs.writeFileSync(shared,'revision:0');fs.writeFileSync(input,'good')
fs.writeFileSync(path.join(project,'admission.test.mjs'),`import fs from 'node:fs';import {test,expect} from 'vitest';
test('changed input must execute',()=>expect(fs.readFileSync(${JSON.stringify(input)},'utf8')).toBe('good'));\n`)
fs.writeFileSync(path.join(project,'admission-control.test.mjs'),`import {test,expect} from 'vitest';test('control',()=>expect(2+2).toBe(4));\n`)
fs.writeFileSync(path.join(project,'vitest.config.mjs'),`import fs from 'node:fs';
if(!fs.readFileSync(${JSON.stringify(shared)},'utf8').startsWith('revision:'))throw Error('bad config input');
export default {css:{postcss:{}},test:{include:['admission.test.mjs','admission-control.test.mjs'],maxWorkers:1}};\n`)
const env={...process.env,QUOD_VITEST_RECORDING:'adaptive',QUOD_VITEST_LINEAGE:'0',
  QUOD_VITEST_TIMING:'off',QUOD_VITEST_NAMES_PROOF:'0',QUOD_VITEST_BOOLEAN_PROOF:'0'}
delete env.QUOD_VITEST_JS_DIR
const rows=[]
const ledger=()=>JSON.parse(fs.readFileSync(path.join(state,'ledger.json'),'utf8'))
const proofs=()=>{const x=ledger();delete x.recording_admission;return x}
function run(label,{plain=false,flags=[],recording='adaptive'}={}) {
  const report=path.join(out,label+'-report.json'),summary=path.join(out,label+'-summary.json')
  const args=plain?[process.execPath,'node_modules/vitest/vitest.mjs','run','--reporter=json','--outputFile.json='+report]:
    [binary,'vitest','--dir',state,'--no-fast','--no-narrow','--audits','0','--report',report,'--summary',summary,...flags,'--','run']
  const result=spawnSync(args[0],args.slice(1),{cwd:project,env:{...env,QUOD_VITEST_RECORDING:recording},encoding:'utf8',timeout:90000,maxBuffer:8*1024*1024})
  fs.writeFileSync(path.join(out,label+'.log'),(result.stdout??'')+(result.stderr??''))
  assert.equal(result.error,undefined);assert.ok(result.status===0||result.status===1)
  const data=JSON.parse(fs.readFileSync(report,'utf8'))
  const states=data.testResults.map(f=>[path.basename(f.name),f.assertionResults.map(a=>[a.fullName??a.title,a.status]).sort()]).sort()
  assert.equal(states.length,2);assert.equal(states.reduce((n,s)=>n+s[1].length,0),2)
  const row={label,exit:result.status,states,summary:plain?null:JSON.parse(fs.readFileSync(summary,'utf8'))}
  rows.push(row);fs.writeFileSync(path.join(out,'runs.json'),JSON.stringify(rows,null,2)+'\n')
  console.log(JSON.stringify({label,exit:row.exit,policy:row.summary?.recording_policy,ran:row.summary?.ran,replayed:row.summary?.replayed}))
  return row
}
const initial=run('plain-before',{plain:true})
assert.equal(initial.exit,0)
for(const name of ['cold','witness','warm'])assert.deepEqual(run(name).states,initial.states)
assert.equal(rows.at(-1).summary.replayed,2,'require actual ordinary replay before churn')
for(let i=1;i<=2;i++) {
  fs.writeFileSync(shared,'revision:'+i)
  const row=run('recorded-miss-'+i)
  assert.equal(row.summary.recording_policy,'record');assert.equal(row.summary.replayed,0)
}
const before=proofs()
for(let i=3;i<=9;i++) {
  fs.writeFileSync(shared,'revision:'+i)
  fs.writeFileSync(input,i===3?'bad':'good')
  const row=run('backoff-'+i)
  assert.equal(row.summary.recording_policy,'adaptive-plain')
  assert.equal(row.summary.ran,2);assert.equal(row.summary.replayed,0)
  assert.deepEqual(proofs(),before,'plain backoff must not refresh any proof')
  if(i===3) {
    const plain=run('plain-changed',{plain:true})
    assert.equal(row.exit,1);assert.equal(plain.exit,1);assert.deepEqual(row.states,plain.states)
  } else {assert.equal(row.exit,0);assert.deepEqual(row.states,initial.states)}
}
fs.writeFileSync(shared,'revision:10')
assert.equal(run('periodic-recording').summary.recording_policy,'record')
fs.writeFileSync(shared,'revision:11')
assert.equal(run('backoff-before-stability').summary.recording_policy,'adaptive-plain')
assert.equal(run('stable-recording').summary.recording_policy,'record')
assert.equal(ledger().recording_admission.misses,0)
assert.equal(run('stable-witness').exit,0)
assert.equal(run('recovered-replay').summary.replayed,2)

// Force just the economic hint into backoff-ready state to isolate override
// checks. This does not fabricate a record's input or result evidence.
function primeHint() {
  const l=ledger();l.recording_admission={shared:'fixture-previous-input',misses:2,skipped:0}
  fs.writeFileSync(path.join(state,'ledger.json'),JSON.stringify(l))
}
primeHint();fs.writeFileSync(shared,'revision:12')
assert.equal(run('forced-full',{flags:['--full']}).summary.recording_policy,'record')
primeHint();fs.writeFileSync(shared,'revision:13')
assert.equal(run('switch-off',{recording:'off'}).summary.recording_policy,'record')
primeHint();fs.writeFileSync(shared,'revision:14')
const explicitBefore=fs.readFileSync(path.join(state,'ledger.json'),'utf8')
assert.equal(run('explicit-no-record',{flags:['--no-record']}).summary.recording_policy,'no-record')
assert.equal(fs.readFileSync(path.join(state,'ledger.json'),'utf8'),explicitBefore)
primeHint()
const pending=ledger();Object.values(pending.files)[0].lineage='fixture-pending-audit'
fs.writeFileSync(path.join(state,'ledger.json'),JSON.stringify(pending))
assert.equal(run('pending-lineage').summary.recording_policy,'record')
const result={sourceOverride:false,wrong:0,backoffRuns:rows.filter(r=>r.summary?.recording_policy==='adaptive-plain').length,
  failedInputExecuted:true,proofsPreserved:true,periodicRetry:true,stableRecovery:true,
  fullOverride:true,disabledOverride:true,explicitNoRecordUnchanged:true,pendingLineagePreventsBackoff:true}
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify(result))
