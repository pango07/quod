// Remote-only regression: changing a parent link changes realpath's result
// without changing the final file's bytes, type or direct link target.
import fs from 'node:fs'
import path from 'node:path'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const [projectArg,binaryArg,outArg,expected,sourceArg]=process.argv.slice(2)
assert.ok(['stale','correct'].includes(expected))
const project=path.resolve(projectArg),binary=path.resolve(binaryArg),out=path.resolve(outArg)
assert.equal(fs.existsSync(out),false);fs.mkdirSync(out,{recursive:true})
for(const dir of ['realpath-a','realpath-b']) {
  fs.mkdirSync(path.join(project,dir),{recursive:true})
  fs.writeFileSync(path.join(project,dir,'input'),'identical bytes\n')
}
const link=path.join(project,'realpath-current')
fs.rmSync(link,{force:true});fs.symlinkSync('realpath-a',link)
const operations={
  sync:`fs.realpathSync(input)`,
  native_sync:`fs.realpathSync.native(input)`,
  callback:`await new Promise((resolve,reject)=>fs.realpath(input,(e,p)=>e?reject(e):resolve(p)))`,
  native_callback:`await new Promise((resolve,reject)=>fs.realpath.native(input,(e,p)=>e?reject(e):resolve(p)))`,
  promise:`await fs.promises.realpath(input)`,
}
const files=[]
for(const [name,expression] of Object.entries(operations)) {
  const file=`realpath-${name}.test.mjs`;files.push(file)
  fs.writeFileSync(path.join(project,file),`import fs from 'node:fs';import {test,expect} from 'vitest';
const input='realpath-current/input';
test(${JSON.stringify(name)},async()=>expect(${expression}).toMatch(/\\/realpath-a\\/input$/));\n`)
}
files.push('realpath-control.test.mjs')
fs.writeFileSync(path.join(project,files.at(-1)),`import fs from 'node:fs';import {test,expect} from 'vitest';
test('content control',()=>expect(fs.readFileSync('realpath-current/input','utf8')).toBe('identical bytes\\n'));\n`)
fs.writeFileSync(path.join(project,'vitest.config.mjs'),`export default {css:{postcss:{}},test:{include:${JSON.stringify(files)},maxWorkers:1}};\n`)
const env={...process.env,QUOD_VITEST_LINEAGE:'0',QUOD_VITEST_TIMING:'off',
  QUOD_VITEST_NAMES_PROOF:'0',QUOD_VITEST_BOOLEAN_PROOF:'0'}
delete env.QUOD_VITEST_JS_DIR
if(sourceArg)env.QUOD_VITEST_JS_DIR=path.resolve(sourceArg)
const runs=[],state=path.join(out,'state')
function run(label,plain=false) {
  const report=path.join(out,label+'-report.json'),summary=path.join(out,label+'-summary.json')
  const args=plain?[process.execPath,'node_modules/vitest/vitest.mjs','run','--reporter=json','--outputFile.json='+report]:
    [binary,'vitest','--dir',state,'--audits','0','--report',report,'--summary',summary,'--','run']
  const result=spawnSync(args[0],args.slice(1),{cwd:project,env,encoding:'utf8',timeout:90000,maxBuffer:8*1024*1024})
  fs.writeFileSync(path.join(out,label+'.log'),(result.stdout??'')+(result.stderr??''))
  assert.equal(result.error,undefined);assert.ok(result.status===0||result.status===1)
  const data=JSON.parse(fs.readFileSync(report,'utf8'))
  const states=data.testResults.flatMap(f=>f.assertionResults.map(a=>[a.fullName??a.title,a.status])).sort()
  assert.equal(data.testResults.length,6);assert.equal(states.length,6)
  const row={label,exit:result.status,states,summary:plain?null:JSON.parse(fs.readFileSync(summary,'utf8'))}
  runs.push(row);fs.writeFileSync(path.join(out,'runs.json'),JSON.stringify(runs,null,2)+'\n')
  console.log(JSON.stringify({label,exit:row.exit,states,replayed:row.summary?.replayed}))
  return row
}
const initial=run('plain-before',true)
assert.equal(initial.exit,0);assert.ok(initial.states.every(([,s])=>s==='passed'))
for(const label of ['record','witness','warm']) {
  const row=run(label);assert.equal(row.exit,0);assert.deepEqual(row.states,initial.states)
}
const warm=runs.at(-1).summary.replayed
assert.ok(warm>=1,'at least the content control must replay before mutation')
fs.unlinkSync(link);fs.symlinkSync('realpath-b',link)
const changed=run('changed-quod'),plain=run('changed-plain',true)
assert.equal(plain.exit,1);assert.equal(plain.states.filter(([,s])=>s==='failed').length,5)
const wrong=changed.states.filter((s,i)=>JSON.stringify(s)!==JSON.stringify(plain.states[i]))
const evidence={expected,wrong:wrong.length,wrongTests:wrong.map(([name])=>name),warmReplayed:warm,
  changedReplayed:changed.summary.replayed,quodExit:changed.exit,plainExit:plain.exit,
  sourceOverride:!!sourceArg,mutation:'Only a parent symlink changes; final file type and contents remain equal'}
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(evidence,null,2)+'\n')
console.log(JSON.stringify(evidence))
if(expected==='stale')assert.ok(wrong.length>0,'require a reproduced stale replay')
else {assert.equal(wrong.length,0);assert.deepEqual(changed.states,plain.states);assert.equal(changed.exit,plain.exit)}
