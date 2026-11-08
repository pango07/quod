// Full native Cloud commands, identically timed in both arms. No command
// result is cached by Quod. GitHub job lifetime includes action/post costs.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import os from 'node:os';
import {spawnSync} from 'node:child_process';

const mode = process.env.INCREMENTAL_ARM;
const phase = process.env.INCREMENTAL_PHASE;
assert(['plain', 'quod'].includes(mode));
assert(['cold', 'warm', 'changed'].includes(phase));
const root = path.resolve('cloud');
const output = path.join(process.env.RUNNER_TEMP, 'incremental-result.json');
const hash = body => crypto.createHash('sha256').update(body).digest('hex');
const state = () => {
  const file = path.join(root, 'tsconfig.tsbuildinfo');
  return fs.existsSync(file) ? {bytes: fs.statSync(file).size, sha256: hash(fs.readFileSync(file))} : null;
};
const before = state();
const nextBefore = fs.existsSync(path.join(root, '.next/cache'));
if (mode === 'plain' || phase === 'cold') {
  assert.equal(before, null, 'cold/plain has unexpected compiler state');
  assert.equal(nextBefore, false, 'cold/plain has unexpected build state');
} else {
  assert(before && nextBefore, 'fresh runner did not restore both native caches');
}
// Change an actual build input, not an unused file. Both arms must produce
// the new metadata even when one restored the previous production cache.
const marker = 'Quod incremental invalidation control 43';
if (phase === 'changed') {
  const layout = path.join(root, 'app/layout.tsx');
  const source = fs.readFileSync(layout, 'utf8');
  const old = 'description: "See which CI steps Quod replayed, why, and how much time it saved."';
  assert.equal(source.split(old).length, 2, 'fixed workload layout differs');
  fs.writeFileSync(layout, source.replace(old, `description: "${marker}"`));
}
const report = {schema: 'quod-incremental-ci-v1', mode, phase, source: process.env.GITHUB_SHA,
  runId: process.env.GITHUB_RUN_ID, attempt: process.env.GITHUB_RUN_ATTEMPT,
  host: {cpu: os.cpus()[0]?.model, cores: os.cpus().length, memoryBytes: os.totalmem(),
    imageVersion: process.env.ImageVersion, node: process.version},
  compilerStateBefore: before, nextStateBefore: nextBefore, commands: [], status: 'running'};
const commands = ['npm ci', 'npm run lint', 'npx next typegen && npx tsc --noEmit',
  'node scripts/vitest-quod.mjs', 'npm run build'];
try {
  for (const command of commands) {
    console.log(`::group::${command}`);
    const start = performance.now();
    const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', command],
      {cwd: root, stdio: 'inherit', timeout: 600000});
    report.commands.push({command, elapsedMs: performance.now() - start, exitCode: result.status});
    console.log('::endgroup::');
    assert.equal(result.error, undefined, command);
    assert.equal(result.status, 0, command);
  }
  report.compilerStateAfter = state();
  assert(report.compilerStateAfter, 'compiler did not persist native state');
  if (phase === 'changed') {
    const server = path.join(root, '.next/server');
    const files = fs.readdirSync(server, {recursive: true}).filter(file => file.endsWith('.js'));
    report.changedOutputPresent = files.some(file => fs.readFileSync(path.join(server, file), 'utf8').includes(marker));
    assert.equal(report.changedOutputPresent, true, 'build returned stale layout metadata');
  }
  const diff = spawnSync('git', ['diff', '--name-only', '-z'], {encoding: 'utf8'});
  assert.equal(diff.status, 0);
  assert.deepEqual(diff.stdout.split('\0').filter(Boolean), phase === 'changed' ? ['cloud/app/layout.tsx'] : [],
    'native pipeline unexpectedly modified tracked source');
  report.status = 'success';
} finally {
  fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report));
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY,
    `### Native incremental experiment: ${phase} / ${mode}\n\n` +
    `All original commands execute. Cache restore is not test replay. ` +
    `Use complete GitHub job lifetimes, including cache save, for comparisons.\n\n` +
    '| Command | Seconds |\n|---|---:|\n' +
    report.commands.map(c => `| ${c.command} | ${(c.elapsedMs / 1000).toFixed(3)} |`).join('\n') + '\n');
}
