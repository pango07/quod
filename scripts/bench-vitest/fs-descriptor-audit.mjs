// Real packaged replay with unchanged bytes and changed descriptor metadata.
import fs from 'node:fs'
import path from 'node:path'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const [projectArg,binaryArg,outArg,expected,sourceArg,upgrade] = process.argv.slice(2)
assert.ok(['stale','correct'].includes(expected))
assert.ok(!upgrade||upgrade==='upgrade')
const project=path.resolve(projectArg),binary=path.resolve(binaryArg),out=path.resolve(outArg)
assert.equal(fs.existsSync(out),false);fs.mkdirSync(out,{recursive:true})
const input=path.join(project,'descriptor-input.txt')
fs.writeFileSync(input,'unchanged bytes\n');fs.utimesSync(input,1577836800,1577836800)
fs.writeFileSync(path.join(project,'vitest.config.mjs'),`export default {
 css:{postcss:{}},test:{include:['fd-metadata.test.mjs','fd-control.test.mjs'],maxWorkers:1}
};\n`)
fs.writeFileSync(path.join(project,'fd-metadata.test.mjs'),`
import fs from 'node:fs';import {test,expect} from 'vitest';
const file='descriptor-input.txt';
test('sync descriptor mtime',()=>{const fd=fs.openSync(file,'r');try{
 expect(fs.fstatSync(fd).mtimeMs).toBe(1577836800000);
}finally{fs.closeSync(fd)}});
test('callback descriptor mtime',async()=>{const fd=fs.openSync(file,'r');try{
 const stat=await new Promise((resolve,reject)=>fs.fstat(fd,(e,s)=>e?reject(e):resolve(s)));
 expect(stat.mtimeMs).toBe(1577836800000);
}finally{fs.closeSync(fd)}});
test('FileHandle mtime',async()=>{const handle=await fs.promises.open(file,'r');try{
 expect((await handle.stat()).mtimeMs).toBe(1577836800000);
}finally{await handle.close()}});
`)
fs.writeFileSync(path.join(project,'fd-control.test.mjs'),`
import fs from 'node:fs';import {test,expect} from 'vitest';
test('ordinary content control',()=>expect(fs.readFileSync('descriptor-input.txt','utf8')).toBe('unchanged bytes\\n'));
`)
const state=path.join(out,'state'),runs=[]
const env={...process.env,QUOD_VITEST_LINEAGE:'0',QUOD_VITEST_TIMING:'off',
 QUOD_VITEST_NAMES_PROOF:'0',QUOD_VITEST_BOOLEAN_PROOF:'0'}
delete env.QUOD_VITEST_JS_DIR
function run(label,plain=false,training=false) {
  const report=path.join(out,label+'-report.json'),summary=path.join(out,label+'-summary.json')
  const args=plain?[process.execPath,'node_modules/vitest/vitest.mjs','run','--reporter=json','--outputFile.json='+report]:
    [binary,'vitest','--dir',state,'--audits','0','--report',report,'--summary',summary,'--','run']
  const runEnv={...env}
  if(sourceArg&&!(upgrade&&training))runEnv.QUOD_VITEST_JS_DIR=path.resolve(sourceArg)
  const result=spawnSync(args[0],args.slice(1),{cwd:project,env:runEnv,encoding:'utf8',timeout:90000,maxBuffer:8*1024*1024})
  fs.writeFileSync(path.join(out,label+'.log'),(result.stdout??'')+(result.stderr??''))
  assert.equal(result.error,undefined);assert.ok(result.status===0||result.status===1)
  const data=JSON.parse(fs.readFileSync(report,'utf8'))
  const states=data.testResults.flatMap(f=>f.assertionResults.map(a=>[a.fullName??a.title,a.status])).sort()
  assert.equal(data.testResults.length,2);assert.equal(states.length,4)
  const row={label,exit:result.status,states,summary:plain?null:JSON.parse(fs.readFileSync(summary,'utf8'))}
  runs.push(row);fs.writeFileSync(path.join(out,'runs.json'),JSON.stringify(runs,null,2)+'\n')
  console.log(JSON.stringify({label,exit:row.exit,states,replayed:row.summary?.replayed}))
  return row
}
const before=run('before-plain',true)
assert.equal(before.exit,0);assert.ok(before.states.every(([,s])=>s==='passed'))
for(const label of ['record','confirm','warm']) {
  const r=run(label,false,true);assert.equal(r.exit,0);assert.deepEqual(r.states,before.states)
}
const warm=runs.at(-1).summary.replayed
assert.equal(warm,expected==='stale'||upgrade?2:1,'control must replay; unsupported descriptor tests must run')
const bytes=fs.readFileSync(input);fs.utimesSync(input,1609459200,1609459200)
assert.deepEqual(fs.readFileSync(input),bytes)
const changed=run('changed-quod'),plain=run('changed-plain',true)
assert.equal(plain.exit,1);assert.equal(plain.states.filter(([,s])=>s==='failed').length,3)
const wrong=changed.states.filter((s,i)=>JSON.stringify(s)!==JSON.stringify(plain.states[i])).length
const evidence={expected,wrong,warmReplayed:warm,replayed:changed.summary.replayed,
 quodExit:changed.exit,plainExit:plain.exit,sourceOverride:!!sourceArg,upgrade:!!upgrade,
 mutation:'Only input file timestamps change; contents and test sources stay fixed'}
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(evidence,null,2)+'\n')
if(expected==='stale') {
 assert.equal(wrong,3,'baseline must demonstrate stale descriptor replay')
 assert.equal(changed.exit,0);assert.equal(changed.summary.replayed,2)
}else {
 assert.equal(wrong,0);assert.deepEqual(changed.states,plain.states);assert.equal(changed.exit,plain.exit)
 assert.equal(changed.summary.replayed,upgrade?0:1)
}
