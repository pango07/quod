// Diagnostic only. A certificate authenticates no response and starts no worker.
// Native preparation remains mandatory. Invalid certificates mean full compile.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {fileURLToPath} from 'node:url'
import os from 'node:os'
import {spawnSync} from 'node:child_process'
export function canonicalManagedJSON(value) {
  function visit(item) {
    if (item === null || typeof item === 'string' || typeof item === 'boolean') return item
    if (typeof item === 'number') {
      assert(Number.isFinite(item) && !Object.is(item, -0), 'non-canonical configuration number')
      return item
    }
    assert(typeof item === 'object', 'configuration must contain JSON values only')
    if (Array.isArray(item)) {
      assert.equal(Object.getOwnPropertySymbols(item).length, 0, 'symbol configuration keys unsupported')
      assert.equal(Object.getOwnPropertyNames(item).length, item.length + 1, 'sparse/custom arrays unsupported')
      const result = []
      Object.setPrototypeOf(result, null)
      for (let index = 0; index < item.length; index++) {
        const descriptor = Object.getOwnPropertyDescriptor(item, String(index))
        assert(descriptor?.enumerable && 'value' in descriptor, 'configuration array accessors unsupported')
        result[index] = visit(descriptor.value)
      }
      return result
    }
    assert(Object.getPrototypeOf(item) === Object.prototype || Object.getPrototypeOf(item) === null,
      'configuration must contain plain objects')
    assert.equal(Object.getOwnPropertySymbols(item).length, 0, 'symbol configuration keys unsupported')
    const result = Object.create(null)
    for (const key of Object.getOwnPropertyNames(item).sort()) {
      const descriptor = Object.getOwnPropertyDescriptor(item, key)
      assert(descriptor.enumerable && 'value' in descriptor, 'configuration accessors/non-enumerable keys unsupported')
      result[key] = visit(descriptor.value)
    }
    return result
  }
  return JSON.stringify(visit(value))
}


const here = path.dirname(fileURLToPath(import.meta.url))
const SCHEMA = 'quod-managed-resolved-compile-certificate-v1-diagnostic'
// Reviewed public compiler plus the complete fresh npm-ci four-package tree.
// A matching package version is not approval for a different implementation.
const REVIEWED_COMPILER_SHA256 = 'a87f1f8fafea2e0f1b4a1e299199f5581172a2fe90f59882d3c64eb50f68acaf'
const REVIEWED_TOOL_FILES_SHA256 = '7b7ae881ef3186e4b564c26ffa6e8d40b3d3875a15e41b10d93a6bb24cb3a354'
const LIMITS = {paths: 100000, resolverBytes: 16 * 1024 * 1024, resolverTimeoutMs: 30000,
  toolBytes: 128 * 1024 * 1024, sourceBytes: 256 * 1024 * 1024,
  fileBytes: 16 * 1024 * 1024, certificateBytes: 16 * 1024 * 1024,
  certificateSetBytes: 128 * 1024 * 1024, certificates: 512}
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const json = canonicalManagedJSON
const hash = value => sha(json(value))
const digest = value => typeof value === 'string' && /^[0-9a-f]{64}$/.test(value)
const proofs = new WeakMap()
const certifiedBuilds = new WeakMap()
let compilerEpoch = null, initialization = null
const inside = (root, file) => file === root || file.startsWith(root + path.sep)
const slash = value => value.split(path.sep).join('/')
const exact = (value, keys) => assert(value && Object.getPrototypeOf(value) === Object.prototype &&
  Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key)), 'certificate fields differ')

function context(projectArg, adapterArg) {
  const project = fs.realpathSync(projectArg), adapter = fs.realpathSync(adapterArg)
  assert(!inside(project, adapter) && !inside(adapter, project), 'disjoint certificate roots required')
  const roots = [['/project', project], ['/quod-adapter', adapter]]
  function name(file) {
    const found = roots.find(([, root]) => inside(root, file))
    assert(found, 'external certificate path unsupported')
    const relative = slash(path.relative(found[1], file))
    assert(!relative.split('/').includes('.git'), 'Git objects are not certificate inputs')
    return found[0] + (relative ? '/' + relative : '')
  }
  function file(canonical) {
    assert(typeof canonical === 'string', 'canonical input path required')
    const found = roots.find(([prefix]) => canonical.startsWith(prefix + '/'))
    assert(found, 'external or root certificate input unsupported')
    const relative = canonical.slice(found[0].length + 1)
    assert(relative && !relative.split('/').some(part => !part || part === '.' || part === '..' || part === '.git'), 'noncanonical input path')
    const target = path.join(found[1], relative)
    assert(name(target) === canonical && name(fs.realpathSync(target)) === canonical, 'certificate input is no longer canonical')
    return target
  }
  return {project, adapter, roots, name, file}
}

