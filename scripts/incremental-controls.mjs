// Executes only in GitHub Actions. Compare actual restored compiler state
// against a clean compiler on deliberate errors and input changes.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';

const compiler = path.resolve('cloud/node_modules/typescript/bin/tsc');
const base = fs.mkdtempSync(path.join(os.tmpdir(), 'quod-incremental-controls-'));
const initial = {
  'tsconfig.json': JSON.stringify({compilerOptions: {strict: true, incremental: true, noEmit: true,
    types: [], skipLibCheck: true}, include: ['src/**/*.ts', 'generated/**/*.ts']}),
  'src/api.ts': 'export const value: number = 42;\n',
  'src/use.ts': 'import {value} from "./api"; export const result: number = value;\n',
  'generated/schema.d.ts': 'export type Value = number;\n',
  'src/generated.ts': 'import type {Value} from "../generated/schema"; export const result: Value = 42;\n',
};
const write = (root, file, body) => {
  fs.mkdirSync(path.dirname(path.join(root, file)), {recursive: true});
  fs.writeFileSync(path.join(root, file), body);
};
const compile = root => {
  const result = spawnSync(process.execPath, [compiler, '--pretty', 'false'],
    {cwd: root, encoding: 'utf8', timeout: 60000, maxBuffer: 1024 * 1024});
  assert.equal(result.error, undefined);
  assert.equal(result.signal, null);
  return {status: result.status, stdout: result.stdout, stderr: result.stderr};
};
const cases = [
  ['unchanged', () => {}, false],
  ['implementation-change', r => write(r, 'src/api.ts', 'export const value: number = 43;\n'), false],
  ['dependency-type-change', r => write(r, 'src/api.ts', 'export const value: string = "changed";\n'), true],
  ['new-file-error', r => write(r, 'src/new.ts', 'export const invalid: number = "wrong";\n'), true],
  ['deleted-dependency', r => fs.unlinkSync(path.join(r, 'src/api.ts')), true],
  ['generated-type-change', r => write(r, 'generated/schema.d.ts', 'export type Value = string;\n'), true],
  ['config-change', r => {
    const cfg = JSON.parse(initial['tsconfig.json']); cfg.compilerOptions.noUnusedLocals = true;
    write(r, 'tsconfig.json', JSON.stringify(cfg)); write(r, 'src/unused.ts', 'export {}; const unused = 1;\n');
  }, true],
];
try {
  for (const [name, mutate, fails] of cases) {
    const warm = path.join(base, name, 'warm'), cold = path.join(base, name, 'cold');
    for (const [file, body] of Object.entries(initial)) write(warm, file, body);
    assert.equal(compile(warm).status, 0, name + ' seed');
    assert(fs.statSync(path.join(warm, 'tsconfig.tsbuildinfo')).size > 0);
    mutate(warm);
    fs.cpSync(warm, cold, {recursive: true, filter: file => !file.endsWith('.tsbuildinfo')});
    const restored = compile(warm), plain = compile(cold);
    assert.deepEqual(restored, plain, name + ' differs with restored compiler state');
    assert.equal(restored.status !== 0, fails, name + ' expected diagnostic');
    console.log(JSON.stringify({control: name, agrees: true, rejected: fails}));
  }
} finally { fs.rmSync(base, {recursive: true, force: true}); }
