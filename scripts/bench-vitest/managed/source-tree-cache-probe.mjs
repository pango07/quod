// Remote-only paired compiler investigation. No result/receipt reuse.
import fs from 'node:fs'
import path from 'node:path'
import assert from 'node:assert/strict'
import {createManagedCompiler, canonicalManagedJSON} from './compiler.mjs'
import {prepareManagedMatrix} from './preparation.mjs'

const [projectArg, outArg] = process.argv.slice(2)
assert(process.env.GITHUB_ACTIONS === 'true', 'source cache investigation runs on GitHub only')
const project = fs.realpathSync(projectArg), out = path.resolve(outArg)
assert(!fs.existsSync(out)); fs.mkdirSync(out, {recursive: true})
const base = {epoch: 1700000000000, seed: 42, env: {NODE_ENV: 'test'}}
const suite = await prepareManagedMatrix({project, seed: base.seed})
const rootFiles = new Set(['standardSchema.test.ts', 'server/stream.property.test.ts',
  'server/stream.test.ts', 'server/zod4.functions.test.ts', 'server/stream.distinct.test.ts'])
const chosen = suite.prepared.filter(row => row.project === 'convex-helpers'
  ? rootFiles.has(row.file.replace('packages/convex-helpers/', ''))
  : /^zod-4\.[0-6]$/.test(row.project) && row.file === 'packages/convex-helpers/standardSchema.test.ts')
assert.equal(chosen.length, 12, 'exact twelve actual project/file compiler instances required')
const compilers = {on: createManagedCompiler({project, sourceTreeCache: true}),
  off: createManagedCompiler({project, sourceTreeCache: false})}
const result = {schema: 'quod-source-tree-cache-paired-probe-v1', passed: false,
  receiptAdmission: false, sourceReadsCached: false, resolutionsCached: false, finalBundleASTCached: false,
  order: 'alternate off/on and on/off pairs; same source/config and fresh esbuild builds',
  timingLimit: 'same-runner paired diagnostic; no general acceleration or full-job claim', pairs: [], checks: []}
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n')
const save = () => write(path.join(out, 'result.json'), result)
const check = (name, passed) => { result.checks.push({name, passed: Boolean(passed)}); save(); assert(passed, name) }
const options = candidate => ({...base, vitest: candidate.config, file: '/project/' + candidate.file,
  providedContext: candidate.providedContext, ...(candidate.zodAlias ? {zodAlias: candidate.zodAlias} : {})})