function regular(file, maximum = LIMITS.fileBytes) {
  const before = fs.lstatSync(file, {bigint: true})
  assert(before.isFile() && before.size <= BigInt(maximum), 'bounded regular input required')
  const bytes = fs.readFileSync(file), after = fs.lstatSync(file, {bigint: true})
  assert(BigInt(bytes.length) === before.size && before.size === after.size && before.ino === after.ino &&
    before.dev === after.dev && before.mtimeNs === after.mtimeNs && before.ctimeNs === after.ctimeNs,
  'input changed during sampled read')
  return bytes
}
// The builtin-only bootstrap is imported before project/tool modules. Retain
// its own and the running Node file observations immediately, not at capture.
const initialSelfSha256 = sha(regular(fileURLToPath(import.meta.url)))
const initialNodeSha256 = sha(regular(fs.realpathSync(process.execPath), LIMITS.toolBytes))

// Unlike the conservative certificate profile, this profile does not snapshot
// unrelated namespace paths or JSON. Every original and emitted query is rerun
// in a new plain Node process; physical module bytes come only from onLoad.
const geometry = ctx => ({project: ctx.project, adapter: ctx.adapter})

// Only an actual validation in this module instance can issue this token.
// A ledger factory must call this reader, not trust the diagnostic identity
// property or deserialize a token. The returned copy cannot edit stored proof.
export function readValidatedCompileIdentity(proof) {
  assert(proof && typeof proof === 'object' && proofs.has(proof), 'unissued compile equivalence proof')
  return JSON.parse(proofs.get(proof))
}

function resolveInNewNode(ctx, requests, tools) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'quod-resolved-certificate-'))
  fs.chmodSync(directory, 0o700)
  try {
    const requestFile = path.join(directory, 'request.json')
    const bytes = Buffer.from(json({project: ctx.project, adapterRoot: ctx.adapter, requests,
      expectedToolsSha256: hash(tools)}) + '\n')
    assert(bytes.length <= LIMITS.resolverBytes, 'resolver request byte bound exceeded')
    fs.writeFileSync(requestFile, bytes, {flag: 'wx', mode: 0o600})
    const child = spawnSync(process.execPath, [fileURLToPath(import.meta.url), '--resolve-child', requestFile],
      {cwd: ctx.project, env: process.env, timeout: LIMITS.resolverTimeoutMs,
        killSignal: 'SIGKILL', maxBuffer: LIMITS.resolverBytes})
    assert(!child.error && child.status === 0 && child.signal === null,
      'fresh plain resolver Node failed: ' + (child.error?.message || child.stderr?.toString('utf8').slice(0, 2048) || child.status))
    assert(child.stdout.length <= LIMITS.resolverBytes && child.stderr.length <= LIMITS.resolverBytes, 'resolver output bound exceeded')
    const value = JSON.parse(child.stdout.toString('utf8'))
    assert(Buffer.from(json(value) + '\n').equals(child.stdout), 'noncanonical resolver response')
    exact(value, ['schema', 'passed', 'requestSha256', 'toolIdentitySha256', 'rootGeometry', 'targets', 'node', 'execArgv'])
    assert(value.schema === 'quod-managed-resolved-import-child-v1' && value.passed === true &&
      value.requestSha256 === sha(bytes) && value.toolIdentitySha256 === hash(tools) &&
      json(value.rootGeometry) === json(geometry(ctx)) && value.node === process.version &&
      Array.isArray(value.execArgv) && value.execArgv.length === 0 &&
      Array.isArray(value.targets) && value.targets.length === requests.length && value.targets.every(target => typeof target === 'string'),
    'resolver response identity differs')
    return {targets: value.targets, requestSha256: value.requestSha256,
      toolIdentitySha256: value.toolIdentitySha256, node: value.node, execArgv: value.execArgv,
      profile: 'one new plain Node process and one new resolver-only esbuild build'}
  } finally { fs.rmSync(directory, {recursive: true, force: true}) }
}

