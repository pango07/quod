import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'

function observe(expression, { present = true, value = 'before', enabled = true } = {}) {
  const source = `
import {createRecorder, patchEnv} from ${JSON.stringify(new URL('./record.mjs', import.meta.url).href)};
const original = process.env;
delete original.QUOD_VITEST_HIDE;
${present ? `original.CACHE_DESCRIPTOR_INPUT = ${JSON.stringify(value)};` : 'delete original.CACHE_DESCRIPTOR_INPUT;'}
const r = createRecorder({stateDir:'/nonexistent-quod-state', mode:'worker'});
r.on = ${enabled};
patchEnv(r);
const value = (${expression});
r.on = false;
process.env = original;
console.log(JSON.stringify({value, keys:[...r.keys].filter(k => k.includes('CACHE_DESCRIPTOR_INPUT'))}));
`;
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', source],
    { encoding: 'utf8', timeout: 10000 });
  assert.equal(result.status, 0, result.stderr || String(result.error));
  return JSON.parse(result.stdout);
}

for (const [name, expression] of [
  ['Object descriptor', 'Object.getOwnPropertyDescriptor(process.env, "CACHE_DESCRIPTOR_INPUT").value'],
  ['Reflect descriptor', 'Reflect.getOwnPropertyDescriptor(process.env, "CACHE_DESCRIPTOR_INPUT").value'],
  ['bulk descriptors', 'Object.getOwnPropertyDescriptors(process.env).CACHE_DESCRIPTOR_INPUT.value'],
  ['direct read', 'process.env.CACHE_DESCRIPTOR_INPUT'],
]) test(`${name} retains the observable value dependency`, () => {
  const before = observe(expression);
  const after = observe(expression, { value: 'after' });
  assert.equal(before.value, 'before');
  assert.equal(after.value, 'after');
  assert.deepEqual(before.keys, ['env:CACHE_DESCRIPTOR_INPUT']);
  assert.deepEqual(after.keys, before.keys);
});

test('descriptor attributes and values stay native', () => {
  const result = observe('Object.getOwnPropertyDescriptor(process.env, "CACHE_DESCRIPTOR_INPUT")');
  assert.deepEqual(result.value, { value: 'before', writable: true, enumerable: true, configurable: true });
  assert.deepEqual(result.keys, ['env:CACHE_DESCRIPTOR_INPUT']);
});

test('a missing descriptor and an empty value remain distinct inputs', () => {
  const expression = 'Object.getOwnPropertyDescriptor(process.env, "CACHE_DESCRIPTOR_INPUT") ?? null';
  const absent = observe(expression, { present: false });
  const empty = observe(expression, { value: '' });
  assert.equal(absent.value, null);
  assert.equal(empty.value.value, '');
  assert.deepEqual(absent.keys, ['env:CACHE_DESCRIPTOR_INPUT']);
  assert.deepEqual(empty.keys, absent.keys);
});

test('enumeration preserves names and conservatively retains descriptor values', () => {
  const result = observe('Object.keys(process.env).includes("CACHE_DESCRIPTOR_INPUT")');
  assert.equal(result.value, true);
  assert.deepEqual(result.keys, ['env:CACHE_DESCRIPTOR_INPUT']);
});

test('disabled observation does not record descriptor reads', () => {
  const result = observe('Object.getOwnPropertyDescriptor(process.env, "CACHE_DESCRIPTOR_INPUT").value', { enabled: false });
  assert.equal(result.value, 'before');
  assert.deepEqual(result.keys, []);
});
