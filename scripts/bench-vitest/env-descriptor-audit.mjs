// Real packaged replay, then a value-only environment change. The old
// packaged binary must demonstrate the regression before the staged fix
// is accepted. No source/test files change between training and mutation.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';

const [projectArg, binaryArg, outArg, expected, sourceArg] = process.argv.slice(2);
assert.ok(['stale', 'correct'].includes(expected));
const project = path.resolve(projectArg), binary = path.resolve(binaryArg), out = path.resolve(outArg);
assert.equal(fs.existsSync(out), false, 'audit output must be fresh');
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(path.join(project, 'vitest.config.mjs'), `export default {
  css: {postcss:{}}, test: {include:['env-descriptor.test.mjs'], maxWorkers:1}
};\n`);
fs.writeFileSync(path.join(project, 'env-descriptor.test.mjs'), `
import {test, expect} from 'vitest';
test('Object descriptor value', () => {
  expect(Object.getOwnPropertyDescriptor(process.env, 'CACHE_DESCRIPTOR_INPUT').value).toBe('before');
});
test('Reflect descriptor value', () => {
  expect(Reflect.getOwnPropertyDescriptor(process.env, 'CACHE_DESCRIPTOR_INPUT').value).toBe('before');
});
`);
const state = path.join(out, 'state');
const env = { ...process.env, QUOD_VITEST_LINEAGE:'0', QUOD_VITEST_TIMING:'off',
  QUOD_VITEST_NAMES_PROOF:'0', QUOD_VITEST_BOOLEAN_PROOF:'0' };
delete env.QUOD_VITEST_JS_DIR;
if (sourceArg) env.QUOD_VITEST_JS_DIR = path.resolve(sourceArg);
const runs = [];
function run(label, input, plain = false) {
  const report = path.join(out, label + '-report.json'), summary = path.join(out, label + '-summary.json');
  const args = plain
    ? [process.execPath, 'node_modules/vitest/vitest.mjs', 'run', '--reporter=json', '--outputFile.json=' + report]
    : [binary, 'vitest', '--dir', state, '--audits', '0', '--report', report, '--summary', summary, '--', 'run'];
  const result = spawnSync(args[0], args.slice(1), {cwd:project, env:{...env,CACHE_DESCRIPTOR_INPUT:input},
    encoding:'utf8', timeout:90000, maxBuffer:8*1024*1024});
  fs.writeFileSync(path.join(out,label+'.log'), (result.stdout??'')+(result.stderr??''));
  assert.equal(result.error, undefined);
  assert.ok(result.status === 0 || result.status === 1, 'unexpected process exit');
  const data = JSON.parse(fs.readFileSync(report, 'utf8'));
  const states = data.testResults.flatMap(f=>f.assertionResults.map(a=>[a.fullName??a.title,a.status])).sort();
  assert.equal(states.length,2,'missing test evidence');
  const row = {label,exit:result.status,states,summary:plain?null:JSON.parse(fs.readFileSync(summary,'utf8'))};
  runs.push(row);
  fs.writeFileSync(path.join(out,'runs.json'),JSON.stringify(runs,null,2)+'\n');
  console.log(JSON.stringify({label,exit:row.exit,states,replayed:row.summary?.replayed}));
  return row;
}
const initial = run('before-plain','before',true);
assert.equal(initial.exit,0);
for (const label of ['record','confirm','warm']) {
  const row = run(label,'before');
  assert.equal(row.exit,0);assert.deepEqual(row.states,initial.states);
}
assert.equal(runs.at(-1).summary.replayed,1,'exercise a real hit before changing the input');
const changed = run('changed-quod','after'), plain = run('changed-plain','after',true);
assert.equal(plain.exit,1);
assert.ok(plain.states.every(([,s])=>s==='failed'));
const wrong = changed.states.filter((s,i)=>JSON.stringify(s)!==JSON.stringify(plain.states[i])).length;
const evidence = {expected,wrong,replayed:changed.summary.replayed,quodExit:changed.exit,plainExit:plain.exit,
  mutation:'Only CACHE_DESCRIPTOR_INPUT changes from before to after',sourceOverride:!!sourceArg};
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(evidence,null,2)+'\n');
if (expected==='stale') {
  assert.equal(wrong,2,'the baseline must demonstrate the stale replay');
  assert.equal(changed.summary.replayed,1);assert.equal(changed.exit,0);
} else {
  assert.equal(wrong,0,'descriptor value changes must invalidate replay');
  assert.equal(changed.summary.replayed,0);assert.equal(changed.exit,plain.exit);
}