function toolIdentity(ctx) {
  assert(process.platform === 'linux' && process.arch === 'x64', 'finite Linux x64 compiler profile required')
  assert(process.version === 'v24.18.0', 'unreviewed compiler Node version')
  assert.equal(process.execArgv.length, 0, 'unreviewed compiler Node CLI flags')
  for (const key of ['NODE_PATH', 'NODE_OPTIONS', 'NODE_PRESERVE_SYMLINKS', 'NODE_COMPILE_CACHE', 'ESBUILD_BINARY_PATH'])
    assert(!process.env[key], 'unreviewed compiler environment: ' + key)
  const own = createRequire(import.meta.url), project = createRequire(path.join(ctx.project, 'package.json'))
  const packages = [own.resolve('esbuild/package.json'), own.resolve('@esbuild/linux-x64/package.json'), project.resolve('typescript/package.json')]
  const wrapperAPI = path.join(fs.realpathSync(path.dirname(packages[2])), 'lib/typescript.js')
  const delegated = createRequire(wrapperAPI)
  packages.push(delegated.resolve('@typescript/old/package.json'))
  const roots = packages.map(file => fs.realpathSync(path.dirname(file)))
  const esbuildAPI = path.join(roots[0], 'lib/main.js'), tsAPI = path.join(roots[2], 'lib/typescript.js')
  const delegatedAPI = path.join(roots[3], 'lib/typescript.js')
  const binary = path.join(roots[1], 'bin/esbuild')
  const entries = {esbuildESM: fs.realpathSync(fileURLToPath(import.meta.resolve('esbuild'))),
    esbuildCJS: fs.realpathSync(own.resolve('esbuild')), typescriptCJS: fs.realpathSync(project.resolve('typescript')),
    typescriptDelegatedCJS: fs.realpathSync(delegated.resolve('@typescript/old')),
    esbuildBinary: fs.realpathSync(createRequire(esbuildAPI).resolve('@esbuild/linux-x64/bin/esbuild'))}
  assert(entries.esbuildESM === esbuildAPI && entries.esbuildCJS === esbuildAPI &&
    entries.typescriptCJS === tsAPI && entries.typescriptDelegatedCJS === delegatedAPI && entries.esbuildBinary === binary,
  'unreviewed actual compiler API or binary entry')
  assert(packages.slice(0, 2).every(file => JSON.parse(regular(file).toString('utf8')).version === '0.28.2'), 'unreviewed esbuild source version')
  const wrapperPackage = JSON.parse(regular(packages[2]).toString('utf8')), delegatedPackage = JSON.parse(regular(packages[3]).toString('utf8'))
  assert(wrapperPackage.name === '@typescript/typescript6' && wrapperPackage.version === '6.0.2' &&
    delegatedPackage.name === 'typescript' && delegatedPackage.version === '6.0.3', 'unreviewed TypeScript source version')
  assert(sha(regular(tsAPI)) === 'd3f3cd2b04b7f466f4484df921b744223f7bd1f3e353ec9110bdf52695b983d5', 'unreviewed TypeScript delegation wrapper')
  const files = [], counts = {paths: 0, bytes: 0}
  function walk(file, label) {
    assert(++counts.paths <= LIMITS.paths, 'compiler tool path bound exceeded')
    const st = fs.lstatSync(file)
    if (st.isDirectory()) {
      files.push({name: label, kind: 'directory', mode: st.mode & 0o7777})
      for (const child of fs.readdirSync(file).sort()) walk(path.join(file, child), label + '/' + child)
    } else {
      assert(st.isFile(), 'nonregular compiler tool unsupported')
      const bytes = regular(file, LIMITS.toolBytes)
      counts.bytes += bytes.length
      assert(counts.bytes <= LIMITS.toolBytes, 'compiler tool byte bound exceeded')
      files.push({name: label, kind: 'file', mode: st.mode & 0o7777, bytes: bytes.length, sha256: sha(bytes)})
    }
  }
  for (const manifest of packages) walk(path.dirname(manifest), ctx.name(fs.realpathSync(path.dirname(manifest))))
  assert(hash(files) === REVIEWED_TOOL_FILES_SHA256, 'unreviewed compiler tool file inventory')
  const hostSources = ['compiler.mjs', 'resolved-compile-certificate.mjs'].map(name => ({name,
    sha256: sha(regular(path.join(here, name)))}))
  assert(hostSources[0].sha256 === REVIEWED_COMPILER_SHA256, 'unreviewed compiler source bytes')
  const nodeSha256 = sha(regular(fs.realpathSync(process.execPath), LIMITS.toolBytes))
  assert(hostSources[1].sha256 === initialSelfSha256 && nodeSha256 === initialNodeSha256,
    'running bootstrap or Node file epoch changed')
  return {platform: process.platform, arch: process.arch, node: process.version,
    nodeSha256,
    typescript: {wrapper: {name: wrapperPackage.name, version: wrapperPackage.version},
      implementation: {name: delegatedPackage.name, version: delegatedPackage.version}},
    environment: {NODE_ENV: process.env.NODE_ENV ?? null, ESBUILD_WORKER_THREADS: process.env.ESBUILD_WORKER_THREADS ?? null},
    adapterPackageJsonSha256: sha(regular(path.join(here, 'package.json'))),
    packageRoots: roots.map(root => ctx.name(root)),
    entries: Object.fromEntries(Object.entries(entries).map(([key, file]) => [key, ctx.name(file)])),
    files, hostSources, counts}
}

