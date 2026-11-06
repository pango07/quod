// Real built binary: type-only reads replay across metadata edits, but a
// file/directory transition must execute the newly selected tests.
import fs from 'node:fs'
import path from 'node:path'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const [projectArg,binary] = process.argv.slice(2)
const project=path.resolve(projectArg), out='/tmp/quod-stat-type-audit'
fs.mkdirSync(out,{recursive:true})
const input=path.join(project,'stat-type-input')
fs.writeFileSync(input,'unchanged bytes\n')
fs.chmodSync(input,0o644)
fs.utimesSync(input,1577836800,1577836800)
fs.writeFileSync(path.join(project,'vitest.config.mjs'),`import fs from 'node:fs';
const isFile=fs.statSync('stat-type-input').isFile();
export default {css:{postcss:{}},test:{include:isFile ? ['tests/add.test.js'] : ['tests/*.test.js']}};
`)
const env={...process.env,QUOD_VITEST_NAMES_PROOF:'1',QUOD_VITEST_BOOLEAN_PROOF:'0',QUOD_VITEST_LINEAGE:'0',QUOD_VITEST_TIMING:'off'}
const runs=[]
function run(label,plain=false) {
  const report=path.join(out,label+'-report.json'), summary=path.join(out,label+'-summary.json')
  const args=plain ? ['node','node_modules/vitest/vitest.mjs','run','--reporter=json',`--outputFile=${report}`]
    : [binary,'vitest','--dir',path.join(out,'state'),'--audits','0','--report',report,'--summary',summary,'--','run']
  const r=spawnSync(args[0],args.slice(1),{cwd:project,env,encoding:'utf8',timeout:90000})
  fs.writeFileSync(path.join(out,label+'.log'),(r.stdout??'')+(r.stderr??''))
  assert.equal(r.error,undefined)
  assert.equal(r.status,0,r.stderr)
  const data=JSON.parse(fs.readFileSync(report,'utf8'))
  const states=data.testResults.flatMap(f=>f.assertionResults.map(a=>[path.basename(f.name),a.fullName??a.title,a.status])).sort()
  assert.ok(states.length && states.every(row=>row[1]))
  const row={label,states,summary:plain?null:JSON.parse(fs.readFileSync(summary,'utf8'))}
  runs.push(row);fs.writeFileSync(path.join(out,'runs.json'),JSON.stringify(runs,null,2))
  console.log(JSON.stringify({label,states,replayed:row.summary?.replayed}))
  return row
}
const baseline=run('plain-before',true)
assert.equal(baseline.states.length,1)
for(const label of ['record','confirm','warm']) assert.deepEqual(run(label).states,baseline.states)
assert.equal(runs.at(-1).summary.replayed,1)
const bytes=fs.readFileSync(input)
fs.chmodSync(input,0o755);fs.utimesSync(input,1609459200,1609459200)
assert.deepEqual(fs.readFileSync(input),bytes)
const metadata=run('metadata-only-change')
assert.equal(metadata.summary.replayed,1,'proved type does not depend on times or permissions')
assert.deepEqual(metadata.states,run('metadata-only-plain',true).states)
fs.unlinkSync(input);fs.mkdirSync(input)
const changed=run('file-became-directory'), plain=run('directory-plain',true)
assert.equal(changed.summary.replayed,0,'changed shared type must rerun selection')
assert.equal(plain.states.length,2)
assert.deepEqual(changed.states,plain.states)
// Node's public Stats wrapper shares the native constructor's original
// prototype. Replacing the internal constructor's prototype can turn an
// apparent isFile() query into a metadata read without changing that wrapper.
fs.rmdirSync(input);fs.writeFileSync(input,'unchanged bytes\n')
fs.utimesSync(input,1577836800,1577836800)
fs.writeFileSync(path.join(project,'vitest.config.mjs'),`import fs from 'node:fs';
const Ctor=fs.Stats.prototype.constructor, original=Ctor.prototype;
Ctor.prototype=Object.create(original);
Ctor.prototype.isFile=function(){return this.mtimeMs<1600000000000};
let isFile;
try { isFile=fs.statSync('stat-type-input').isFile(); }
finally { Ctor.prototype=original; }
export default {css:{postcss:{}},test:{include:isFile ? ['tests/add.test.js'] : ['tests/*.test.js']}};
`)
const prototypeBefore=run('prototype-plain-before',true)
assert.equal(prototypeBefore.states.length,1)
for(const label of ['prototype-record','prototype-confirm','prototype-warm']) assert.deepEqual(run(label).states,prototypeBefore.states)
assert.equal(runs.at(-1).summary.replayed,1)
fs.utimesSync(input,1609459200,1609459200)
const prototypeChanged=run('prototype-metadata-change'),prototypePlain=run('prototype-metadata-plain',true)
assert.equal(prototypeChanged.summary.replayed,0,'internal constructor replacement must retain metadata')
assert.equal(prototypePlain.states.length,2)
assert.deepEqual(prototypeChanged.states,prototypePlain.states)
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify({wrong:0,metadataReplayed:metadata.summary.replayed,
  typeChangeReplayed:changed.summary.replayed,selectedTests:plain.states.length,
  prototypeMetadataReplayed:prototypeChanged.summary.replayed,prototypeSelectedTests:prototypePlain.states.length},null,2))
