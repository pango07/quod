// Remote-only diagnostic; never admits a receipt or skips a real command.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {fileURLToPath, pathToFileURL} from 'node:url'
import {spawnSync} from 'node:child_process'
import {createRequire} from 'node:module'
import {captureResolvedCompileCertificate, validateResolvedCompileCertificates, readValidatedCompileIdentity, createCertifiedManagedCompiler, canonicalManagedJSON} from './resolved-compile-certificate.mjs'

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
    const compile = await createCertifiedManagedCompiler({project: request.project, sourceTreeCache: false})
    const built = await compile('runner.mjs', request.options, request.file)
    saveChild('fresh.mjs', Buffer.from(built.bundle))
    saveChild('fresh-build.json', Buffer.from(JSON.stringify({...built, bundle: undefined, options: request.options,
      node: process.version, execArgv: process.execArgv}, null, 2) + '\n'))
    if (request.capture) {
      const certificate = captureResolvedCompileCertificate({project: request.project, built, options: request.options, testFile: request.file})
      saveChild('certificate.json', certificate.bytes)
      saveChild('certificate-summary.json', Buffer.from(JSON.stringify({bytes: certificate.bytes.length, sha256: certificate.sha256}) + '\n'))
    }
    process.exit(0)
  } catch (error) {
    saveChild('error.json', Buffer.from(JSON.stringify({name: error.name, message: error.message, stack: error.stack}, null, 2) + '\n'))
    process.exit(1)
  }
}
if (['--flags-child', '--validate-child'].includes(process.argv[2])) {
  const value = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'))
  const decisions = await validateResolvedCompileCertificates({project: value.project, requests: [{
    certificateBytes: fs.readFileSync(value.certificateFile), trustedCertificateSha256: value.sha256,
    options: value.options, testFile: value.testFile}]})
  console.log(canonicalManagedJSON({execArgv: process.execArgv, decisions})); process.exit(0)
}
if (process.argv[2] === '--matrix-evidence') {
  const [projectArg, matrixArg, outArg] = process.argv.slice(3)
  const project = fs.realpathSync(projectArg), matrix = fs.realpathSync(matrixArg), out = path.resolve(outArg)
  assert(!fs.existsSync(out)); fs.mkdirSync(out)
  const result = JSON.parse(fs.readFileSync(path.join(matrix, 'result.json'), 'utf8'))
  const candidates = result.instances.filter(row => row.decision === 'managed')
  assert(result.passed === true && result.selected === 120 && candidates.length === 108,
    'complete current 108 managed and 120 native matrix required')
  const requests = [], sizes = [], started = performance.now()
  const compile = await createCertifiedManagedCompiler({project})
  const {prepareManagedMatrix} = await import('./preparation.mjs')
  const currentSuite = await prepareManagedMatrix({project, seed: 42})
  const currentCandidates = new Map(currentSuite.prepared.map(row => [row.instanceId, row]))
  assert(currentSuite.selected.length === 120, 'fresh full matrix selection differs')
  for (const row of candidates) {
    assert(/^[0-9a-f]{64}$/.test(row.instanceId))
    const directory = path.join(matrix, 'instances', row.instanceId)
    const previous = JSON.parse(fs.readFileSync(path.join(directory, 'test-build.json'), 'utf8'))
    const candidate = currentCandidates.get(row.instanceId)
    assert(candidate && candidate.file === row.file && candidate.project === row.project, 'fresh current matrix instance differs')
    const testFile = candidate.absolute
    const options = {epoch: 1700000000000, seed: 42, env: {NODE_ENV: 'test'}, vitest: candidate.config,
      file: '/project/' + candidate.file, providedContext: candidate.providedContext,
      ...(candidate.zodAlias ? {zodAlias: candidate.zodAlias} : {})}
    assert(canonicalManagedJSON(options) === canonicalManagedJSON(previous.options) &&
      canonicalManagedJSON(candidate.identity) === canonicalManagedJSON(previous.preparation), 'fresh preparation/configuration differs')
    const built = await compile('runner.mjs', options, testFile)
    built.options = options; built.preparation = candidate.identity
    fs.writeFileSync(path.join(out, row.instanceId + '.mjs'), built.bundle)
    fs.writeFileSync(path.join(out, row.instanceId + '-build.json'), JSON.stringify({...built, bundle: undefined}, null, 2) + '\n')
    assert(fs.readFileSync(path.join(directory, 'managed/worker.mjs')).equals(Buffer.from(built.bundle)) &&
      canonicalManagedJSON(previous.identity) === canonicalManagedJSON(built.identity), 'fresh certified full matrix build differs')
    const certificate = captureResolvedCompileCertificate({project, built, options: built.options, testFile})
    fs.writeFileSync(path.join(out, row.instanceId + '.json'), certificate.bytes)
    requests.push({certificateBytes: certificate.bytes, trustedCertificateSha256: certificate.sha256, options: built.options, testFile})
    sizes.push({instanceId: row.instanceId, certificateBytes: certificate.bytes.length,
      certificateSha256: certificate.sha256, compileIdentityBytes: Buffer.byteLength(canonicalManagedJSON(built.identity)),
      identitySha256: built.identitySha256, bundleSha256: built.sha256})
  }
  const captureSeconds = (performance.now() - started) / 1000, validationStart = performance.now()
  const decisions = await validateResolvedCompileCertificates({project, requests})
  const validationSeconds = (performance.now() - validationStart) / 1000
  const issues = []
  for (const [index, decision] of decisions.entries()) {
    if (!decision.hit || decision.identitySha256 !== sizes[index].identitySha256 || decision.bundleSha256 !== sizes[index].bundleSha256)
      issues.push({instanceId: sizes[index].instanceId, reason: decision.reason || 'identity differs'})
    else if (readValidatedCompileIdentity(decision.proof).identitySha256 !== sizes[index].identitySha256)
      issues.push({instanceId: sizes[index].instanceId, reason: 'opaque proof differs'})
  }
  const evidence = {schema: 'quod-managed-resolved-compile-full-matrix-v1', passed: issues.length === 0,
    productionEnabled: false, receiptAdmission: false, nativeSelected: result.selected,
    nativeConformanceMatrixPassed: result.passed, candidates: sizes.length, captureSeconds, validationSeconds,
    scope: 'Current invocation bundles are read as data only; this step does not execute any retained artifact code or admit receipts.',
    certificateBytes: sizes.reduce((total, row) => total + row.certificateBytes, 0),
    compileIdentityBytes: sizes.reduce((total, row) => total + row.compileIdentityBytes, 0),
    maxCertificateBytes: Math.max(...sizes.map(row => row.certificateBytes)),
    maxCompileIdentityBytes: Math.max(...sizes.map(row => row.compileIdentityBytes)),
    sizes, issues}
  let expandedBytes = 0, retainedPaths = 0
  function countEvidence(file) {
    assert(++retainedPaths <= 20000, 'retained evidence path bound exceeded')
    const stat = fs.lstatSync(file)
    if (stat.isDirectory()) for (const name of fs.readdirSync(file)) countEvidence(path.join(file, name))
    else {
      assert(stat.isFile() && stat.size <= 16 * 1024 * 1024, 'bounded regular retained evidence required')
      expandedBytes += stat.size
      assert(expandedBytes + 16 * 1024 * 1024 <= 1024 * 1024 * 1024, 'whole retained evidence expanded byte bound exceeded')
    }
  }
  for (const root of [out, matrix, path.join(path.dirname(out), 'resolved-compile-certificate'),
    path.join(path.dirname(out), 'resolved-certificate-compiler-checks')]) countEvidence(root)
  evidence.evidenceBounds = {expandedBytesObserved: expandedBytes, pathsObserved: retainedPaths,
    expandedBytesMax: 1024 * 1024 * 1024, reservedOtherBytes: 16 * 1024 * 1024, perFileBytesMax: 16 * 1024 * 1024}
  fs.writeFileSync(path.join(out, 'result.json'), JSON.stringify(evidence, null, 2) + '\n')
  console.log(canonicalManagedJSON({passed: evidence.passed, candidates: sizes.length,
    certificateBytes: evidence.certificateBytes, compileIdentityBytes: evidence.compileIdentityBytes, issues: issues.length}))
  process.exit(evidence.passed ? 0 : 1)
}
const [projectArg, outArg] = process.argv.slice(2), project = fs.realpathSync(projectArg), out = path.resolve(outArg)
assert(!fs.existsSync(out)); fs.mkdirSync(out, {recursive: true})
const fixture = path.join(project, '.quod-resolved-compile-fixture')
assert(!fs.existsSync(fixture))
const result = {schema: 'quod-managed-resolved-compile-certificate-probe-v1', passed: false,
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
const compiler = await createCertifiedManagedCompiler({project})
const fresh = await createCertifiedManagedCompiler({project, sourceTreeCache: false})
const build = async (file, config, target = compiler) => target('runner.mjs', config, file)
const request = (seed, config, file) => ({certificateBytes: seed.bytes, trustedCertificateSha256: seed.sha256,
  options: config, testFile: file})
const capture = (built, config, file) => captureResolvedCompileCertificate({project, built, options: config, testFile: file})
const validate = requests => validateResolvedCompileCertificates({project, requests})
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
function separateCompile(directory, config, file, {script = fileURLToPath(import.meta.url), capture = false} = {}) {
  const childOut = path.join(directory, 'separate-node'), requestFile = path.join(directory, 'child-request.json')
  write(requestFile, {project, options: config, file, out: childOut, capture})
  const argv = [script, '--compile-child', requestFile]
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
async function adapterRelocation(config, entry) {
  const source = path.dirname(fileURLToPath(import.meta.url))
  const copies = ['A', 'B'].map(name => path.join(path.dirname(source), '.quod-certificate-adapter-' + name))
  const created = [], directory = path.join(out, 'adapter-relocation')
  fs.mkdirSync(directory)
  // Copy the complete installed adapter, including its tools, without changing
  // symlink target text. Retain data inventories/certificates, never the copies.
  function inventory(root) {
    const rows = []; let bytes = 0
    function walk(file, name = '.') {
      assert(rows.length < 20000, 'adapter copy path bound')
      const stat = fs.lstatSync(file), row = {name, mode: stat.mode & 0o7777}
      if (stat.isDirectory()) {
        row.kind = 'directory'; rows.push(row)
        for (const child of fs.readdirSync(file).sort()) walk(path.join(file, child), name + '/' + child)
      } else if (stat.isSymbolicLink()) {
        row.kind = 'symlink'; row.target = fs.readlinkSync(file); rows.push(row)
      } else {
        assert(stat.isFile() && stat.size <= 256 * 1024 * 1024, 'bounded regular adapter copy input')
        bytes += stat.size; assert(bytes <= 512 * 1024 * 1024, 'adapter copy byte bound')
        row.kind = 'file'; row.bytes = stat.size; row.sha256 = sha(fs.readFileSync(file)); rows.push(row)
      }
    }
    walk(root); return {rows, bytes}
  }
  try {
    for (const copy of copies) assert(!fs.lstatSync(copy, {throwIfNoEntry: false}), 'owned adapter copy already exists')
    const sourceInventory = inventory(source)
    write(path.join(directory, 'source-inventory.json'), sourceInventory)
    for (const copy of copies) {
      created.push(copy)
      fs.cpSync(source, copy, {recursive: true, force: false, errorOnExist: true, verbatimSymlinks: true})
    }
    const inventories = copies.map(inventory)
    inventories.forEach((value, index) => write(path.join(directory, index ? 'B-inventory.json' : 'A-inventory.json'), value))
    check('adjacent adapter copies retain every source/tool byte mode and symlink target',
      inventories.every(value => canonicalManagedJSON(value) === canonicalManagedJSON(sourceInventory)))
    const certificateModules = await Promise.all(copies.map(copy => import(pathToFileURL(path.join(copy, 'resolved-compile-certificate.mjs')).href)))
    const built = [], certificates = []
    for (let index = 0; index < copies.length; index++) {
      const label = index ? 'B' : 'A', childDirectory = path.join(directory, label + '-child')
      fs.mkdirSync(childDirectory)
      const child = separateCompile(childDirectory, config, entry,
        {script: path.join(copies[index], 'resolved-compile-certificate-probe.mjs'), capture: true})
      check('adapter ' + label + ' fresh plain producer completes', child.status === 0)
      const childOut = path.join(childDirectory, 'separate-node')
      built[index] = JSON.parse(fs.readFileSync(path.join(childOut, 'fresh-build.json'), 'utf8'))
      built[index].bundle = fs.readFileSync(path.join(childOut, 'fresh.mjs'), 'utf8')
      const bytes = fs.readFileSync(path.join(childOut, 'certificate.json'))
      certificates[index] = {bytes, sha256: sha(bytes)}
      for (const [from, to] of [['fresh-build.json', label + '-build.json'], ['fresh.mjs', label + '.mjs'], ['certificate.json', label + '-certificate.json']])
        fs.renameSync(path.join(childOut, from), path.join(directory, to))
    }
    const values = certificates.map(seed => JSON.parse(seed.bytes))
    const positive = (await certificateModules[0].validateResolvedCompileCertificates({project,
      requests: [request(certificates[0], config, entry)]}))[0]
    const relocated = (await certificateModules[1].validateResolvedCompileCertificates({project,
      requests: [request(certificates[0], config, entry)]}))[0]
    const compact = value => ({hit: value.hit, reason: value.reason || null, next: value.next || null,
      identitySha256: value.identitySha256 || null, bundleSha256: value.bundleSha256 || null})
    result.adapterRelocation = {roots: copies, sourceInventorySha256: sha(canonicalManagedJSON(sourceInventory)),
      copiedPaths: sourceInventory.rows.length, copiedBytes: sourceInventory.bytes,
      geometry: values.map(value => value.rootGeometry),
      sameTools: canonicalManagedJSON(values[0].tools) === canonicalManagedJSON(values[1].tools),
      positive: compact(positive), relocated: compact(relocated), bundleSha256: built.map(value => value.sha256),
      certificates: certificates.map(seed => ({bytes: seed.bytes.length, sha256: seed.sha256})),
      bundleBytesEqual: built[0].bundle === built[1].bundle}
    write(path.join(directory, 'result.json'), result.adapterRelocation); save()
    check('relocation fixture has identical complete source inventory and tools', result.adapterRelocation.sameTools)
    check('adapter A certificate validates against its actual physical roots', positive.hit === true)
    check('adapter B rejects A certificate despite equal complete adapter inventory and tools',
      !relocated.hit && relocated.next === 'full-compile' && relocated.reason === 'current physical roots/tool inputs differ')
    check('fresh adjacent adapter bundles expose actual physical path sensitivity', !result.adapterRelocation.bundleBytesEqual)
  } finally {
    for (const copy of created) fs.rmSync(copy, {recursive: true, force: true})
  }
}
let fixtureCreated = false
async function warmToolControls(initial, seed, config, entry) {
  const here = path.dirname(fileURLToPath(import.meta.url)), own = createRequire(import.meta.url)
  const projectRequire = createRequire(path.join(project, 'package.json')), module = await import('./compiler.mjs')
  const directory = path.join(out, 'warm-tools'); fs.mkdirSync(directory)
  result.warmTools = []
  const rejected = async operation => { try { await operation(); return null } catch (error) { return error.message } }
  async function controls(name, change, restore, {freshChild = false} = {}) {
    const target = path.join(directory, name); fs.mkdirSync(target)
    const row = {name}; result.warmTools.push(row)
    try {
      row.mutation = await change(target)
      row.captureError = await rejected(() => capture(initial, config, entry))
      row.buildError = await rejected(() => build(entry, config))
      row.factoryError = await rejected(() => createCertifiedManagedCompiler({project}))
      let plain
      try { plain = await module.createManagedCompiler({project})('runner.mjs', config, entry) }
      catch (error) { row.plainBuildError = error.message }
      if (plain) {
        fs.writeFileSync(path.join(target, 'uncertified.mjs'), plain.bundle)
        write(path.join(target, 'uncertified-build.json'), {...plain, bundle: undefined})
        row.plainCaptureError = await rejected(() => capture(plain, config, entry))
      }
      if (freshChild) row.child = separateCompile(target, config, entry, {capture: true})
      write(path.join(target, 'result.json'), row); save()
      check(name + ' warm build capture and new factory decline changed epoch', row.captureError && row.buildError && row.factoryError)
      check(name + ' ordinary warm compiler cannot gain certified provenance',
        plain ? row.plainCaptureError?.includes('actual certified compiler build required')
          : name === 'typescript-exports' && row.plainBuildError?.includes('QUOD_REPLACED_TS_API'))
      if (freshChild) {
        check(name + ' fresh plain child loads changed compiler and certifies actual changed bytes', row.child.status === 0 &&
          !fs.readFileSync(path.join(target, 'separate-node/fresh.mjs')).equals(Buffer.from(initial.bundle)))
      }
    } finally { await restore() }
    const recovered = await build(entry, config)
    check(name + ' restored exact bytes and loaded API identities recover epoch',
      recovered.bundle === initial.bundle && capture(recovered, config, entry).sha256 === seed.sha256)
  }
  for (const [name, file] of [['compiler-source', path.join(here, 'compiler.mjs')],
    ['esbuild-api', own.resolve('esbuild')], ['typescript-api', projectRequire.resolve('typescript')]]) {
    const bytes = fs.readFileSync(file)
    assert(bytes.length < 16 * 1024 * 1024)
    const changed = name === 'compiler-source' ? Buffer.from(bytes.toString('utf8').replaceAll("target: 'es2022'", "target: 'es2021'"))
      : Buffer.concat([bytes, Buffer.from('\n// QUOD_SAME_VERSION_WARM_TOOL_MUTATION\n')])
    assert(!changed.equals(bytes))
    await controls(name, target => {
      fs.writeFileSync(file, changed); fs.writeFileSync(path.join(target, 'mutated-tool.mjs'), changed)
      return {file, beforeSha256: sha(bytes), changedSha256: sha(changed), packageVersionUnchanged: true}
    }, () => fs.writeFileSync(file, bytes), {freshChild: name === 'compiler-source'})
  }
  const tsModule = own.cache[projectRequire.resolve('typescript')], originalExports = tsModule.exports
  await controls('typescript-exports', () => {
    tsModule.exports = {...originalExports, createSourceFile: () => { throw new Error('QUOD_REPLACED_TS_API') }}
    return {moduleFile: projectRequire.resolve('typescript'), changed: 'module.exports and used createSourceFile'}
  }, () => { tsModule.exports = originalExports })
}
async function apiRouteControls(seed, config, entry) {
  const here = path.dirname(fileURLToPath(import.meta.url)), own = createRequire(import.meta.url)
  const projectRequire = createRequire(path.join(project, 'package.json'))
  const directory = path.join(out, 'api-routes'); fs.mkdirSync(directory)
  result.apiRoutes = []
  for (const [name, root, manifest, entryFile] of [
    ['esbuild', here, own.resolve('esbuild/package.json'), own.resolve('esbuild')],
    ['typescript', project, projectRequire.resolve('typescript/package.json'), projectRequire.resolve('typescript')],
  ]) {
    const target = path.join(directory, name); fs.mkdirSync(target)
    const packageFile = path.join(root, 'package.json'), original = fs.readFileSync(packageFile)
    const names = ['.quod-api-wrapper.mjs', '.quod-api-require.cjs', '.quod-api-manifest.json']
    for (const file of names) assert(!fs.lstatSync(path.join(root, file), {throwIfNoEntry: false}), 'owned API fixture already exists')
    const relative = './' + path.relative(root, entryFile).split(path.sep).join('/')
    const changed = {...JSON.parse(original), name, exports: {'.': {import: './' + names[0], require: './' + names[1]}, './package.json': './' + names[2]}}
    let child, decision
    try {
      fs.writeFileSync(path.join(root, names[0]), 'export * from ' + JSON.stringify(relative) + ';\n', {flag: 'wx'})
      fs.writeFileSync(path.join(root, names[1]), 'module.exports=require(' + JSON.stringify(relative) + ');\n', {flag: 'wx'})
      fs.symlinkSync(manifest, path.join(root, names[2]))
      fs.writeFileSync(packageFile, JSON.stringify(changed) + '\n')
      fs.writeFileSync(path.join(target, 'original-package.json'), original)
      write(path.join(target, 'mutated-package.json'), changed)
      write(path.join(target, 'routes.json'), {manifestTarget: manifest, wrapperSource: fs.readFileSync(path.join(root, names[0]), 'utf8'), requireSource: fs.readFileSync(path.join(root, names[1]), 'utf8')})
      const requestFile = path.join(target, 'request.json')
      write(requestFile, {project, certificateFile: path.join(out, 'fixture-certificate.json'), sha256: seed.sha256, options: config, testFile: entry})
      child = spawnSync(process.execPath, [fileURLToPath(import.meta.url), '--validate-child', requestFile],
        {cwd: project, env: process.env, timeout: 30000, killSignal: 'SIGKILL', maxBuffer: 1024 * 1024})
      fs.writeFileSync(path.join(target, 'stdout.log'), child.stdout || ''); fs.writeFileSync(path.join(target, 'stderr.log'), child.stderr || '')
      decision = child.status === 0 ? JSON.parse(child.stdout.toString('utf8')) : null
      const row = {name, status: child.status, signal: child.signal, error: child.error?.message || null, decision}
      result.apiRoutes.push(row); write(path.join(target, 'result.json'), row); save()
    } finally {
      fs.writeFileSync(packageFile, original)
      for (const file of names) fs.rmSync(path.join(root, file), {force: true})
    }
    check(name + ' fresh actual API entry rejects self-reference wrapper with unchanged manifest target',
      child.status === 0 && decision.decisions[0].hit === false && decision.decisions[0].reason === 'unreviewed actual compiler API or binary entry')
    check(name + ' exact package restoration recovers certificate', (await validate([request(seed, config, entry)]))[0].hit)
  }
}
try {
  const {prepareManagedMatrix} = await import('./preparation.mjs')
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
    const fromProof = readValidatedCompileIdentity(candidateIdentity.proof)
    check(index + ' opaque proof reader yields the complete actual fresh identity',
      canonicalManagedJSON(fromProof.identity) === canonicalManagedJSON(actual.identity) && fromProof.identitySha256 === actual.identitySha256)
    fromProof.identity.bundleSha256 = '0'.repeat(64)
    check(index + ' mutating a read copy cannot modify private proof identity',
      readValidatedCompileIdentity(candidateIdentity.proof).bundleSha256 === actual.sha256 &&
      readValidatedCompileIdentity(candidateIdentity.proof).identity.bundleSha256 === actual.sha256)
    for (const token of [{}, Object.freeze(Object.create(null)), JSON.parse(JSON.stringify(candidateIdentity.proof))]) {
      let rejected = false
      try { readValidatedCompileIdentity(token) } catch { rejected = true }
      check(index + ' forged or serialized token cannot authorize a compile identity', rejected)
    }
  }

  fs.mkdirSync(fixture); fixtureCreated = true
  fs.mkdirSync(path.join(fixture, 'modules'))
  for (const name of ['quod-input-package', 'quod-side-effect-package']) fs.mkdirSync(path.join(fixture, 'node_modules', name), {recursive: true})
  const files = {
    'entry.ts': 'import type {T} from "./types.js"; import {alias as mapped} from "quod-path-value"; import "quod-side-effect-package"; import {value} from "./dep"; import {alias} from "./alias.mjs"; import {pkg} from "quod-input-package"; console.log(value,alias,pkg,mapped,Object.keys(import.meta.glob("./modules/*.mjs"))); class Base { set field(value) { console.log(value) } } class Named extends Base { field=1 } console.log(Object.hasOwn(new Named(),"field")); // A\n',
    'types.ts': 'export type T = number;\n', 'dep.js': 'export const value=42;\n',
    'one.mjs': 'export const alias=1;\n', 'two.mjs': 'export const alias=2;\n',
    'modules/alpha.mjs': 'export default 1;\n',
    'tsconfig.json': '{"compilerOptions":{"useDefineForClassFields":true,"baseUrl":".","paths":{"quod-path-value":["./one.mjs"]}}}\n',
    'node_modules/quod-input-package/package.json': '{"name":"quod-input-package","type":"module","exports":"./one.mjs"}\n',
    'node_modules/quod-input-package/one.mjs': 'export const pkg=1;\n',
    'node_modules/quod-input-package/two.mjs': 'export const pkg=2;\n',
    'node_modules/quod-side-effect-package/package.json': '{"name":"quod-side-effect-package","type":"module","exports":"./index.js","sideEffects":true}\n',
    'node_modules/quod-side-effect-package/index.js': 'console.log("QUOD_SIDE_EFFECT",this);\n',
  }
  const fixed = 1700000000
  const put = (name, bytes) => { const file = path.join(fixture, name); fs.writeFileSync(file, bytes); fs.utimesSync(file, fixed, fixed) }
  for (const [name, bytes] of Object.entries(files)) put(name, bytes)
  fs.symlinkSync('one.mjs', path.join(fixture, 'alias.mjs'))
  const rootCandidate = chosen.find(row => row.project === 'convex-helpers'), entry = path.join(fixture, 'entry.ts')
  const config = {...options(rootCandidate), file: '/project/.quod-resolved-compile-fixture/entry.ts'}
  const initial = await build(entry, config), seed = capture(initial, config, entry)
  fs.writeFileSync(path.join(out, 'fixture-certificate.json'), seed.bytes)
  fs.writeFileSync(path.join(out, 'fixture-seed.mjs'), initial.bundle)
  write(path.join(out, 'fixture-seed-build.json'), {...initial, bundle: undefined, options: config})
  snapshotFixture(path.join(out, 'fixture-baseline'))
  check('erased type-only import has an original resolution fact', initial.identity.resolutionFacts.some(fact =>
    fact.phase === 'original' && fact.specifier === './types.js' && fact.target.endsWith('/types.ts')) &&
    !initial.identity.resolutionFacts.some(fact => fact.phase === 'emitted' && fact.specifier === './types.js'))
  check('unchanged small fixture validates', (await validate([request(seed, config, entry)]))[0].hit)
  let unissuedRejected = false, changedRejected = false
  try { capture({...initial}, config, entry) } catch { unissuedRejected = true }
  const originalIdentity = initial.identity
  try {
    initial.identity = {...originalIdentity, bundleSha256: '0'.repeat(64)}
    try { capture(initial, config, entry) } catch { changedRejected = true }
  } finally { initial.identity = originalIdentity }
  check('deserialized or copied build object has no certified issuer', unissuedRejected)
  check('mutating certified build identity invalidates its issuer binding', changedRejected)
  async function mutation(name, change, restore, {reject = false, config: currentConfig = config, identityChanges = true, hit = false} = {}) {
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
      check(name + (hit ? ' certificate accepts current equal inputs' : ' certificate miss requires full compilation'),
        hit ? decision.hit === true : !decision.hit && decision.next === 'full-compile')
      check(name + ' actual fresh compiler confirms expected changed or rejected input', reject ? Boolean(rejection)
        : !rejection && (rebuilt.identitySha256 !== initial.identitySha256) === identityChanges)
      if (hit) check(name + ' accepted candidate equals exact fresh bundle and full compile identity',
        rebuilt.bundle === initial.bundle && canonicalManagedJSON(rebuilt.identity) === canonicalManagedJSON(initial.identity) &&
        canonicalManagedJSON(readValidatedCompileIdentity(decision.proof).identity) === canonicalManagedJSON(rebuilt.identity))
      if (name === 'tsconfig bytes changed') {
        const child = JSON.parse(fs.readFileSync(path.join(directory, 'separate-node/fresh-build.json'), 'utf8'))
        check('tsconfig metadata mutation has equal exact bundle and full identity in both fresh compilers',
          row.separateNode.status === 0 && row.separateNode.execArgv.length === 0 &&
          canonicalManagedJSON(rebuilt.identity) === canonicalManagedJSON(initial.identity) &&
          canonicalManagedJSON(child.identity) === canonicalManagedJSON(initial.identity) &&
          rebuilt.bundle === initial.bundle && fs.readFileSync(path.join(directory, 'separate-node/fresh.mjs')).equals(Buffer.from(initial.bundle)))
      }
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
  await mutation('package sideEffects metadata ignored by path-only plugin',
    () => put('node_modules/quod-side-effect-package/package.json', files['node_modules/quod-side-effect-package/package.json'].replace('"sideEffects":true', '"sideEffects":false')),
    () => put('node_modules/quod-side-effect-package/package.json', files['node_modules/quod-side-effect-package/package.json']), {identityChanges: false, hit: true})
  await mutation('package type metadata ignored by path-only plugin',
    () => put('node_modules/quod-side-effect-package/package.json', files['node_modules/quod-side-effect-package/package.json'].replace('"type":"module"', '"type":"commonjs"')),
    () => put('node_modules/quod-side-effect-package/package.json', files['node_modules/quod-side-effect-package/package.json']), {identityChanges: false, hit: true})
  // Failed runs 42408563595 and 42409190659 retained the correction evidence:
  // this path-only resolver drops TSConfig transform metadata before onLoad.
  // A separate plain Node produced the exact same full identity and bundle.
  await mutation('tsconfig bytes changed', () => put('tsconfig.json', files['tsconfig.json'].replace('true', 'false')),
    () => put('tsconfig.json', files['tsconfig.json']), {identityChanges: false, hit: true})
  await mutation('tsconfig paths changes current resolver target',
    () => put('tsconfig.json', files['tsconfig.json'].replace('./one.mjs', './two.mjs')), () => put('tsconfig.json', files['tsconfig.json']))
  await mutation('glob match added', () => put('modules/beta.mjs', 'export default 2;\n'), () => fs.rmSync(path.join(fixture, 'modules/beta.mjs')))
  await mutation('glob match renamed', () => fs.renameSync(path.join(fixture, 'modules/alpha.mjs'), path.join(fixture, 'modules/omega.mjs')),
    () => fs.renameSync(path.join(fixture, 'modules/omega.mjs'), path.join(fixture, 'modules/alpha.mjs')))
  await mutation('symlink target changed', () => { fs.rmSync(path.join(fixture, 'alias.mjs')); fs.symlinkSync('two.mjs', path.join(fixture, 'alias.mjs')) },
    () => { fs.rmSync(path.join(fixture, 'alias.mjs')); fs.symlinkSync('one.mjs', path.join(fixture, 'alias.mjs')) })
  await mutation('unrelated path added allows equal input candidate', () => put('unrelated.txt', 'not read by compiler\n'),
    () => fs.rmSync(path.join(fixture, 'unrelated.txt')), {identityChanges: false, hit: true})
  await mutation('unrelated JSON added allows equal input candidate', () => put('unrelated.json', '{"unused":42}\n'),
    () => fs.rmSync(path.join(fixture, 'unrelated.json')), {identityChanges: false, hit: true})
  const changedConfig = {...config, seed: 43}
  await mutation('declared configuration changed', () => {}, () => {}, {config: changedConfig})
  const aliasCandidate = chosen.find(row => row.project === 'zod-4.0')
  const aliasConfig = {...options(aliasCandidate), file: config.file}
  await mutation('actual project alias and provided context changed', () => {}, () => {}, {config: aliasConfig})
  for (const [name, mutate] of [
    ['obsolete schema', value => { value.schema += '-obsolete' }],
    ['coherent tool metadata tamper', value => { value.tools.nodeSha256 = '0'.repeat(64) }],
    ['omitted original resolution facts', value => { delete value.compile.resolutionFacts }],
    ['omitted physical input coverage', value => { delete value.compile.inputCoverage }],
    ['coherent missing physical input', value => { value.compile.inputCoverage.physicalSources.pop() }],
    ['unknown virtual input', value => { value.compile.inputCoverage.virtualInputs.push('<unknown>') }],
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
  await warmToolControls(initial, seed, config, entry)
  await apiRouteControls(seed, config, entry)
  const flagsRequest = path.join(out, 'flags-request.json')
  write(flagsRequest, {project, certificateFile: path.join(out, 'fixture-certificate.json'), sha256: seed.sha256, options: config, testFile: entry})
  const flagsChild = spawnSync(process.execPath, ['--stack-trace-limit=17', fileURLToPath(import.meta.url), '--flags-child', flagsRequest],
    {cwd: project, env: process.env, timeout: 30000, killSignal: 'SIGKILL', maxBuffer: 1024 * 1024})
  fs.writeFileSync(path.join(out, 'flags-child-stdout.log'), flagsChild.stdout || '')
  fs.writeFileSync(path.join(out, 'flags-child-stderr.log'), flagsChild.stderr || '')
  const flagsEvidence = {status: flagsChild.status, signal: flagsChild.signal, error: flagsChild.error?.message || null,
    result: flagsChild.status === 0 ? JSON.parse(flagsChild.stdout.toString('utf8')) : null}
  write(path.join(out, 'flags-child.json'), flagsEvidence)
  check('actual nonempty Node flags decline before resolver or proof issuance',
    flagsEvidence.status === 0 && flagsEvidence.result.execArgv[0] === '--stack-trace-limit=17' &&
    flagsEvidence.result.decisions[0].hit === false && flagsEvidence.result.decisions[0].reason === 'unreviewed compiler Node CLI flags')
  await adapterRelocation(config, entry)
  result.passed = result.checks.every(row => row.pass)
} catch (error) { result.error = {name: error.name, message: error.message, stack: error.stack}; process.exitCode = 1 }
finally { if (fixtureCreated) fs.rmSync(fixture, {recursive: true, force: true}); save() }
console.log(canonicalManagedJSON({passed: result.passed, checks: result.checks.length,
  sharedValidationSeconds: result.sharedValidationSeconds ?? null, error: result.error || null}))
if (!result.passed) process.exitCode = 1
