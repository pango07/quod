// Finite preparation profile for the original convex-helpers twelve-commit
// window. This is not a generic Vite plugin adapter. Every decision prepares.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {pathToFileURL, fileURLToPath} from 'node:url'
import {canonicalManagedJSON} from './compiler.mjs'

const sha = value => crypto.createHash('sha256').update(value).digest('hex')
const pins = {
  'vitest.config.mts': '8b67460e3582eaf8d6a7209d5f7f8f89e8a44fcf6133a561c42f87b9dce52b79',
  'scripts/zodVersions.mjs': '19bfcf628785773af81477811d18e4721262dd5896411e7bfbf0d9bf285d0714',
  'packages/convex-helpers/vitest.config.mts': '6ebdf10bf5ffbf225b3cc3c6f47a4efc91ce7b5c068ba3031aa2438ea602d945',
  'example-ui/vite.config.mts': '54af8d06f01ed257ee4d5d5af7e08a2c7adc1022adcbaa370efcd42e2d82a953',
}

// These data files are admitted only after exact-byte review. Their complete
// hashes remain evidence, while the key uses the executable config, its actual
// resolved result and the compiler's closed module bytes. Unused dependency
// additions must not invalidate an unchanged stream workload. Preparation still
// reads package.json on every decision through the pinned zodVersions script.
const dataPins = {
  'package.json': new Set([
    '970d98ebb0fbd32fd59ea7e5310b60489d7f78fb0e31d6960c99e804a6c0e1a6',
    '6f989749970c30db06920dda63d8862573621d26140fce6ba1a93395dafe893d',
    'bc6d0845e54f80e78158f8c1843f6d2af5c729d8747ca8400f665d2517e0e749',
    '238ecccd794e5adc119a9a48d150d0f1598854a505d398645a1c02673e3c0b20',
    'bc31adf44eb03c5fabc622d0461ba9ae31bc6e5282b88133e3f60dd211c0ddb7',
    'f3a7d9519b58622ac5f803197923f47783f7539a750faa6d2681d643cea01275',
    'd860d2a7293ed42763e15a1571e3813be4e2d4c722a1e609585d185099e64603',
  ]),
  'package-lock.json': new Set([
    '1676eef6d2611918c8933903c475479f8df8aa63844780d2e3091688dda776c6',
    '9d08aca826028a4888364e6df31c45ab820816f41a195fffea063ddf56b6df0f',
    'abe6d7c318948a0de0207319dda1349a387132dc5d544476161d91643934222e',
    'ec3725fcc85ddd94e708c3fdb2771825b9db30afe77bd2b0f993ea7fe6f26ae1',
  ]),
}
const inside = (root, file) => file === root || file.startsWith(root + path.sep)

// Canonicalize only documented path fields. Replacing every occurrence of a
// checkout path could silently change an arbitrary user string or environment.
export function canonicalizeManagedConfig(serialized, project) {
  const omittedUndefined = []
  function data(value, keys = []) {
    if (value === undefined) { omittedUndefined.push(keys.join('.')); return undefined }
    if (value === null || typeof value !== 'object') {
      assert(['string', 'number', 'boolean'].includes(typeof value) || value === null,
        'non-data resolved configuration: ' + keys.join('.'))
      return value
    }
    assert(Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null || Array.isArray(value),
      'non-plain resolved configuration: ' + keys.join('.'))
    assert.equal(Object.getOwnPropertySymbols(value).length, 0)
    const output = Array.isArray(value) ? [] : Object.create(null)
    for (const key of Object.keys(value)) {
      const descriptor = Object.getOwnPropertyDescriptor(value, key)
      assert(descriptor && 'value' in descriptor, 'configuration accessor: ' + keys.concat(key).join('.'))
      const next = data(descriptor.value, keys.concat(key))
      if (next !== undefined) output[key] = next
      else assert(!Array.isArray(value), 'undefined configuration array element')
    }
    return output
  }
  const config = data(serialized)
  for (const keys of [['root'], ['coverage', 'reportsDirectory'], ['coverage', 'coverageFilesDirectory'], ['attachmentsDir']]) {
    let owner = config
    for (const key of keys.slice(0, -1)) owner = owner?.[key]
    const key = keys.at(-1), value = owner?.[key]
    if (value === undefined) continue
    assert(typeof value === 'string' && (value === project || value.startsWith(project + '/')),
      'configuration path outside project: ' + keys.join('.'))
    owner[key] = '/project' + value.slice(project.length)
  }
  // Ambient seed is deliberately replaced by the caller's declared seed.
  assert(config.sequence && typeof config.sequence === 'object')
  const encoded = canonicalManagedJSON(config)
  assert(!encoded.includes(project), 'unreviewed checkout-dependent configuration field')
  return {config, omittedUndefined: omittedUndefined.sort()}
}