function toolCache(ctx, tools) {
  const roots = tools.packageRoots.map(name => ctx.file(name))
  return new Map(Object.entries(createRequire(import.meta.url).cache)
    .filter(([file]) => roots.some(root => inside(root, file))))
}

const usedTS = ['version', 'createSourceFile', 'forEachChild', 'isBinaryExpression', 'isCallExpression',
  'isElementAccessExpression', 'isExportDeclaration', 'isExternalModuleReference', 'isIdentifier',
  'isImportDeclaration', 'isImportEqualsDeclaration', 'isMetaProperty', 'isNamedImports', 'isNewExpression',
  'isParenthesizedExpression', 'isPropertyAccessExpression', 'isStringLiteral', 'isStringLiteralLike',
  'isVariableStatement', 'ScriptKind', 'ScriptTarget', 'SyntaxKind']

function snapshotAPIs(ctx, tools) {
  const cache = createRequire(import.meta.url).cache
  return [[tools.entries.esbuildCJS, ['build', 'version']], [tools.entries.typescriptCJS, usedTS],
    [tools.entries.typescriptDelegatedCJS, usedTS]].map(([file, names]) => {
    const module = cache[ctx.file(file)]
    assert(module && module.exports, 'expected loaded compiler API missing')
    return {module, exports: module.exports, properties: names.map(name => {
      const descriptor = Object.getOwnPropertyDescriptor(module.exports, name), value = module.exports[name]
      assert(descriptor, 'missing used compiler API: ' + name)
      let enumDescriptors = null
      if (['ScriptKind', 'ScriptTarget', 'SyntaxKind'].includes(name)) {
        enumDescriptors = Object.getOwnPropertyDescriptors(value)
        assert(Reflect.ownKeys(enumDescriptors).length <= 5000 && Object.values(enumDescriptors).every(row =>
          'value' in row && ['string', 'number'].includes(typeof row.value)), 'unreviewed TypeScript enum shape')
      }
      return {name, descriptor, value, enumDescriptors}
    })}
  })
}

function assertCompilerEpoch(ctx) {
  assert(compilerEpoch && json(geometry(ctx)) === compilerEpoch.geometry, 'unknown compiler process epoch')
  const currentTools = toolIdentity(ctx)
  if (json(currentTools) !== compilerEpoch.tools) {
    const expectedTools = JSON.parse(compilerEpoch.tools)
    const boundedEnvironment = value => Object.fromEntries(Object.entries(value).map(([key, entry]) =>
      [key, typeof entry === 'string' ? entry.slice(0, 80) : entry]))
    assert.fail('compiler tool epoch changed; ' + json({expectedToolsSha256: sha(compilerEpoch.tools),
      currentToolsSha256: hash(currentTools),
      expectedEnvironment: boundedEnvironment(expectedTools.environment),
      currentEnvironment: boundedEnvironment(currentTools.environment),
      changedTopLevelFields: [...new Set([...Object.keys(expectedTools), ...Object.keys(currentTools)])].sort()
        .filter(key => json(expectedTools[key] ?? null) !== json(currentTools[key] ?? null))}))
  }
  const current = toolCache(ctx, JSON.parse(compilerEpoch.tools))
  assert(current.size === compilerEpoch.modules.size && [...current].every(([file, module]) =>
    compilerEpoch.modules.get(file) === module), 'loaded compiler module epoch changed')
  for (const api of compilerEpoch.apis) {
    assert(api.module.exports === api.exports, 'loaded compiler exports changed')
    for (const property of api.properties) {
      assert.deepEqual(Object.getOwnPropertyDescriptor(api.exports, property.name), property.descriptor, 'used compiler export descriptor changed')
      assert(api.exports[property.name] === property.value, 'used compiler export value changed')
      if (property.enumDescriptors) assert.deepEqual(Object.getOwnPropertyDescriptors(property.value), property.enumDescriptors, 'used TypeScript enum changed')
    }
  }
}

