// Deliberately bounded research policy for the unchanged c11 workload. This is
// not a generic Vite plugin adapter. Preparation executes on every decision.
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
  'package.json': 'd860d2a7293ed42763e15a1571e3813be4e2d4c722a1e609585d185099e64603',
  'package-lock.json': 'ec3725fcc85ddd94e708c3fdb2771825b9db30afe77bd2b0f993ea7fe6f26ae1',
}

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

export async function prepareManagedProject({project, file, seed}) {
  const started = performance.now(), sources = []
  for (const [name, expected] of Object.entries(pins)) {
    const actual = sha(fs.readFileSync(path.join(project, name)))
    assert.equal(actual, expected, 'unsupported preparation source: ' + name)
    sources.push({name, sha256: actual})
  }
  const require = createRequire(path.join(project, 'package.json'))
  assert.equal(require('vitest/package.json').version, '5.0.0')
  const {createVitest} = await import(pathToFileURL(require.resolve('vitest/node')).href)
  const ctx = await createVitest({root: project, watch: false, run: true, reporters: []})
  let config, evidence, omittedUndefined
  try {
    const specs = await ctx.globTestSpecifications([path.join(project, file)])
    evidence = {projects: ctx.projects.map(p => ({name: p.config.name, root: p.config.root})),
      selected: specs.map(s => ({file: s.moduleId, project: s.project.config.name}))}
    assert.equal(specs.length, 1, 'one actual Vitest test specification required')
    const p = specs[0].project
    assert.equal(p.config.name, 'convex-helpers')
    assert.equal(p.config.root, path.join(project, 'packages/convex-helpers'))
    assert.equal(p.config.environment, 'edge-runtime')
    assert.equal(p.config.setupFiles.length, 0, 'test setup unsupported')
    assert(!p.config.globalSetup || p.config.globalSetup.length === 0, 'global setup unsupported')
    assert.deepEqual(Object.keys(p.getProvidedContext()), [], 'provided context unsupported')
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
    ;({config, omittedUndefined} = canonicalizeManagedConfig(serialized, project))
    config.sequence.seed = seed
  } finally { await ctx.close() }
  const identity = {schema: 'quod-managed-preparation-v1-research',
    policy: 'pinned c11 configuration; preparation always executes; no setup, snapshots, coverage or provided context',
    preparerSha256: sha(fs.readFileSync(fileURLToPath(import.meta.url))), sources,
    configurationSha256: sha(canonicalManagedJSON(config)), omittedUndefined}
  return {config, identity, evidence: {...evidence, identity, seconds: (performance.now() - started) / 1000}}
}
