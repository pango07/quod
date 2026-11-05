import { test } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { pathToFileURL } from 'node:url'

const probe = new URL('./native-stack-probe.mjs', import.meta.url).href
function run(body, user = null) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'quod-stack-'))
  try {
    fs.mkdirSync(path.join(root, 'node_modules/vite'), {recursive:true})
    const tool = path.join(root, 'node_modules/vite/reader.mjs')
    fs.writeFileSync(tool, `import {nativeCaller} from ${JSON.stringify(probe)};
      const read = () => nativeCaller(false, ${JSON.stringify(probe)}, [], ['/node_modules/vite/'], [], []);
      ${body}`)
    const entry = user === null ? tool : path.join(root, 'user.mjs')
    if (user !== null) fs.writeFileSync(entry, user.replaceAll('TOOL_URL', JSON.stringify(pathToFileURL(tool).href)))
    const result = spawnSync(process.execPath, [entry], {encoding:'utf8'})
    assert.equal(result.status, 0, result.stderr)
    return JSON.parse(result.stdout)
  } finally { fs.rmSync(root, {recursive:true, force:true}) }
}

test('tool-only frames qualify without invoking the application formatter', () => {
  const result = run(`let called=0; const formatter=()=>{called++;return 'fake'};
    Error.prepareStackTrace=formatter;
    const r=read(); console.log(JSON.stringify({...r,called,same:Error.prepareStackTrace===formatter}));`)
  assert.equal(result.tool, true)
  assert.equal(result.called, 0)
  assert.equal(result.same, true)
})
test('a synchronous user below a tool remains a value-keyed caller', () => {
  const result = run('export function tool(){return read()}', 'import {tool} from TOOL_URL; console.log(JSON.stringify(tool()));')
  assert.equal(result.tool, false)
  assert.match(result.frame, /user\.mjs/)
})
test('an asynchronous user below a resumed tool is retained', () => {
  const result = run('export async function tool(){await Promise.resolve();return read()}',
    'import {tool} from TOOL_URL; async function user(){return await tool()} console.log(JSON.stringify(await user()));')
  assert.equal(result.tool, false)
  assert.match(result.frame, /user\.mjs/)
})
test('Promise.all preserves its user causal parent', () => {
  const result = run('export async function tool(){await Promise.resolve();return read()}',
    'import {tool} from TOOL_URL; async function user(){return await Promise.all([tool()])} console.log(JSON.stringify((await user())[0]));')
  assert.equal(result.tool, false)
  assert.match(result.frame, /user\.mjs/)
})
test('a user beyond two hundred frames is retained', () => {
  const result = run('export function tool(n){return n?tool(n-1):read()}',
    'import {tool} from TOOL_URL; console.log(JSON.stringify(tool(230)));')
  assert.equal(result.tool, false)
  assert.match(result.frame, /user\.mjs/)
})
test('an evaluated tool-shaped source URL is not trusted', () => {
  const result = run(`const r=eval('read()\\n//# sourceURL=/node_modules/vite/spoof.mjs'); console.log(JSON.stringify(r));`)
  assert.equal(result.tool, false)
  assert.match(result.frame, /eval/)
})
test('a stateful custom formatter cannot hide a user', () => {
  const result = run('export function tool(){return read()}',
    `import {tool} from TOOL_URL; Error.prepareStackTrace=()=> 'Error\\n at /node_modules/vite/fake.js:1:1'; console.log(JSON.stringify(tool()));`)
  assert.equal(result.tool, false)
  assert.match(result.frame, /user\.mjs/)
})