/**
 * Builtin-only bootstrap: call before importing preparation/planner/dispatcher,
 * which may indirectly import the ordinary compiler. Unknown prior tool loads
 * decline instead of stamping a warm module with current on-disk bytes.
 */
export async function createCertifiedManagedCompiler({project, adapterRoot = here, sourceTreeCache = true}) {
  const ctx = context(project, adapterRoot)
  if (!compilerEpoch) {
    if (!initialization) initialization = (async () => {
      const tools = toolIdentity(ctx), fingerprint = json(tools)
      assert(toolCache(ctx, tools).size === 0, 'compiler tools already loaded outside certified epoch')
      const beforeCache = new Set(Object.keys(createRequire(import.meta.url).cache))
      const module = await import('./compiler.mjs')
      assert(json(toolIdentity(ctx)) === fingerprint, 'compiler tool epoch changed during import')
      assert(module.MANAGED_COMPILER_LOADED_IDENTITY.compilerSha256 === tools.hostSources[0].sha256 &&
        fs.realpathSync(fileURLToPath(module.MANAGED_COMPILER_LOADED_IDENTITY.esbuildAPI)) === ctx.file(tools.entries.esbuildESM) &&
        module.MANAGED_COMPILER_LOADED_IDENTITY.esbuildVersion === '0.28.2', 'loaded compiler code differs from observed epoch')
      // This is the first project TypeScript load in this certified process.
      createRequire(path.join(ctx.project, 'package.json'))('typescript')
      assert(json(toolIdentity(ctx)) === fingerprint, 'compiler tool epoch changed during TypeScript load')
      const cache = createRequire(import.meta.url).cache
      assert(cache[ctx.file(tools.entries.typescriptCJS)].exports === cache[ctx.file(tools.entries.typescriptDelegatedCJS)].exports,
        'TypeScript wrapper did not expose the reviewed delegated API')
      assert(cache[ctx.file(tools.entries.typescriptDelegatedCJS)].exports.version === '6.0.3', 'loaded delegated TypeScript API version differs')
      const loadedTools = toolCache(ctx, tools)
      assert(Object.keys(cache).every(file => beforeCache.has(file) || loadedTools.has(file)), 'unreviewed compiler tool dependency loaded')
      compilerEpoch = {geometry: json(geometry(ctx)), tools: fingerprint, modules: toolCache(ctx, tools),
        apis: snapshotAPIs(ctx, tools), module}
    })()
    await initialization
  }
  assertCompilerEpoch(ctx)
  const compile = compilerEpoch.module.createManagedCompiler({project: ctx.project, adapterRoot: ctx.adapter, sourceTreeCache})
  assertCompilerEpoch(ctx)
  return async (...args) => {
    assertCompilerEpoch(ctx)
    const built = await compile(...args)
    assertCompilerEpoch(ctx)
    certifiedBuilds.set(built, {tools: compilerEpoch.tools, identity: json(built.identity),
      identitySha256: built.identitySha256, bundleSha256: built.sha256, closure: json(built.closure)})
    return built
  }
}

function currentSelection(ctx, {entry = 'runner.mjs', testFile, options}) {
  assert(entry === 'runner.mjs', 'runner-only certificate profile')
  return {entry: ctx.name(fs.realpathSync(path.join(ctx.adapter, entry))),
    testEntry: ctx.name(fs.realpathSync(testFile)), configurationSha256: hash(options)}
}

function sourceObservations(ctx, compile, cache = new Map(), retained = {bytes: 0}) {
  assert(Array.isArray(compile.sources) && compile.sources.length <= LIMITS.paths, 'source path bound exceeded')
  const grouped = new Map(), rows = []
  for (const source of compile.sources) {
    const prior = grouped.get(source.source) || []; prior.push(source); grouped.set(source.source, prior)
  }
  let total = 0
  for (const name of [...grouped.keys()].sort()) {
    let bytes = cache.get(name)
    if (!bytes) {
      bytes = regular(ctx.file(name)); retained.bytes += bytes.length
      assert(retained.bytes <= LIMITS.sourceBytes, 'shared source byte bound exceeded')
      cache.set(name, bytes)
    }
    total += bytes.length
    assert(total <= LIMITS.sourceBytes, 'source byte bound exceeded')
    const source = grouped.get(name)
    assert(source.every(row => row.sourceSha256 === sha(bytes.toString('utf8'))), 'loaded source bytes changed')
    rows.push({name, bytes: bytes.length, rawSha256: sha(bytes)})
  }
  return rows
}