function reviewProject(project) {
  const sources = [], dataSources = []
  for (const dir of ['', 'packages/convex-helpers', 'example-ui']) {
    for (const stem of ['vitest.config', 'vite.config']) {
      for (const extension of ['ts', 'mts', 'cts', 'js', 'mjs', 'cjs']) {
        const name = path.posix.join(dir, stem + '.' + extension)
        assert(!fs.existsSync(path.join(project, name)) || Object.hasOwn(pins, name),
          'unsupported alternative preparation config: ' + name)
      }
    }
  }
  for (const [name, expected] of Object.entries(pins)) {
    const actual = sha(fs.readFileSync(path.join(project, name)))
    assert.equal(actual, expected, 'unsupported preparation source: ' + name)
    sources.push({name, sha256: actual})
  }
  for (const [name, allowed] of Object.entries(dataPins)) {
    const actual = sha(fs.readFileSync(path.join(project, name)))
    assert(allowed.has(actual), 'unsupported preparation data source: ' + name)
    dataSources.push({name, sha256: actual})
  }
  return {sources, dataSources}
}
const zodAliasPattern = /^zod-4\.[0-6]$/
const zodImportPattern = /^zod(?=\/|$)/
const viteAliasPatterns = new Set([/^\/?@vite\/env/.source, /^\/?@vite\/client/.source])

// Vite joins FS_PREFIX ('/@fs/') with an absolute path, producing '/@fs'
// followed by that path's leading slash. Keep the path boundary explicit.
export function canonicalizeManagedAliasReplacement(replacement, project) {
  assert(typeof replacement === 'string' && typeof project === 'string' && project.startsWith('/'))
  for (const prefix of ['', '/@fs']) {
    const start = prefix + project
    if (replacement === start || replacement.startsWith(start + '/')) {
      replacement = prefix + '/project' + replacement.slice(start.length)
      break
    }
  }
  assert(!replacement.includes(project), 'unreviewed checkout path in alias')
  return replacement
}

function matrixAliases(p, project, zodAlias) {
  const aliases = p.viteConfig.resolve.alias
  assert(Array.isArray(aliases), 'resolved alias list required')
  let zodRules = 0
  const normalized = aliases.map(alias => {
    assert(alias && typeof alias === 'object' && typeof alias.replacement === 'string' &&
      alias.customResolver === undefined, 'custom alias resolver unsupported')
    const find = alias.find
    assert(find instanceof RegExp && find.flags === '', 'unreviewed string or flagged alias')
    if (find.source === zodImportPattern.source) {
      zodRules++
      assert(zodAlias && alias.replacement === zodAlias, 'resolved Zod alias differs from project')
    } else {
      assert(viteAliasPatterns.has(find.source), 'unreviewed resolved alias: ' + find.source)
      assert(alias.replacement.includes('/vite/dist/client/'), 'unreviewed Vite client alias target')
    }
    const replacement = canonicalizeManagedAliasReplacement(alias.replacement, project)
    return {find: {source: find.source, flags: find.flags}, replacement}
  })
  assert.equal(zodRules, zodAlias ? 1 : 0, 'resolved Zod alias multiplicity differs')
  return normalized
}

