// Remote-only diagnostic; never admits a receipt or skips a real command.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {fileURLToPath} from 'node:url'
import {spawnSync} from 'node:child_process'
import {createManagedCompiler, canonicalManagedJSON} from './compiler.mjs'
import {prepareManagedMatrix} from './preparation.mjs'
import {captureManagedCompileCertificate, validateManagedCompileCertificates} from './compile-input-certificate.mjs'

assert(process.env.GITHUB_ACTIONS === 'true', 'GitHub-only certificate investigation')
// Only the remote tsconfig diagnostic uses this branch. It creates a new plain
// Node process and therefore does not share the parent's loaded esbuild API.
if (process.argv[2] === '--compile-child') {
  const request = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'))
  assert(!fs.existsSync(request.out)); fs.mkdirSync(request.out)
  const saveChild = (name, bytes) => {
    assert(bytes.length <= 16 * 1024 * 1024, 'bounded compile child output required')
    fs.writeFileSync(path.join(request.out, name), bytes, {flag: 'wx'})
  }
  try {
    const compile = createManagedCompiler({project: request.project, sourceTreeCache: false})
    const built = await compile('runner.mjs', request.options, request.file)
    saveChild('fresh.mjs', Buffer.from(built.bundle))
    saveChild('fresh-build.json', Buffer.from(JSON.stringify({...built, bundle: undefined, options: request.options,
      node: process.version, execArgv: process.execArgv}, null, 2) + '\n'))
    process.exit(0)
  } catch (error) {
    saveChild('error.json', Buffer.from(JSON.stringify({name: error.name, message: error.message, stack: error.stack}, null, 2) + '\n'))
    process.exit(1)
  }
}
const [projectArg, outArg] = process.argv.slice(2), project = fs.realpathSync(projectArg), out = path.resolve(outArg)
assert(!fs.existsSync(out)); fs.mkdirSync(out, {recursive: true})
const fixture = path.join(project, '.quod-compile-input-fixture')
assert(!fs.existsSync(fixture))
const result = {schema: 'quod-managed-compile-input-certificate-probe-v1', passed: false,
  productionEnabled: false, receiptAdmission: false, nativePreparationAlwaysRuns: true,
  timingLimit: 'same-runner diagnostic only; no full-job or whole-window acceleration claim',
  atomicity: 'sampled before/after reads; active or adversarial workspace atomicity is not claimed',
  pairs: [], checks: [], mutations: []}
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const write = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n')
const save = () => write(path.join(out, 'result.json'), result)
const check = (name, pass) => { result.checks.push({name, pass: Boolean(pass)}); save(); assert(pass, name) }
const base = {epoch: 1700000000000, seed: 42, env: {NODE_ENV: 'test'}}
const options = candidate => ({...base, vitest: candidate.config, file: '/project/' + candidate.file,
  providedContext: candidate.providedContext, ...(candidate.zodAlias ? {zodAlias: candidate.zodAlias} : {})})
const compiler = createManagedCompiler({project})
const fresh = createManagedCompiler({project, sourceTreeCache: false})
const build = async (file, config, target = compiler) => target('runner.mjs', config, file)
const request = (seed, config, file) => ({certificateBytes: seed.bytes, trustedCertificateSha256: seed.sha256,
  options: config, testFile: file})