function checkedFacts(ctx, compile, selection) {
  assert(compile.schema === 'quod-managed-compile-v1-research' && Array.isArray(compile.resolutionFacts) && compile.resolutionFacts.length,
    'complete original/emitted resolution facts required')
  assert(compile.esbuildVersion === '0.28.2', 'unreviewed compiled esbuild source version')
  assert(compile.typescriptVersion === '6.0.3', 'unreviewed compiled TypeScript source version')
  assert(compile.compilerSha256 === sha(regular(path.join(here, 'compiler.mjs'))), 'compiler bytes differ')
  for (const key of ['entry', 'testEntry', 'configurationSha256']) assert(compile[key] === selection[key], 'current selection/config differs')
  exact(compile.inputCoverage, ['schema', 'physicalSources', 'virtualInputs'])
  assert(compile.inputCoverage.schema === 'quod-managed-physical-input-coverage-v1', 'physical input coverage required')
  assert(json(compile.inputCoverage.physicalSources) === json(compile.sources.map(row => row.source).sort()), 'physical input/source multiset differs')
  assert(Array.isArray(compile.inputCoverage.virtualInputs) && compile.inputCoverage.virtualInputs.every(value =>
    value === '<define:__QUOD_CONFIG__>' || value === 'quod-managed-als:public-async-local-storage'), 'unknown virtual input coverage')
  assert(json(compile.inputCoverage.virtualInputs) === json([...new Set(compile.inputCoverage.virtualInputs)].sort()), 'noncanonical virtual input coverage')
  assert(compile.inputCoverage.virtualInputs.includes('quod-managed-als:public-async-local-storage') ===
    compile.resolutionFacts.some(row => row.phase === 'emitted' && row.route === 'public-als'), 'ALS virtual input/fact mismatch')
  const requests = [], targets = []
  for (const fact of compile.resolutionFacts) {
    exact(fact, ['phase', 'source', 'specifier', 'kind', 'route', 'target', 'zodAlias'])
    assert(['original', 'emitted'].includes(fact.phase) && typeof fact.specifier === 'string', 'resolution fact differs')
    assert(fact.zodAlias === (compile.policy.zodAlias || null), 'resolution alias differs')
    if (fact.route === 'ordinary') {
      const importer = fact.source ? ctx.file(fact.source) : ''
      const target = ctx.file(fact.target)
      requests.push({path: fact.kind === 'entry-point' ? target : fact.specifier, importer,
        resolveDir: importer ? path.dirname(importer) : ctx.project, kind: fact.kind, zodAlias: fact.zodAlias})
      targets.push(target)
    } else if (fact.route === 'test-entry') {
      assert(fact.source === selection.entry && fact.specifier === '__QUOD_TEST_ENTRY__' &&
        fact.kind === 'dynamic-import' && fact.target === selection.testEntry, 'test-entry resolution differs')
    } else if (fact.route === 'native-als') {
      assert(fact.source === '/quod-adapter/prelude.mjs' && fact.specifier === 'node:async_hooks' &&
        fact.kind === 'import-statement' && fact.target === 'node:async_hooks', 'native ALS resolution differs')
    } else if (fact.route === 'public-als') {
      ctx.file(fact.source)
      assert(['async_hooks', 'node:async_hooks'].includes(fact.specifier) && fact.kind === 'import-statement' &&
        fact.target === '/virtual/public-async-local-storage', 'public ALS resolution differs')
    } else if (fact.route === 'als-implementation') {
      assert(fact.phase === 'emitted' && fact.source === '/virtual/public-async-local-storage' &&
        fact.specifier === 'quod:als-implementation' && fact.target === '/quod-adapter/prelude.mjs', 'private ALS resolution differs')
    } else if (fact.route === 'injected-helper') {
      ctx.file(fact.source)
      assert(fact.phase === 'emitted' && fact.specifier === 'quod:compiler-capabilities' &&
        fact.target === '/quod-adapter/prelude.mjs', 'injected helper resolution differs')
    } else throw new Error('unknown resolution route')
  }
  return {requests, targets}
}