/** Stable identity of one actual TestSpecification, retaining project repeats. */
export function describeManagedSpecification(specification, project) {
  const absolute = fs.realpathSync(specification.moduleId)
  assert(inside(project, absolute), 'selected file outside project')
  const file = path.relative(project, absolute).split(path.sep).join('/')
  const name = specification.project.config.name || ''
  assert(typeof specification.pool === 'string' && typeof specification.taskId === 'string',
    'actual specification pool and task identity required')
  // Vitest5 filename-only filters produce[]; its interpretTaskModes treats
  // that identically to undefined. Nonempty line selections remain distinct.
  const testLines = Array.isArray(specification.testLines) && specification.testLines.length === 0
    ? null : specification.testLines ?? null
  const selection = {pool: specification.pool, testLines,
    testIds: specification.testIds ?? null, testTagsFilter: specification.testTagsFilter ?? null,
    testNamePattern: specification.testNamePattern ? {source: specification.testNamePattern.source,
      flags: specification.testNamePattern.flags} : null}
  const instance = {file, project: name, ...selection}
  return {...instance, absolute, taskId: specification.taskId,
    instanceId: sha(canonicalManagedJSON(instance))}
}

function prepareSpecification(specification, project, seed, reviewed, matrix = false) {
  const p = specification.project, absolute = fs.realpathSync(specification.moduleId)
  assert(inside(project, absolute), 'selected file outside project')
  const file = path.relative(project, absolute).split(path.sep).join('/')
  const zodAlias = matrix && zodAliasPattern.test(p.config.name) ? p.config.name : undefined
  assert(p.config.name === 'convex-helpers' || zodAlias, 'unreviewed project')
  assert.equal(p.config.root, path.join(project, 'packages/convex-helpers'))
  assert.equal(p.config.environment, 'edge-runtime')
  assert.equal(p.config.setupFiles.length, 0, 'test setup unsupported')
  assert(!p.config.globalSetup || p.config.globalSetup.length === 0, 'global setup unsupported')
  const providedContext = p.getProvidedContext()
  canonicalManagedJSON(providedContext)
  assert.deepEqual(providedContext, zodAlias ? {expectedZodAlias: zodAlias} : {}, 'provided context unsupported')
  const serialized = p.serializedConfig
  assert.equal(serialized.snapshotSerializers.length, 0)
  assert(!serialized.snapshotEnvironment)
  assert(!serialized.coverage.enabled, 'coverage unsupported')
  assert(!serialized.browser.enabled, 'browser execution unsupported')
  assert.equal(serialized.pool, 'forks', 'custom pools unsupported')
  assert.equal(serialized.snapshotOptions.updateSnapshot, 'none')
  assert(!serialized.serializedDefines, 'custom defines unsupported')
  assert(!serialized.inspector.enabled)
  assert(!fs.existsSync(path.join(project, path.dirname(file), '__snapshots__', path.basename(file) + '.snap')),
    'existing snapshot unsupported')
  const {config, omittedUndefined} = canonicalizeManagedConfig(serialized, project)
  config.sequence.seed = seed
  const identity = {schema: 'quod-managed-preparation-v1-research',
    policy: 'reviewed original twelve-commit configuration; preparation always executes; no setup, snapshots, coverage or provided context',
    preparerSha256: sha(fs.readFileSync(fileURLToPath(import.meta.url))), sources: reviewed.sources,
    configurationSha256: sha(canonicalManagedJSON(config)), omittedUndefined}
  if (!matrix) return {file, absolute, project: p.config.name, config, identity}
  const descriptor = describeManagedSpecification(specification, project)
  assert.equal(descriptor.pool, 'forks', 'non-test pool unsupported')
  assert(['testLines', 'testIds', 'testTagsFilter', 'testNamePattern'].every(key => descriptor[key] === null),
    'filtered TestSpecification unsupported')
  assert.equal(config.name, p.config.name, 'serialized project name differs')
  assert.equal(config.isolate, true, 'per-file isolation required')
  const aliases = matrixAliases(p, project, zodAlias)
  const context = JSON.parse(canonicalManagedJSON(providedContext))
  const matrixIdentity = {...identity, schema: 'quod-managed-matrix-preparation-v1-research',
    policy: 'reviewed finite edge projects; exact Zod alias and plain provided context; one fresh realm per actual specification',
    specification: {...descriptor, absolute: '/project/' + file}, aliases,
    zodAlias: zodAlias || null, providedContext: context}
  return {...descriptor, config, identity: matrixIdentity, aliases,
    ...(zodAlias ? {zodAlias} : {}), providedContext: context}
}