const capture = (built, config, file) => captureManagedCompileCertificate({project, built, options: config, testFile: file})
const validate = requests => validateManagedCompileCertificates({project, requests})
function snapshotFixture(directory) {
  fs.mkdirSync(directory, {recursive: true})
  const rows = []
  let bytes = 0
  function walk(file, relative = '') {
    assert(rows.length < 64, 'tiny fixture snapshot path bound')
    const stat = fs.lstatSync(file), row = {name: relative || '.', mode: stat.mode & 0o7777}
    if (stat.isDirectory()) {
      row.kind = 'directory'; rows.push(row)
      for (const name of fs.readdirSync(file).sort()) walk(path.join(file, name), relative ? relative + '/' + name : name)
    } else if (stat.isSymbolicLink()) {
      row.kind = 'symlink'; row.target = fs.readlinkSync(file)
      try {row.realTarget = fs.realpathSync(file)} catch (error) {row.realTargetError = error.code}
      rows.push(row)
    } else {
      assert(stat.isFile(), 'tiny fixture snapshot regular data required')
      const data = fs.readFileSync(file); bytes += data.length
      assert(bytes <= 1024 * 1024, 'tiny fixture snapshot byte bound')
      row.kind = 'file'; row.bytes = data.length; row.sha256 = sha(data); rows.push(row)
      const target = path.join(directory, 'files', relative)
      fs.mkdirSync(path.dirname(target), {recursive: true}); fs.writeFileSync(target, data, {flag: 'wx'})
    }
  }
  walk(fixture)
  const inventory = {schema: 'quod-compile-input-mutation-snapshot-v1', rows, bytes,
    symlinks: 'target text only; no links are copied into evidence'}
  write(path.join(directory, 'inventory.json'), inventory)
  return inventory
}
function separateCompile(directory, config, file) {
  const childOut = path.join(directory, 'separate-node'), requestFile = path.join(directory, 'child-request.json')
  write(requestFile, {project, options: config, file, out: childOut})
  const argv = [fileURLToPath(import.meta.url), '--compile-child', requestFile]
  const child = spawnSync(process.execPath, argv, {cwd: project, env: process.env,
    timeout: 30000, killSignal: 'SIGKILL', maxBuffer: 1024 * 1024})
  const stdout = child.stdout || Buffer.alloc(0), stderr = child.stderr || Buffer.alloc(0)
  fs.writeFileSync(path.join(directory, 'child-stdout.log'), stdout.subarray(0, 1024 * 1024))
  fs.writeFileSync(path.join(directory, 'child-stderr.log'), stderr.subarray(0, 1024 * 1024))
  const evidence = {node: process.execPath, argv, execArgv: [], timeoutMs: 30000, maxBytesPerLog: 1024 * 1024,
    status: child.status, signal: child.signal, stdoutBytes: stdout.length, stderrBytes: stderr.length,
    error: child.error ? {name: child.error.name, message: child.error.message, code: child.error.code || null} : null}
  write(path.join(directory, 'child-process.json'), evidence)
  if (child.status === 0 && fs.existsSync(path.join(childOut, 'fresh-build.json'))) {
    const built = JSON.parse(fs.readFileSync(path.join(childOut, 'fresh-build.json'), 'utf8'))
    return {...evidence, identitySha256: built.identitySha256, bundleSha256: built.sha256}
  }
  return evidence
}
let fixtureCreated = false
try {
  const suite = await prepareManagedMatrix({project, seed: base.seed})
  const rootFiles = new Set(['standardSchema.test.ts', 'server/stream.property.test.ts', 'server/stream.test.ts',
    'server/zod4.functions.test.ts', 'server/stream.distinct.test.ts'])
  const chosen = suite.prepared.filter(row => row.project === 'convex-helpers'
    ? rootFiles.has(row.file.replace('packages/convex-helpers/', ''))
    : /^zod-4\.[0-6]$/.test(row.project) && row.file === 'packages/convex-helpers/standardSchema.test.ts')
  assert.equal(chosen.length, 12)
  const requests = []
  for (const candidate of chosen) {
    const config = options(candidate), built = await build(candidate.absolute, config)
    const seed = capture(built, config, candidate.absolute)
    const dir = path.join(out, candidate.instanceId); fs.mkdirSync(dir)
    fs.writeFileSync(path.join(dir, 'certificate.json'), seed.bytes)
    fs.writeFileSync(path.join(dir, 'seed.mjs'), built.bundle)
    write(path.join(dir, 'seed-build.json'), {...built, bundle: undefined, options: config})
    result.pairs.push({instanceId: candidate.instanceId, project: candidate.project, file: candidate.file,
      certificateSha256: seed.sha256, certificateBytes: seed.bytes.length, seedCompileSeconds: built.seconds})
    requests.push(request(seed, config, candidate.absolute))
  }
  const start = performance.now(), validated = await validate(requests)
  result.sharedValidationSeconds = (performance.now() - start) / 1000
  for (const [index, candidate] of chosen.entries()) {
    const actual = await build(candidate.absolute, options(candidate), fresh), candidateIdentity = validated[index]
    const dir = path.join(out, candidate.instanceId)
    fs.writeFileSync(path.join(dir, 'fresh.mjs'), actual.bundle)
    write(path.join(dir, 'fresh-build.json'), {...actual, bundle: undefined, options: options(candidate)})
    write(path.join(dir, 'validated.json'), candidateIdentity)
    result.pairs[index].freshCompileSeconds = actual.seconds
    check(index + ' current certificate validates after fresh native preparation', candidateIdentity.hit === true)
    check(index + ' full fresh compile identity equals validated candidate',
      candidateIdentity.identitySha256 === actual.identitySha256 && canonicalManagedJSON(candidateIdentity.identity) === canonicalManagedJSON(actual.identity))
    check(index + ' actual fresh bundle bytes equal original compiled bytes',
      candidateIdentity.bundleSha256 === actual.sha256 && fs.readFileSync(path.join(dir, 'seed.mjs')).equals(Buffer.from(actual.bundle)))
  }

  fs.mkdirSync(fixture); fixtureCreated = true
  fs.mkdirSync(path.join(fixture, 'modules'))
  const packageDir = path.join(fixture, 'node_modules/quod-input-package'); fs.mkdirSync(packageDir, {recursive: true})
  const files = {
    'entry.ts': 'import type {T} from "./types.js"; import {value} from "./dep"; import {alias} from "./alias.mjs"; import {pkg} from "quod-input-package"; console.log(value,alias,pkg,Object.keys(import.meta.glob("./modules/*.mjs"))); class Base { set field(value) { console.log(value) } } class Named extends Base { field=1 } console.log(Object.hasOwn(new Named(),"field")); // A\n',
    'types.ts': 'export type T = number;\n', 'dep.js': 'export const value=42;\n',
    'one.mjs': 'export const alias=1;\n', 'two.mjs': 'export const alias=2;\n',
    'modules/alpha.mjs': 'export default 1;\n',
    'tsconfig.json': '{"compilerOptions":{"useDefineForClassFields":true}}\n',
    'node_modules/quod-input-package/package.json': '{"name":"quod-input-package","type":"module","exports":"./one.mjs"}\n',
    'node_modules/quod-input-package/one.mjs': 'export const pkg=1;\n',
    'node_modules/quod-input-package/two.mjs': 'export const pkg=2;\n',
  }
  const fixed = 1700000000
  const put = (name, bytes) => { const file = path.join(fixture, name); fs.writeFileSync(file, bytes); fs.utimesSync(file, fixed, fixed) }
  for (const [name, bytes] of Object.entries(files)) put(name, bytes)
  fs.symlinkSync('one.mjs', path.join(fixture, 'alias.mjs'))
  const rootCandidate = chosen.find(row => row.project === 'convex-helpers'), entry = path.join(fixture, 'entry.ts')
  const config = {...options(rootCandidate), file: '/project/.quod-compile-input-fixture/entry.ts'}
  const initial = await build(entry, config), seed = capture(initial, config, entry)
  fs.writeFileSync(path.join(out, 'fixture-certificate.json'), seed.bytes)
  fs.writeFileSync(path.join(out, 'fixture-seed.mjs'), initial.bundle)
  write(path.join(out, 'fixture-seed-build.json'), {...initial, bundle: undefined, options: config})
  snapshotFixture(path.join(out, 'fixture-baseline'))
  check('erased type-only import has an original resolution fact', initial.identity.resolutionFacts.some(fact =>
    fact.phase === 'original' && fact.specifier === './types.js' && fact.target.endsWith('/types.ts')) &&
    !initial.identity.resolutionFacts.some(fact => fact.phase === 'emitted' && fact.specifier === './types.js'))
  check('unchanged small fixture validates', (await validate([request(seed, config, entry)]))[0].hit)
  async function mutation(name, change, restore, {reject = false, config: currentConfig = config, identityChanges = true} = {}) {
    const directory = path.join(out, 'mutations', String(result.mutations.length + 1).padStart(2, '0') + '-' + name.replace(/[^A-Za-z0-9]+/g, '-'))
    fs.mkdirSync(directory, {recursive: true})
    const row = {name, observation: null, decision: null, freshIdentitySha256: null, freshBundleSha256: null, rejected: null}
    result.mutations.push(row)
    try {
      row.observation = await change() || null
      snapshotFixture(path.join(directory, 'fixture'))
      const decision = (await validate([request(seed, currentConfig, entry)]))[0]
      row.decision = decision; write(path.join(directory, 'decision.json'), decision); save()
      let rebuilt, rejection, freshError
      try { rebuilt = await build(entry, currentConfig, fresh) }
      catch (error) { rejection = error.message; freshError = {name: error.name, message: error.message, stack: error.stack} }
      row.freshIdentitySha256 = rebuilt?.identitySha256 || null; row.freshBundleSha256 = rebuilt?.sha256 || null; row.rejected = rejection || null
      if (rebuilt) {
        fs.writeFileSync(path.join(directory, 'fresh.mjs'), rebuilt.bundle)
        write(path.join(directory, 'fresh-build.json'), {...rebuilt, bundle: undefined, options: currentConfig})
      } else write(path.join(directory, 'fresh-error.json'), freshError || {message: rejection})
      if (name === 'tsconfig bytes changed') row.separateNode = separateCompile(directory, currentConfig, entry)
      write(path.join(directory, 'mutation.json'), row); save()
      // Preserve all actual mutant inputs/results, including the separate Node
      // diagnostic, before either assertion can abort the investigation.
      if (row.observation?.sameMetadataRequired) check(name + ' exact size and nanosecond mtime stay equal',
        row.observation.sizeUnchanged && row.observation.mtimeUnchanged)
      check(name + ' certificate miss requires full compilation', !decision.hit && decision.next === 'full-compile')
      check(name + ' actual fresh compiler confirms expected changed or rejected input', reject ? Boolean(rejection)
        : !rejection && (rebuilt.identitySha256 !== initial.identitySha256) === identityChanges)
    } finally { await restore() }
    check(name + ' restored inputs recover prior certificate', (await validate([request(seed, config, entry)]))[0].hit)
  }
  for (const [name, file, changed] of [
    ['same-metadata dependency bytes', 'dep.js', files['dep.js'].replace('42', '43')],
    ['same-metadata source comment bytes', 'entry.ts', files['entry.ts'].replace('// A', '// B')],
  ]) await mutation(name, () => {
    const before = fs.statSync(path.join(fixture, file), {bigint: true}); put(file, changed)
    const after = fs.statSync(path.join(fixture, file), {bigint: true})
    return {sameMetadataRequired: true, size: String(after.size), mtimeNs: String(after.mtimeNs),
      sizeUnchanged: before.size === after.size, mtimeUnchanged: before.mtimeNs === after.mtimeNs}
  }, () => put(file, files[file]))
  await mutation('higher-priority extension added', () => put('dep.ts', 'export const value=43;\n'), () => fs.rmSync(path.join(fixture, 'dep.ts')))
  await mutation('resolved dependency removed', () => fs.rmSync(path.join(fixture, 'dep.js')), () => put('dep.js', files['dep.js']), {reject: true})
  await mutation('erased original resolution target shadowed', () => put('types.js', 'export const T=1;\n'), () => fs.rmSync(path.join(fixture, 'types.js')))
  await mutation('erased original resolution reaches private adapter', () => fs.symlinkSync(path.join(path.dirname(fileURLToPath(import.meta.url)), 'prelude.mjs'), path.join(fixture, 'types.js')),
    () => fs.rmSync(path.join(fixture, 'types.js')), {reject: true})
  await mutation('package exports bytes changed', () => put('node_modules/quod-input-package/package.json', files['node_modules/quod-input-package/package.json'].replace('./one.mjs', './two.mjs')),
    () => put('node_modules/quod-input-package/package.json', files['node_modules/quod-input-package/package.json']))
  await mutation('tsconfig bytes changed', () => put('tsconfig.json', files['tsconfig.json'].replace('true', 'false')), () => put('tsconfig.json', files['tsconfig.json']))
  await mutation('glob match added', () => put('modules/beta.mjs', 'export default 2;\n'), () => fs.rmSync(path.join(fixture, 'modules/beta.mjs')))
  await mutation('glob match renamed', () => fs.renameSync(path.join(fixture, 'modules/alpha.mjs'), path.join(fixture, 'modules/omega.mjs')),
    () => fs.renameSync(path.join(fixture, 'modules/omega.mjs'), path.join(fixture, 'modules/alpha.mjs')))
  await mutation('symlink target changed', () => { fs.rmSync(path.join(fixture, 'alias.mjs')); fs.symlinkSync('two.mjs', path.join(fixture, 'alias.mjs')) },
    () => { fs.rmSync(path.join(fixture, 'alias.mjs')); fs.symlinkSync('one.mjs', path.join(fixture, 'alias.mjs')) })
  await mutation('unrelated path added conservatively misses', () => put('unrelated.txt', 'not read by compiler\n'),
    () => fs.rmSync(path.join(fixture, 'unrelated.txt')), {identityChanges: false})
  const changedConfig = {...config, seed: 43}
  await mutation('declared configuration changed', () => {}, () => {}, {config: changedConfig})
  const aliasCandidate = chosen.find(row => row.project === 'zod-4.0')
  const aliasConfig = {...options(aliasCandidate), file: config.file}
  await mutation('actual project alias and provided context changed', () => {}, () => {}, {config: aliasConfig})
  for (const [name, mutate] of [
    ['obsolete schema', value => { value.schema += '-obsolete' }],
    ['coherent tool metadata tamper', value => { value.tools.nodeSha256 = '0'.repeat(64) }],
    ['omitted original resolution facts', value => { delete value.compile.resolutionFacts }],
  ]) {
    const value = JSON.parse(seed.bytes); mutate(value)
    const bytes = Buffer.from(canonicalManagedJSON(value) + '\n')
    const invalid = request({bytes, sha256: sha(bytes)}, config, entry)
    const decision = (await validate([invalid]))[0]
    const directory = path.join(out, 'invalid-certificates'); fs.mkdirSync(directory, {recursive: true})
    fs.writeFileSync(path.join(directory, name.replaceAll(' ', '-') + '.json'), bytes)
    write(path.join(directory, name.replaceAll(' ', '-') + '-decision.json'), {trustedCertificateSha256: sha(bytes), decision})
    check(name + ' declines even with constructed matching outer SHA', !decision.hit)
  }
  const bad = {...request(seed, config, entry), trustedCertificateSha256: '0'.repeat(64)}
  check('unauthenticated bytes cannot seed a compile candidate', !(await validate([bad]))[0].hit)
  result.passed = result.checks.every(row => row.pass)
} catch (error) { result.error = {name: error.name, message: error.message, stack: error.stack}; process.exitCode = 1 }
finally { if (fixtureCreated) fs.rmSync(fixture, {recursive: true, force: true}); save() }
console.log(canonicalManagedJSON({passed: result.passed, checks: result.checks.length,
  sharedValidationSeconds: result.sharedValidationSeconds ?? null, error: result.error || null}))
if (!result.passed) process.exitCode = 1