function currentGlobs(ctx, compile) {
  return compile.globs.map(glob => {
    const dir = path.dirname(ctx.file(glob.source))
    assert(typeof glob.pattern === 'string' && /^(?:\.\/|\.\.\/)/.test(glob.pattern), 'relative glob required')
    const wildcard = glob.pattern.search(/[*?\[\]{}]/), prefix = wildcard < 0 ? glob.pattern : glob.pattern.slice(0, wildcard)
    assert(inside(ctx.project, path.resolve(dir, prefix)), 'glob search root outside project')
    const names = []
    for (const name of fs.globSync(glob.pattern, {cwd: dir})) {
      assert(names.length < LIMITS.paths, 'glob match bound exceeded'); names.push(name)
    }
    const matches = names.map(name => {
      const real = fs.realpathSync(path.resolve(dir, name))
      assert(inside(ctx.project, real) && fs.statSync(real).isFile(), 'glob outside project')
      const relative = slash(name)
      return {name: relative.startsWith('../') || relative.startsWith('./') ? relative : './' + relative,
        target: ctx.name(real)}
    }).sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0)
    return {...glob, names: matches.map(match => match.name), matches,
      admittedTargets: matches.filter(match => match.target === compile.testEntry)}
  })
}

/** Called only after a real successful compile; result is still only data. */
export function captureResolvedCompileCertificate({project, adapterRoot = here, built, options, testFile, entry}) {
  const ctx = context(project, adapterRoot), rootGeometry = geometry(ctx)
  assert(certifiedBuilds.has(built), 'actual certified compiler build required; warm or deserialized builds decline')
  assertCompilerEpoch(ctx)
  const issued = certifiedBuilds.get(built)
  assert(json(built.identity) === issued.identity && built.identitySha256 === issued.identitySha256 &&
    built.sha256 === issued.bundleSha256 && json(built.closure) === issued.closure, 'certified build data changed after compilation')
  assert(built.closure?.finalBundleParsed && built.sha256 === built.identity.bundleSha256 &&
    sha(built.bundle) === built.sha256, 'actual closed bundle required to seed certificate')
  const selection = currentSelection(ctx, {entry, options, testFile})
  checkedFacts(ctx, built.identity, selection)
  const sourceFiles = sourceObservations(ctx, built.identity), tools = toolIdentity(ctx)
  assert(json(tools) === issued.tools, 'actual producer tool epoch differs')
  assert(json(currentGlobs(ctx, built.identity)) === json(built.identity.globs), 'glob names changed after compile')
  assert(json(sourceObservations(ctx, built.identity)) === json(sourceFiles), 'source bytes changed during certificate capture')
  assert(json(toolIdentity(ctx)) === json(tools), 'tool bytes changed during certificate capture')
  assertCompilerEpoch(ctx)
  const certificate = {schema: SCHEMA, limits: LIMITS, selection, compile: built.identity,
    rootGeometry, tools, sourceFiles}
  const bytes = Buffer.from(json(certificate) + '\n')
  assert(bytes.length <= LIMITS.certificateBytes, 'certificate byte bound exceeded')
  return {bytes, sha256: sha(bytes)}
}

/**
 * External trusted SHA must originate from an authenticated transport, never
 * from the certificate itself. The diagnostic fixtures pass constructed seeds.
 * Returns only an identity candidate; response admission remains the ledger's.
 */