/** Resolve all actual selected specifications once; decline individual targets. */
async function prepareSuite({project: projectArg, files, seed}, matrix) {
  const started = performance.now(), project = fs.realpathSync(path.resolve(projectArg))
  assert(Number.isSafeInteger(seed) && seed >= 0 && seed <= 0xffffffff, 'declared uint32 seed required')
  assert(files === undefined || Array.isArray(files) && files.every(file => typeof file === 'string'), 'invalid selection filters')
  const reviewed = reviewProject(project), require = createRequire(path.join(project, 'package.json'))
  assert.equal(require('vitest/package.json').version, '5.0.0')
  if (matrix) {
    process.env.TEST = 'true'
    process.env.VITEST = 'true'
    process.env.NODE_ENV ??= 'test'
  }
  const {createVitest} = await import(pathToFileURL(require.resolve('vitest/node')).href)
  const ctx = await createVitest({root: project, watch: false, run: true, reporters: []})
  try {
    const specs = await ctx.globTestSpecifications(files?.map(file => path.resolve(project, file)))
    const selected = specs.map(specification => matrix ? describeManagedSpecification(specification, project)
      : {file: path.relative(project, specification.moduleId).split(path.sep).join('/'),
        absolute: specification.moduleId, project: specification.project.config.name})
    if (matrix) assert.equal(new Set(selected.map(row => row.instanceId)).size, selected.length,
      'duplicate actual specification identity')
    const prepared = [], declined = []
    for (let index = 0; index < specs.length; index++) {
      try { prepared.push(prepareSpecification(specs[index], project, seed, reviewed, matrix)) }
      catch (error) { declined.push({...selected[index], reason: error.message}) }
    }
    const evidence = {profile: matrix ? 'convex-helpers-original-twelve-matrix' : 'convex-helpers-original-twelve',
      projects: ctx.projects.map(p => ({name: p.config.name, root: p.config.root})), selected,
      reviewedSources: reviewed.sources, reviewedDataSources: reviewed.dataSources,
      keyedInputs: 'executable configuration source bytes, actual selected resolved configuration, closed compiled module bytes; package and lock full bytes are finite admission evidence',
      seconds: (performance.now() - started) / 1000,
      ...(matrix ? {prepared: prepared.map(row => ({instanceId: row.instanceId, file: row.file,
        project: row.project, identity: row.identity})), declined} : {})}
    return {project, selected, prepared, declined, evidence}
  } finally { await ctx.close() }
}

export function prepareManagedSuite(options) { return prepareSuite(options, false) }

/** Separate investigation profile; the existing single-file product is unchanged. */
export function prepareManagedMatrix(options) { return prepareSuite(options, true) }

/** Preserve the research probe's single-spec interface. */
export async function prepareManagedProject({project, file, seed}) {
  const suite = await prepareManagedSuite({project, files: [file], seed})
  assert.equal(suite.selected.length, 1, 'one actual Vitest test specification required')
  assert.equal(suite.prepared.length, 1, suite.declined[0]?.reason || 'selected specification unsupported')
  const {config, identity} = suite.prepared[0]
  return {config, identity, evidence: {...suite.evidence, identity}}
}
