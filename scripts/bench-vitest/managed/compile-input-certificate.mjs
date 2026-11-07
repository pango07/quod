// Diagnostic only. A certificate authenticates no response and starts no worker.
// Native preparation remains mandatory. Invalid certificates mean full compile.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {fileURLToPath} from 'node:url'
import {canonicalManagedJSON, resolveManagedImports} from './compiler.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const SCHEMA = 'quod-managed-compile-input-certificate-v1-diagnostic'
const LIMITS = {paths: 100000, metadataBytes: 128 * 1024 * 1024,
  toolBytes: 128 * 1024 * 1024, sourceBytes: 256 * 1024 * 1024,
  fileBytes: 16 * 1024 * 1024, certificateBytes: 16 * 1024 * 1024,
  certificateSetBytes: 128 * 1024 * 1024, certificates: 512}
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const json = canonicalManagedJSON
const hash = value => sha(json(value))
const digest = value => typeof value === 'string' && /^[0-9a-f]{64}$/.test(value)
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

// Positive sources are read in full; timestamps/inodes only sample read races.
// The namespace retains every path/kind/permission and symlink target, but only
// JSON bytes: all possible package and tsconfig/extends JSON inputs are included.
// .git is excluded explicitly; imports from it decline and glob sets rerun.
function namespace(ctx) {
  const rows = [], ancestors = [], counts = {paths: 0, metadataBytes: 0}
  function walk(file, label) {
    assert(++counts.paths <= LIMITS.paths, 'namespace path bound exceeded')
    const st = fs.lstatSync(file), row = {name: label, mode: st.mode & 0o7777}
    if (st.isDirectory()) {
      row.kind = 'directory'; rows.push(row)
      for (const child of fs.readdirSync(file).sort()) {
        if (child === '.git') continue
        walk(path.join(file, child), label + '/' + child)
      }
    } else if (st.isFile()) {
      row.kind = 'file'
      if (file.endsWith('.json')) {
        const bytes = regular(file)
        counts.metadataBytes += bytes.length
        assert(counts.metadataBytes <= LIMITS.metadataBytes, 'namespace metadata bound exceeded')
        row.bytes = bytes.length; row.sha256 = sha(bytes)
      }
      rows.push(row)
    } else if (st.isSymbolicLink()) {
      row.kind = 'symlink'; row.target = fs.readlinkSync(file)
      try { row.real = ctx.name(fs.realpathSync(file)) }
      catch (error) { assert(error.code === 'ENOENT', 'external or cyclic namespace symlink unsupported'); row.real = null }
      rows.push(row)
    } else throw new Error('special namespace object unsupported')
  }
  for (const [prefix, root] of ctx.roots) walk(root, prefix)
  // Package/config discovery above both roots must also be closed. An external
  // ancestor node_modules or NODE_PATH would add a namespace we have not hashed.
  const parents = new Set()
  for (const [, root] of ctx.roots) for (let parent = path.dirname(root);; parent = path.dirname(parent)) {
    if (!ctx.roots.some(([, candidate]) => inside(candidate, parent))) parents.add(parent)
    if (parent === path.dirname(parent)) break
  }
  for (const parent of [...parents].sort()) {
    const names = fs.readdirSync(parent).filter(name => name === 'package.json' || name.endsWith('.json') && /^(?:ts|js)config/.test(name)).sort()
    const nodeModules = path.join(parent, 'node_modules')
    assert(!fs.existsSync(nodeModules), 'external ancestor node_modules unsupported')
    const configs = names.map(name => {
      const file = path.join(parent, name), bytes = regular(file)
      counts.metadataBytes += bytes.length
      assert(counts.metadataBytes <= LIMITS.metadataBytes, 'ancestor metadata bound exceeded')
      // Extends outside the inventoried roots has no finite proof in v1.
      assert(!/(?:"|')extends(?:"|')\s*:/.test(bytes.toString('utf8')), 'external ancestor config extends unsupported')
      return {name, bytes: bytes.length, sha256: sha(bytes)}
    })
    ancestors.push({path: parent, nodeModules: 'absent', configs})
  }
  return {rows, ancestors, counts}
}

function checkSourceConfigs(ctx, compile) {
  // The compiler may follow a tsconfig extends chain to an arbitrarily named
  // JSON file. Every in-root JSON byte is above; reject any chain escaping it.
  const ts = createRequire(path.join(ctx.project, 'package.json'))('typescript')
  const configFiles = []
  for (const source of compile.sources) {
    for (let dir = path.dirname(ctx.file(source.source));; dir = path.dirname(dir)) {
      for (const name of ['tsconfig.json', 'jsconfig.json']) {
        const file = path.join(dir, name)
        if (fs.existsSync(file)) { ctx.name(fs.realpathSync(file)); configFiles.push(file) }
      }
      if (ctx.roots.some(([, root]) => dir === root)) break
      assert(dir !== path.dirname(dir), 'source configuration escaped inventory roots')
    }
  }
  const seenConfigs = new Set()
  for (let index = 0; index < configFiles.length; index++) {
    const file = configFiles[index]
    if (seenConfigs.has(file)) continue
    seenConfigs.add(file)
    const parsed = ts.parseConfigFileTextToJson(file, regular(file).toString('utf8'))
    assert(!parsed.error, 'unparseable compiler configuration')
    const extensions = parsed.config?.extends === undefined ? []
      : Array.isArray(parsed.config.extends) ? parsed.config.extends : [parsed.config.extends]
    for (const extension of extensions) {
      assert(typeof extension === 'string', 'non-string config extends unsupported')
      let target
      if (extension.startsWith('.') || path.isAbsolute(extension)) {
        target = path.resolve(path.dirname(file), extension)
        if (!target.endsWith('.json')) target += '.json'
      } else target = createRequire(file).resolve(extension)
      target = fs.realpathSync(target)
      assert(target.endsWith('.json'), 'non-JSON config extends unsupported')
      ctx.name(target); configFiles.push(target)
    }
  }
}

function toolIdentity(ctx) {
  assert(process.platform === 'linux' && process.arch === 'x64', 'finite Linux x64 compiler profile required')
  assert.equal(process.execArgv.length, 0, 'unreviewed compiler Node CLI flags')
  for (const key of ['NODE_PATH', 'NODE_OPTIONS', 'ESBUILD_BINARY_PATH']) assert(!process.env[key], 'unreviewed compiler environment: ' + key)
  const own = createRequire(import.meta.url), project = createRequire(path.join(ctx.project, 'package.json'))
  const packages = [own.resolve('esbuild/package.json'), own.resolve('@esbuild/linux-x64/package.json'), project.resolve('typescript/package.json')]
  const files = [], counts = {bytes: 0}
  function walk(file, label) {
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
  const hostSources = ['compiler.mjs', 'compile-input-certificate.mjs'].map(name => ({name,
    sha256: sha(regular(path.join(here, name)))}))
  return {platform: process.platform, arch: process.arch, node: process.version,
    nodeSha256: sha(regular(fs.realpathSync(process.execPath), LIMITS.toolBytes)),
    environment: {NODE_ENV: process.env.NODE_ENV ?? null, ESBUILD_WORKER_THREADS: process.env.ESBUILD_WORKER_THREADS ?? null},
    files, hostSources, counts}
}

function currentSelection(ctx, {entry = 'runner.mjs', testFile, options}) {
  assert(entry === 'runner.mjs', 'runner-only certificate profile')
  return {entry: ctx.name(fs.realpathSync(path.join(ctx.adapter, entry))),
    testEntry: ctx.name(fs.realpathSync(testFile)), configurationSha256: hash(options)}
}

function sourceObservations(ctx, compile, cache = new Map(), retained = {bytes: 0}) {
  const names = [...new Set(compile.sources.map(row => row.source))].sort(), rows = []
  let total = 0
  for (const name of names) {
    let bytes = cache.get(name)
    if (!bytes) {
      bytes = regular(ctx.file(name)); retained.bytes += bytes.length
      assert(retained.bytes <= LIMITS.sourceBytes, 'shared source byte bound exceeded')
      cache.set(name, bytes)
    }
    total += bytes.length
    assert(total <= LIMITS.sourceBytes, 'source byte bound exceeded')
    const source = compile.sources.filter(row => row.source === name)
    assert(source.every(row => row.sourceSha256 === sha(bytes.toString('utf8'))), 'loaded source bytes changed')
    rows.push({name, bytes: bytes.length, rawSha256: sha(bytes)})
  }
  return rows
}

function checkedFacts(ctx, compile, selection) {
  assert(compile.schema === 'quod-managed-compile-v1-research' && Array.isArray(compile.resolutionFacts) && compile.resolutionFacts.length,
    'complete original/emitted resolution facts required')
  assert(compile.compilerSha256 === sha(regular(path.join(here, 'compiler.mjs'))), 'compiler bytes differ')
  for (const key of ['entry', 'testEntry', 'configurationSha256']) assert(compile[key] === selection[key], 'current selection/config differs')
  checkSourceConfigs(ctx, compile)
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
export function captureManagedCompileCertificate({project, adapterRoot = here, built, options, testFile, entry}) {
  const ctx = context(project, adapterRoot), before = namespace(ctx)
  assert(built.closure?.finalBundleParsed && built.sha256 === built.identity.bundleSha256 &&
    sha(built.bundle) === built.sha256, 'actual closed bundle required to seed certificate')
  const selection = currentSelection(ctx, {entry, options, testFile})
  checkedFacts(ctx, built.identity, selection)
  const sourceFiles = sourceObservations(ctx, built.identity), tools = toolIdentity(ctx)
  assert(json(currentGlobs(ctx, built.identity)) === json(built.identity.globs), 'glob names changed after compile')
  assert(json(namespace(ctx)) === json(before), 'namespace changed during certificate capture')
  const certificate = {schema: SCHEMA, limits: LIMITS, selection, compile: built.identity,
    namespace: before, tools, sourceFiles}
  const bytes = Buffer.from(json(certificate) + '\n')
  assert(bytes.length <= LIMITS.certificateBytes, 'certificate byte bound exceeded')
  return {bytes, sha256: sha(bytes)}
}

/**
 * External trusted SHA must originate from an authenticated transport, never
 * from the certificate itself. The diagnostic fixtures pass constructed seeds.
 * Returns only an identity candidate; response admission remains the ledger's.
 */
export async function validateManagedCompileCertificates({project, adapterRoot = here, requests}) {
  const misses = reason => requests.map(() => ({hit: false, reason, next: 'full-compile'}))
  if (!Array.isArray(requests) || requests.length > LIMITS.certificates) throw new Error('bounded certificate requests required')
  try {
    assert(requests.reduce((total, request) => total + (Buffer.isBuffer(request.certificateBytes) ? request.certificateBytes.length : 0), 0)
      <= LIMITS.certificateSetBytes, 'certificate set byte bound exceeded')
    const ctx = context(project, adapterRoot), before = namespace(ctx), tools = toolIdentity(ctx)
    const results = [], candidates = [], sharedRequests = [], sharedTargets = [], sourceCache = new Map(), retainedSources = {bytes: 0}
    for (const request of requests) {
      try {
        const bytes = request.certificateBytes
        assert(Buffer.isBuffer(bytes) && bytes.length <= LIMITS.certificateBytes && digest(request.trustedCertificateSha256) &&
          sha(bytes) === request.trustedCertificateSha256, 'missing or changed authenticated certificate bytes')
        const value = JSON.parse(bytes.toString('utf8'))
        assert(Buffer.from(json(value) + '\n').equals(bytes), 'noncanonical certificate bytes')
        exact(value, ['schema', 'limits', 'selection', 'compile', 'namespace', 'tools', 'sourceFiles'])
        assert(value.schema === SCHEMA && json(value.limits) === json(LIMITS), 'obsolete certificate')
        assert(json(value.namespace) === json(before) && json(value.tools) === json(tools), 'current namespace/tool inputs differ')
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
      const rows = [...unique.values()], resolved = await resolveManagedImports({project: ctx.project, requests: rows.map(row => row.request)})
      assert(resolved.every((target, index) => target === rows[index].target), 'current resolver target differs')
      assert(json(toolIdentity(ctx)) === json(tools), 'tool bytes changed during sampled validation')
      for (const [name, bytes] of sourceCache) assert(regular(ctx.file(name)).equals(bytes), 'source changed during sampled validation')
      assert(json(namespace(ctx)) === json(before), 'namespace changed during sampled validation')
      for (const {index, value} of candidates) results[index] = {hit: true,
        identity: value.compile, identitySha256: hash(value.compile), bundleSha256: value.compile.bundleSha256,
        authority: 'input equivalence only; authenticated passing receipt admission still required',
        uniqueResolverRequests: rows.length, namespacePaths: before.counts.paths}
    }
    return results
  } catch (error) { return misses(error.message) }
}

export const COMPILE_INPUT_CERTIFICATE_SCHEMA = SCHEMA
