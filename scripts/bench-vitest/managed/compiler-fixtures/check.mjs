// GitHub gate only. Compiles admission fixtures and executes tiny closed-glob
// controls in fresh workerd processes. It never installs or runs the heavy test.
import fs from 'node:fs'
import path from 'node:path'
import assert from 'node:assert/strict'
import {fileURLToPath} from 'node:url'
import {createManagedCompiler, canonicalManagedJSON} from '../compiler.mjs'
import {prepareManagedProject} from '../preparation.mjs'
import {createManagedExecutor} from '../executor.mjs'
import {MANAGED_BOUNDARY_CONTRACT, MANAGED_INTRINSIC_POLICY} from '../receipt.mjs'

const [projectArg, outArg] = process.argv.slice(2)
assert(projectArg && outArg, 'usage: check.mjs PROJECT EMPTY_OUTPUT_DIRECTORY')
const project = path.resolve(projectArg), out = path.resolve(outArg)
assert(!fs.existsSync(out)); fs.mkdirSync(out, {recursive: true})
const dir = path.join(project, '.quod-managed-compiler-fixtures')
assert(!fs.existsSync(dir)); fs.mkdirSync(dir)
const here = path.dirname(fileURLToPath(import.meta.url))
for (const name of ['literal-import.test.mjs', 'literal-value.mjs', 'computed-import.test.mjs', 'evaluation.test.mjs', 'metadata.test.mjs', 'glob.test.mjs']) {
  fs.copyFileSync(path.join(here, name), path.join(dir, name))
}
const compiler = createManagedCompiler({project})
const config = {epoch: 1700000000000, seed: 42, env: {NODE_ENV: 'test'}}
const checks = [], bundles = []
const check = (name, passed) => { checks.push({name, passed: Boolean(passed)}); assert(passed, name) }
const save = (name, built) => {
  fs.writeFileSync(path.join(out, name + '.mjs'), built.bundle)
  fs.writeFileSync(path.join(out, name + '.json'), JSON.stringify({...built, bundle: undefined}, null, 2) + '\n')
  bundles.push({name, sha256: built.sha256, identitySha256: built.identitySha256})
}
try {
  for (const [name, reason] of [['literal-import', null], ['computed-import', 'computed module import'],
    ['evaluation', 'dynamic code evaluation'], ['metadata', 'ambient module metadata']]) {
    const built = await compiler('runner.mjs', config, path.join(dir, name + '.test.mjs'))
    save(name, built)
    check(name + ' final AST is closed', built.closure.finalBundleParsed && built.closure.remainingDynamicImports === 0)
    check(name + ' all native imports use ALS only', built.closure.externalImports.every(value => value === 'node:async_hooks'))
    if (reason) check(name + ' injects private sticky rejection', built.identity.transforms.some(row => row.policy === reason))
  }
  const literal = await compiler('runner.mjs', config, path.join(dir, 'literal-import.test.mjs'))
  const repeated = await compiler('runner.mjs', config, path.join(dir, 'literal-import.test.mjs'))
  check('unchanged preparation identity repeats', literal.identitySha256 === repeated.identitySha256)
  check('literal dependency is recorded', literal.identity.sources.some(row => row.source.endsWith('/literal-value.mjs')))
  const changed = await compiler('runner.mjs', {...config, seed: 43}, path.join(dir, 'literal-import.test.mjs'))
  check('declared configuration changes identity', literal.identitySha256 !== changed.identitySha256)
  fs.writeFileSync(path.join(dir, 'literal-value.mjs'), 'export const value = 43\n')
  const changedDependency = await compiler('runner.mjs', config, path.join(dir, 'literal-import.test.mjs'))
  check('literal dependency bytes change identity', literal.identitySha256 !== changedDependency.identitySha256)
  fs.copyFileSync(path.join(here, 'literal-value.mjs'), path.join(dir, 'literal-value.mjs'))
  fs.writeFileSync(path.join(dir, 'unrelated.txt'), 'not a module or glob match\n')
  const unrelated = await compiler('runner.mjs', config, path.join(dir, 'literal-import.test.mjs'))
  check('unrelated file leaves closed identity unchanged', literal.identitySha256 === unrelated.identitySha256)
  const globDir = path.join(dir, 'glob-modules'); fs.mkdirSync(globDir)
  fs.writeFileSync(path.join(globDir, 'one.mjs'), 'export const value = 1\n')
  const globBefore = await compiler('runner.mjs', config, path.join(dir, 'glob.test.mjs'))
  save('glob', globBefore)
  check('glob names are recorded', globBefore.globs.some(row => row.names.includes('./glob-modules/one.mjs')))
  check('glob records resolved module targets', globBefore.globs.some(row =>
    row.matches.some(match => match.target.endsWith('/glob-modules/one.mjs'))))
  fs.renameSync(path.join(globDir, 'one.mjs'), path.join(globDir, 'renamed.mjs'))
  const globAfter = await compiler('runner.mjs', config, path.join(dir, 'glob.test.mjs'))
  check('glob module-name mutation changes identity', globBefore.identitySha256 !== globAfter.identitySha256)
  for (const [name, source] of [
    ['private-namespace', 'import {runtime} from "quod:capabilities"; console.log(runtime)'],
    ['private-generated-namespace', 'import {deny} from "quod:compiler-capabilities"; console.log(deny)'],
    ['private-adapter', `import {runtime} from ${JSON.stringify(path.resolve(here, '../prelude.mjs'))}; console.log(runtime)`],
    ['unused-private-adapter', `import {runtime} from ${JSON.stringify(path.resolve(here, '../prelude.mjs'))}; console.log(42)`],
    ['native-fs', 'import fs from "node:fs"; console.log(fs)'],
    ['native-alias', 'import fs from "fs"; console.log(fs)'],
    ['native-prefix-only', 'import test from "node:test"; console.log(test)'],
    ['als-namespace', 'import * as hooks from "node:async_hooks"; console.log(hooks)'],
    ['als-default', 'import hooks from "node:async_hooks"; console.log(hooks)'],
    ['als-extra', 'import {AsyncResource} from "node:async_hooks"; console.log(AsyncResource)'],
    ['als-reexport', 'export {AsyncLocalStorage} from "node:async_hooks"'],
    ['als-alias-namespace', 'import * as hooks from "async_hooks"; console.log(hooks)'],
    ['als-alias-extra', 'import {AsyncResource} from "async_hooks"; console.log(AsyncResource)'],
    ['als-dynamic', 'await import("node:async_hooks")'],
    ['native-require', 'const fs = require("fs"); console.log(fs)'],
  ]) {
    const file = path.join(dir, name + '.test.mjs'); fs.writeFileSync(file, source + '\n')
    let rejected = false
    try { await compiler('runner.mjs', config, file) } catch { rejected = true }
    check(name + ' rejects preparation', rejected)
  }
  for (const [name, source] of [
    ['type-private-namespace', 'import type {runtime} from "quod:capabilities"; export const value = 42'],
    ['type-private-adapter', `import type {runtime} from ${JSON.stringify(path.resolve(here, '../prelude.mjs'))}; export const value = 42`],
  ]) {
    const file = path.join(dir, name + '.test.ts'); fs.writeFileSync(file, source + '\n')
    let rejected = false
    try { await compiler('runner.mjs', config, file) } catch { rejected = true }
    check(name + ' rejects original source before erasure', rejected)
  }
  const adapterAlias = path.join(dir, 'adapter-alias.mjs')
  fs.symlinkSync(path.resolve(here, '../prelude.mjs'), adapterAlias)
  const adapterAliasTest = path.join(dir, 'adapter-alias.test.mjs')
  fs.writeFileSync(adapterAliasTest, 'import {runtime} from "./adapter-alias.mjs"; console.log(runtime)\n')
  let aliasRejected = false
  try { await compiler('runner.mjs', config, adapterAliasTest) } catch { aliasRejected = true }
  check('resolved symlink to private adapter rejects preparation', aliasRejected)
  const alsFile = path.join(dir, 'als-named.test.mjs')
  fs.writeFileSync(alsFile, 'import {AsyncLocalStorage as ALS} from "node:async_hooks"; console.log(new ALS().getStore())\n')
  const als = await compiler('runner.mjs', config, alsFile); save('als-named', als)
  check('named ALS uses the managed facade', Object.keys(als.metadata.inputs).some(name => name.includes('quod-managed-als:')))
  fs.writeFileSync(alsFile, 'import {AsyncLocalStorage as ALS} from "async_hooks"; console.log(new ALS().getStore())\n')
  const bareALS = await compiler('runner.mjs', config, alsFile); save('als-named-bare', bareALS)
  check('bare native ALS alias uses the managed facade', Object.keys(bareALS.metadata.inputs).some(name => name.includes('quod-managed-als:')))
  check('configuration key order is canonical', canonicalManagedJSON({b: 2, a: 1}) === canonicalManagedJSON({a: 1, b: 2}))
  // Keep the original40 controls above unchanged. The following submatrix
  // executes small fixtures remotely using the actual reviewed root config.
  fs.writeFileSync(path.join(globDir, 'added.mjs'), 'export const value = 2\n')
  const globAdded = await compiler('runner.mjs', config, path.join(dir, 'glob.test.mjs'))
  check('adding a denied foreign glob name changes closed identity', globAfter.identitySha256 !== globAdded.identitySha256)
  fs.unlinkSync(path.join(globDir, 'added.mjs'))
  const globRemoved = await compiler('runner.mjs', config, path.join(dir, 'glob.test.mjs'))
  check('removing that glob name restores the exact prior closed identity', globAfter.identitySha256 === globRemoved.identitySha256)
  process.chdir(project)
  const prepared = await prepareManagedProject({project,
    file: 'packages/convex-helpers/server/stream.property.test.ts', seed: config.seed})
  fs.writeFileSync(path.join(out, 'self-glob-preparation.json'), JSON.stringify(prepared, null, 2) + '\n')
  const runtimeDir = path.join(dir, 'closed-glob-runtime'); fs.mkdirSync(runtimeDir)
  for (const name of ['self-glob.test.mjs', 'foreign-glob.test.mjs', 'name-collision-one.mjs', 'name-collision-two.mjs']) {
    fs.copyFileSync(path.join(here, name), path.join(runtimeDir, name))
  }
  const foreignDir = path.join(runtimeDir, 'foreign-modules'); fs.mkdirSync(foreignDir)
  fs.writeFileSync(path.join(foreignDir, 'value.mjs'), 'globalThis.__quodClosedForeignGlobEvaluations = 1; export const value = 42\n')
  fs.writeFileSync(path.join(foreignDir, 'native.mjs'), 'import fs from "node:fs"; globalThis.__quodClosedForeignGlobEvaluations = 1; export default fs\n')
  const executor = createManagedExecutor({out, watchdogMs: 30000})
  for (const [name, count, unsupported] of [
    ['self-glob', 3, []],
    ['foreign-glob', 1, ['lazy module ./foreign-modules/native.mjs', 'lazy module ./foreign-modules/value.mjs']],
  ]) {
    const file = path.join(runtimeDir, name + '.test.mjs')
    const options = {...config, vitest: prepared.config,
      file: '/project/' + path.relative(project, file).split(path.sep).join('/'), providedContext: {}}
    const built = await compiler('runner.mjs', options, file)
    save(name, {...built, options})
    if (name === 'self-glob') check('original class and function names are an explicit compile policy', built.identity.policy.keepNames === true)
    check(name + ' final module graph is closed', built.closure.finalBundleParsed && built.closure.remainingDynamicImports === 0 &&
      built.closure.remainingDynamicRequires === 0 && built.closure.externalImports.every(value => value === 'node:async_hooks'))
    const admitted = built.globs.flatMap(row => row.admittedTargets)
    check(name + ' admits only its exact actual entry', admitted.every(row => row.target === built.identity.testEntry) && admitted.length === (name === 'self-glob' ? 1 : 0))
    const data = await executor.execute('runtime-' + name, built.bundle)
    check(name + ' actual original assertions pass', data.ok === true && data.rows.length === count && data.rows.every(row => row.state === 'pass'))
    check(name + ' private completion remains sealed', data.boundary.contract === MANAGED_BOUNDARY_CONTRACT &&
      data.boundary.intrinsicPolicy === MANAGED_INTRINSIC_POLICY && data.boundary.completion.sealed === true &&
      data.boundary.completion.intrinsicLockdown === true && data.boundary.completion.reason === '' && data.boundary.pendingTimers === 0 &&
      ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'].every(key => data.boundary.completion[key] === 0) &&
      ['timerErrors', 'unhandledErrors', 'unguardable', 'symbolUnguardable'].every(key => data.boundary[key].length === 0))
    check(name + ' denial inventory is exact after caught calls', canonicalManagedJSON(data.boundary.unsupported.slice().sort()) === canonicalManagedJSON(unsupported))
  }
  fs.symlinkSync(path.resolve(here, '../prelude.mjs'), path.join(foreignDir, 'private.mjs'))
  let privateGlobRejected = false
  try { await compiler('runner.mjs', config, path.join(runtimeDir, 'foreign-glob.test.mjs')) } catch { privateGlobRejected = true }
  check('glob symlink cannot expose a private adapter even as a denied foreign key', privateGlobRejected)
  fs.unlinkSync(path.join(foreignDir, 'private.mjs'))
  let nativeEntryRejected = false
  try { await compiler('runner.mjs', config, path.join(foreignDir, 'native.mjs')) } catch { nativeEntryRejected = true }
  check('glob target native imports remain denied when selected as the actual entry', nativeEntryRejected)
  const result = {schema: 'quod-managed-compiler-regressions-v1', originalCheckCount: 40,
    selfGlobCheckCount: 15, checks, bundles, cacheEnabled: false}
  fs.writeFileSync(path.join(out, 'result.json'), JSON.stringify(result, null, 2) + '\n')
  console.log(JSON.stringify(result))
} finally {
  fs.rmSync(dir, {recursive: true, force: true})
}
