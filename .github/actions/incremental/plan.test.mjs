import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {plan} from './plan.mjs';

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'quod-plan-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  const write = (name, value) => fs.writeFileSync(path.join(root, name), typeof value === 'string' ? value : JSON.stringify(value));
  write('package.json', {devDependencies: {typescript: '5.9.3'}, dependencies: {next: '16.4.3'}});
  write('package-lock.json', {lockfileVersion: 3, packages: {'node_modules/typescript': {version: '5.9.3'}, 'node_modules/next': {version: '16.4.3'}}});
  write('tsconfig.json', {compilerOptions: {incremental: true, noEmit: true}});
  const run = (extra = {}) => plan({workspace: root, revision: 'a'.repeat(40),
    runtime: {node: 'v24.22.2', platform: 'linux', arch: 'x64'}, ...extra});
  return {root, write, run};
}

test('portable state scope survives source changes; native compiler owns invalidation', t => {
  const {write, run} = fixture(t), before = run();
  assert.equal(before.eligible, true);
  write('example.ts', 'const invalid: string = 1');
  const after = run({revision: 'b'.repeat(40)});
  assert.equal(before.prefix, after.prefix);
  assert.notEqual(before.key, after.key);
  assert.equal(before.bytesRead, after.bytesRead);
});
test('lock, runtime, compiler config, project and namespace isolate states', t => {
  const {write, run} = fixture(t), before = run();
  assert.notEqual(run({namespace: 'other'}).prefix, before.prefix);
  assert.notEqual(run({runtime: {node: 'v24.23.0', platform: 'linux', arch: 'x64'}}).prefix, before.prefix);
  write('tsconfig.json', {compilerOptions: {incremental: true, noEmit: true, strict: true}});
  assert.notEqual(run().prefix, before.prefix);
  const config = run();
  write('package-lock.json', {lockfileVersion: 3, packages: {'node_modules/typescript': {version: '6.0.0'}}});
  assert.notEqual(run().prefix, config.prefix);
});
test('Next native cache is explicit and never includes final outputs or node_modules', t => {
  const {root, run} = fixture(t);
  assert.deepEqual(run().paths, [path.join(root, 'tsconfig.tsbuildinfo')]);
  assert.deepEqual(run({next: true}).paths, [path.join(root, 'tsconfig.tsbuildinfo'), path.join(root, '.next/cache')]);
});
test('unsupported TS layouts decline without guessing build-info locations', t => {
  const {write, run} = fixture(t);
  for (const value of [
    {extends: './base.json', compilerOptions: {incremental: true, noEmit: true}},
    {compilerOptions: {incremental: true, noEmit: true, tsBuildInfoFile: '../outside'}},
    {compilerOptions: {incremental: true, noEmit: false}},
    '// JSONC is deliberately unsupported\n{}',
  ]) { write('tsconfig.json', value); assert.equal(run().eligible, false); }
});
test('symlinked cache ancestor and descriptors cannot redirect extraction', t => {
  const {root, run} = fixture(t);
  fs.symlinkSync(os.tmpdir(), path.join(root, '.next'));
  assert.equal(run({next: true}).eligible, false);
  fs.unlinkSync(path.join(root, '.next'));
  fs.unlinkSync(path.join(root, 'tsconfig.json'));
  fs.symlinkSync('/does-not-exist', path.join(root, 'tsconfig.json'));
  assert.equal(run().eligible, false);
});
test('existing state, malformed scope and unsafe project paths bypass', t => {
  const {root, write, run} = fixture(t);
  for (const directory of ['../outside', '/tmp', 'a/../b', 'x\ny']) assert.equal(run({directory}).eligible, false);
  assert.equal(run({namespace: 'x\ny'}).eligible, false);
  assert.equal(run({revision: 'branch'}).eligible, false);
  write('tsconfig.tsbuildinfo', '{}');
  assert.equal(run().eligible, false);
  fs.unlinkSync(path.join(root, 'tsconfig.tsbuildinfo'));
  write('yarn.lock', 'other manager');
  assert.equal(run().eligible, false);
});
