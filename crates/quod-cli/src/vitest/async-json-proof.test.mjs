import {test} from 'node:test'
import assert from 'node:assert/strict'
import {certifyAsyncJsonReads} from './async-json-proof.mjs'

const fixture = `
const readFileAsync = fs.promises.readFile;
const jsonLoader = (_, content) => JSON.parse(content);
async function search(dir, searchPlaces, loaders) {
  const result = {};
  for (const searchPlace of searchPlaces) {
    const filepath = join(dir, searchPlace);
    const content = String(await readFileAsync(filepath));
    const loaderKey = extname(searchPlace) || "noExt";
    const loader = loaders[loaderKey];
    if (searchPlace === "package.json") {
      const pkg = await loader(filepath, content);
      const config = getPackageProp("postcss", pkg);
      if (config != null) { result.config = config; break; }
      continue;
    }
    const isEmpty = content.trim() === "";
    if (!isEmpty) result.config = await loader(filepath, content);
    break;
  }
  return result;
}`;

test('identifies the direct async decode, private branch and pure JSON parser', () => {
  const proof = certifyAsyncJsonReads(fixture)
  assert.equal(proof.reads.length,1)
  assert.equal(proof.parsers.length,1)
  const read=proof.reads[0]
  assert.equal(fixture.slice(read.read.start,read.read.end),'readFileAsync(filepath)')
  assert.equal(fixture.slice(read.convert.start,read.convert.end),'String(await readFileAsync(filepath))')
  assert.equal(fixture.slice(read.loaderCall.start,read.loaderCall.end),'loader(filepath, content)')
  assert.equal(fixture.slice(proof.parsers[0].call.start,proof.parsers[0].call.end),'JSON.parse(content)')
})

test('raw aliases, escaping closures, eval and reads after the candidate parse reject the site', () => {
  for (const change of [
    s=>s.replace('const loaderKey', 'globalThis.raw = content; const loaderKey'),
    s=>s.replace('const loaderKey', 'const alias = content; const loaderKey'),
    s=>s.replace('const loaderKey', 'const leak = () => content; const loaderKey'),
    s=>s.replace('const loaderKey', 'eval("globalThis.raw = content"); const loaderKey'),
    s=>s.replace('const config', 'globalThis.raw = content; const config'),
    s=>s.replace('continue;', 'globalThis.later = () => content; continue;'),
    s=>s.replace('const isEmpty', 'globalThis.raw = content; const isEmpty'),
    s=>s.replace('const isEmpty', 'const deferred = () => loader(filepath, content); const isEmpty'),
    s=>s.replace('continue;', ''),
    s=>s.replace('continue;', 'if (flag) continue;'),
    s=>s.replace('continue;', 'continue outer;'),
  ]) assert.equal(certifyAsyncJsonReads(change(fixture)).reads.length,0,change(fixture))
})

test('unknown coercions, mutable raw bindings, extra arguments and global identifier getters reject the site', () => {
  for (const change of [
    s=>s.replace('const content =', 'let content ='),
    s=>s.replace('String(await readFileAsync(filepath))','decode(await readFileAsync(filepath))'),
    s=>s.replace('String(await readFileAsync(filepath))','String(await readFileAsync(filepath)).trim()'),
    s=>s.replace('String(await readFileAsync(filepath))','String(await readFileAsync(filepath), extra)'),
    s=>s.replace('readFileAsync(filepath)','readFileAsync(filepath, "utf8")'),
    s=>s.replace('readFileAsync(filepath)','readFileAsync(getPath())'),
    s=>s.replace('readFileAsync(filepath)','fs.promises.readFile(filepath)'),
    s=>s.replace('const readFileAsync = fs.promises.readFile;', ''),
    s=>s.replace('const filepath = join(dir, searchPlace);', ''),
    s=>s.replace('const jsonLoader', 'const String = custom; const jsonLoader'),
    s=>s.replace('const jsonLoader', 'const { String } = custom; const jsonLoader'),
    s=>s.replace('async function search(dir,', 'async function search(String, dir,'),
    s=>s.replace('async function search', 'function search'),
    s=>s.replace('const pkg = await loader(filepath, content)', 'const pkg = await loader(filepath, content, extra)'),
    s=>s.replace('const pkg = await loader(filepath, content)', 'const pkg = await loader(other, content)'),
  ]) assert.equal(certifyAsyncJsonReads(change(fixture)).reads.length,0,change(fixture))
})

test('a custom loader does not become a pure parser merely because it eventually calls JSON.parse', () => {
  for (const loader of [
    '(_, content) => { globalThis.raw = content; return JSON.parse(content); }',
    '(_, content) => JSON.parse(content, reviver)',
    '(_, content) => JSON.parse(content.trim())',
    'async (_, content) => JSON.parse(content)',
    '(_, content) => JSON["parse"](content)',
    '(_, content) => (effect(content), JSON.parse(content))',
    '(_, content) => { const copy = content; return JSON.parse(copy); }',
    '(_, content) => JSON.parse(content.replace(/^\\uFEFF/, ""))',
  ]) assert.equal(certifyAsyncJsonReads(`const jsonLoader = ${loader}`).parsers.length,0,loader)
})

test('parser limits and unknown syntax fail closed', () => {
  for (const source of [null, {}, 'const = broken', ' '.repeat(2_000_001)]) assert.deepEqual(certifyAsyncJsonReads(source),{reads:[],parsers:[]})
})