export async function validateResolvedCompileCertificates({project, adapterRoot = here, requests}) {
  const misses = reason => requests.map(() => ({hit: false, reason, next: 'full-compile'}))
  if (!Array.isArray(requests) || requests.length > LIMITS.certificates) throw new Error('bounded certificate requests required')
  try {
    assert(requests.reduce((total, request) => total + (Buffer.isBuffer(request.certificateBytes) ? request.certificateBytes.length : 0), 0)
      <= LIMITS.certificateSetBytes, 'certificate set byte bound exceeded')
    const ctx = context(project, adapterRoot), rootGeometry = geometry(ctx), tools = toolIdentity(ctx)
    if (compilerEpoch) assertCompilerEpoch(ctx)
    const results = [], candidates = [], sharedRequests = [], sharedTargets = [], sourceCache = new Map(), retainedSources = {bytes: 0}
    for (const request of requests) {
      try {
        const bytes = request.certificateBytes
        assert(Buffer.isBuffer(bytes) && bytes.length <= LIMITS.certificateBytes && digest(request.trustedCertificateSha256) &&
          sha(bytes) === request.trustedCertificateSha256, 'missing or changed authenticated certificate bytes')
        const value = JSON.parse(bytes.toString('utf8'))
        assert(Buffer.from(json(value) + '\n').equals(bytes), 'noncanonical certificate bytes')
        exact(value, ['schema', 'limits', 'selection', 'compile', 'rootGeometry', 'tools', 'sourceFiles'])
        assert(value.schema === SCHEMA && json(value.limits) === json(LIMITS), 'obsolete certificate')
        assert(json(value.rootGeometry) === json(rootGeometry) && json(value.tools) === json(tools), 'current physical roots/tool inputs differ')
        const selection = currentSelection(ctx, request)
        assert(json(value.selection) === json(selection), 'current options/selection differs')
        const facts = checkedFacts(ctx, value.compile, selection)
        assert(json(sourceObservations(ctx, value.compile, sourceCache, retainedSources)) === json(value.sourceFiles), 'current full source bytes differ')
        assert(json(currentGlobs(ctx, value.compile)) === json(value.compile.globs), 'current glob set differs')
        const index = results.length
        results.push(null); candidates.push({index, value})
        sharedRequests.push(...facts.requests); sharedTargets.push(...facts.targets)
      } catch (error) { results.push({hit: false, reason: error.message, next: 'full-compile'}) }
    }
    if (candidates.length) {
      const unique = new Map()
      for (let index = 0; index < sharedRequests.length; index++) {
        const key = json(sharedRequests[index]), prior = unique.get(key)
        assert(!prior || prior.target === sharedTargets[index], 'inconsistent resolution certificate')
        unique.set(key, {request: sharedRequests[index], target: sharedTargets[index]})
      }
      const rows = [...unique.values()], resolver = resolveInNewNode(ctx, rows.map(row => row.request), tools)
      const resolved = resolver.targets
      assert(resolved.every((target, index) => target === rows[index].target), 'current resolver target differs')
      assert(json(toolIdentity(ctx)) === json(tools), 'tool bytes changed during sampled validation')
      for (const [name, bytes] of sourceCache) assert(regular(ctx.file(name)).equals(bytes), 'source changed during sampled validation')
      for (const {value} of candidates) assert(json(currentGlobs(ctx, value.compile)) === json(value.compile.globs), 'glob changed during sampled validation')
      if (compilerEpoch) assertCompilerEpoch(ctx)
      const {targets, ...resolverEvidence} = resolver
      for (const {index, value} of candidates) {
        const identity = JSON.parse(json(value.compile)), proof = Object.freeze(Object.create(null))
        proofs.set(proof, json({identity, identitySha256: hash(identity), bundleSha256: identity.bundleSha256,
          configurationSha256: identity.configurationSha256}))
        results[index] = {hit: true, proof, identity, identitySha256: hash(identity), bundleSha256: identity.bundleSha256,
          authority: 'input equivalence only; authenticated passing receipt admission still required',
          uniqueResolverRequests: rows.length, resolver: resolverEvidence}
      }
    }
    return results
  } catch (error) { return misses(error.message) }
}

export const RESOLVED_INPUT_CERTIFICATE_SCHEMA = SCHEMA

// This private child does not import project modules or execute tests. It only
// asks the current compiler's shared resolver for each current physical target.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url) && process.argv[2] === '--resolve-child') {
  try {
    const bytes = regular(process.argv[3], LIMITS.resolverBytes), value = JSON.parse(bytes.toString('utf8'))
    assert(Buffer.from(json(value) + '\n').equals(bytes), 'noncanonical child request')
    exact(value, ['project', 'adapterRoot', 'requests', 'expectedToolsSha256'])
    const ctx = context(value.project, value.adapterRoot), before = toolIdentity(ctx)
    assert(hash(before) === value.expectedToolsSha256, 'child current tool identity differs')
    assert(toolCache(ctx, before).size === 0, 'resolver tools preloaded before snapshot')
    const {resolveManagedImports} = await import('./compiler.mjs')
    assert(json(toolIdentity(ctx)) === json(before), 'child tool bytes changed during import')
    const targets = await resolveManagedImports({project: ctx.project, requests: value.requests})
    assert(json(toolIdentity(ctx)) === json(before), 'child tool bytes changed during resolution')
    const response = Buffer.from(json({schema: 'quod-managed-resolved-import-child-v1', passed: true,
      requestSha256: sha(bytes), toolIdentitySha256: hash(before), rootGeometry: geometry(ctx),
      targets, node: process.version, execArgv: process.execArgv}) + '\n')
    assert(response.length <= LIMITS.resolverBytes, 'child response byte bound exceeded')
    process.stdout.write(response)
  } catch (error) {
    process.stderr.write(String(error.stack || error.message).slice(0, 4096) + '\n')
    process.exitCode = 1
  }
}