let previousFinalParses = {on: 0, off: 0}
const fixtureDir = path.join(project, '.quod-source-cache-fixture')
assert(!fs.existsSync(fixtureDir))
try {
  for (const [index, candidate] of chosen.entries()) {
    const pair = {instanceId: candidate.instanceId, project: candidate.project, file: candidate.file,
      order: index % 2 ? ['on', 'off'] : ['off', 'on'], builds: {}}
    result.pairs.push(pair)
    const directory = path.join(out, candidate.instanceId); fs.mkdirSync(directory)
    for (const mode of pair.order) {
      const built = await compilers[mode]('runner.mjs', options(candidate), candidate.absolute)
      fs.writeFileSync(path.join(directory, mode + '.mjs'), built.bundle)
      write(path.join(directory, mode + '.json'), {...built, bundle: undefined, options: options(candidate)})
      pair.builds[mode] = {seconds: built.seconds, bundleSha256: built.sha256,
        identitySha256: built.identitySha256, sourceTreeCache: built.sourceTreeCache}
      check(`${index}/${mode} final bundle always parses freshly`,
        built.sourceTreeCache.finalBundleParses === previousFinalParses[mode] + 1)
      previousFinalParses[mode] = built.sourceTreeCache.finalBundleParses
      check(`${index}/${mode} cache retains only bounded source ASTs`,
        built.sourceTreeCache.enabled === (mode === 'on') && built.sourceTreeCache.retainedEntries <= 128 &&
        built.sourceTreeCache.retainedSourceBytes <= 8 * 1024 * 1024 &&
        (mode === 'on' || built.sourceTreeCache.hits === 0 && built.sourceTreeCache.retainedEntries === 0))
    }
    check(`${index} cached and uncached compiler output bytes agree`,
      fs.readFileSync(path.join(directory, 'on.mjs')).equals(fs.readFileSync(path.join(directory, 'off.mjs'))))
    check(`${index} complete compile identities agree`, pair.builds.on.identitySha256 === pair.builds.off.identitySha256)
  }
  check('source ASTs are reused across actual builds', result.pairs.at(-1).builds.on.sourceTreeCache.crossBuildHits > 0)
  fs.mkdirSync(fixtureDir)
  const file = path.join(fixtureDir, 'entry.mjs'), candidate = chosen.find(row => row.project === 'convex-helpers')
  const config = {...options(candidate), file: '/project/.quod-source-cache-fixture/entry.mjs'}
  fs.writeFileSync(file, 'console.log(42)\n')
  const fixedTime = 1700000000
  fs.utimesSync(file, fixedTime, fixedTime)
  const initial = fs.statSync(file, {bigint: true}), before = await compilers.on('runner.mjs', config, file)
  fs.writeFileSync(file, 'console.log(43)\n'); fs.utimesSync(file, fixedTime, fixedTime)
  const changed = await compilers.on('runner.mjs', config, file)
  check('same-size same-mtime source bytes change the closed identity',
    fs.statSync(file, {bigint: true}).size === initial.size && fs.statSync(file, {bigint: true}).mtimeNs === initial.mtimeNs && before.identitySha256 !== changed.identitySha256 && before.sha256 !== changed.sha256)
  fs.writeFileSync(file, 'console.log(42)\n'); fs.utimesSync(file, fixedTime, fixedTime)
  const restored = await compilers.on('runner.mjs', config, file)
  check('restored source bytes restore the complete closed identity', before.identitySha256 === restored.identitySha256)
  for (const [name, unsafe] of [
    ['native', 'import fs from "node:fs"; console.log(fs)\n'],
    ['private', 'import {deny} from "quod:compiler-capabilities"; console.log(deny)\n'],
    ['parse', 'const = invalid syntax;\n'],
    ['type-native', 'import type {Stats} from "node:fs"; export const value=1;\n'],
    ['type-private', 'import type {deny} from "quod:compiler-capabilities"; export const value=1;\n'],
  ]) {
    const target = name.startsWith('type-') ? path.join(fixtureDir, 'typed-entry.ts') : file
    const targetConfig = {...config, file: '/project/' + path.relative(project, target).split(path.sep).join('/')}
    const safe = 'export const value=1;'.padEnd(unsafe.length - 1, ' ') + '\n'
    assert.equal(Buffer.byteLength(safe), Buffer.byteLength(unsafe))
    fs.writeFileSync(target, safe); fs.utimesSync(target, fixedTime, fixedTime)
    const stamp = fs.statSync(target, {bigint: true})
    await compilers.on('runner.mjs', targetConfig, target)
    fs.writeFileSync(target, unsafe); fs.utimesSync(target, fixedTime, fixedTime)
    check(`${name} mutation preserves exact size and nanosecond mtime`,
      fs.statSync(target, {bigint: true}).size === stamp.size && fs.statSync(target, {bigint: true}).mtimeNs === stamp.mtimeNs)
    let rejected = false
    try { await compilers.on('runner.mjs', targetConfig, target) } catch { rejected = true }
    check(`same-metadata ${name} mutation still rejects compilation`, rejected)
  }
  result.seconds = Object.fromEntries(['on', 'off'].map(mode => [mode,
    result.pairs.reduce((sum, pair) => sum + pair.builds[mode].seconds, 0)]))
  result.passed = result.checks.every(row => row.passed)
} catch (error) { result.error = {name: error.name, message: error.message, stack: error.stack}; process.exitCode = 1 }
finally { fs.rmSync(fixtureDir, {recursive: true, force: true}); save() }
console.log(canonicalManagedJSON({passed: result.passed, seconds: result.seconds, checks: result.checks.length,
  error: result.error || null}))
if (!result.passed) process.exitCode = 1
