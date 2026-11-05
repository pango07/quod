// Measurement only: this does not authorize replay or alter Quod's keys.
// It varies manifest declarations with one fixed installed dependency tree
// to isolate their effect on configuration. Equal hashes are a lead, not
// proof: plugin closures, transforms and setup effects remain unvalidated.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { createRequire } from 'node:module'
import { pathToFileURL } from 'node:url'
import { spawnSync } from 'node:child_process'
import { performance } from 'node:perf_hooks'

const [projectArg, outArg, mode] = process.argv.slice(2)
const project = path.resolve(projectArg)
const out = path.resolve(outArg)
const pkgPath = path.join(project, 'package.json')
const original = fs.readFileSync(pkgPath, 'utf8')
const hash = value => crypto.createHash('sha256').update(JSON.stringify(value)).digest('hex')

// Retain scalar distinctions and non-JSON values in diagnostics. Functions
// are explicitly opaque: source text cannot describe their closed-over data.
function canonical(value, seen = new Map(), location = '$') {
  if (value === undefined) return ['undefined']
  if (typeof value === 'function') return ['OPAQUE_FUNCTION', value.name]
  if (typeof value === 'number' && !Number.isFinite(value)) return ['number', String(value)]
  if (typeof value === 'bigint') return ['bigint', String(value)]
  if (value === null || typeof value !== 'object') return value
  if (seen.has(value)) return ['reference', seen.get(value)]
  seen.set(value, location)
  if (value instanceof RegExp) return ['regexp', value.source, value.flags]
  if (value instanceof URL) return ['url', value.href]
  if (value instanceof Date) return ['date', value.toISOString()]
  if (Array.isArray(value)) return value.map((v, i) => canonical(v, seen, `${location}[${i}]`))
  if (value instanceof Map) return ['map', [...value].map(([k, v], i) => [canonical(k, seen, `${location}.k${i}`), canonical(v, seen, `${location}.v${i}`)])]
  if (value instanceof Set) return ['set', [...value].map((v, i) => canonical(v, seen, `${location}.s${i}`))]
  const result = {}
  for (const k of Object.keys(value).sort()) result[k] = canonical(value[k], seen, `${location}.${k}`)
  return result
}

if (mode) {
  process.chdir(project)
  process.env.TEST = 'true'
  process.env.VITEST = 'true'
  process.env.NODE_ENV ??= 'test'
  const req = createRequire(pkgPath)
  const vitestPkgPath = req.resolve('vitest/package.json')
  const vp = JSON.parse(fs.readFileSync(vitestPkgPath, 'utf8'))
  const entry = vp.exports['./node']
  const rel = typeof entry === 'string' ? entry : entry.import ?? entry.default
  const V = await import(pathToFileURL(path.join(path.dirname(vitestPkgPath), rel)).href)
  const start = performance.now()
  // Hold Vitest's default timestamp-derived sequence seed constant only in
  // this diagnostic, so independent process starts are comparable.
  const probeSeed = 12345
  const ctx = await V.createVitest('test', { run: true, watch: false, sequence: { seed: probeSeed } }, {}, {})
  try {
    const configured = performance.now()
    const specs = await ctx.specifications.getRelevantTestSpecifications([])
    const projects = ctx.projects.map(p => ({
      name: p.name,
      workerConfig: canonical(p.serializedConfig),
      context: typeof p.getProvidedContext === 'function' ? canonical(p.getProvidedContext()) : ['UNKNOWN_CONTEXT'],
      plugins: (p.vite?.config?.plugins ?? []).map(plugin => ({
        name: plugin.name,
        opaqueHooks: Object.keys(plugin).filter(k => typeof plugin[k] === 'function' || typeof plugin[k]?.handler === 'function').sort(),
      })),
    })).sort((a,b) => a.name.localeCompare(b.name))
    const files = specs.map(s => [s.project.name, path.relative(project, s.moduleId), s.pool]).sort((a,b) => JSON.stringify(a).localeCompare(JSON.stringify(b)))
    if (!files.length || !projects.length) throw Error('missing resolved projects or test selection')
    const record = { mode, probeSeed, vitest: vp.version, configure_ms: configured-start, selection_ms: performance.now()-configured, projects, files,
      configHash: hash(projects.map(({name, workerConfig, context}) => ({name, workerConfig, context}))), filesHash: hash(files),
      replayEligible: false, unvalidated: ['plugin closures and outputs', 'transformed modules and resolution', 'global setup effects', 'runtime observations'] }
    fs.mkdirSync(out, { recursive: true })
    fs.writeFileSync(path.join(out, mode+'.json'), JSON.stringify(record, null, 2))
  } finally { await ctx.close() }
} else {
  const cases = [
    ['before-both', { chalk: '4.1.2', commander: '10.0.1' }],
    ['after-chalk', { chalk: '6.0.1', commander: '10.0.1' }],
    ['after-both', null],
  ]
  try {
    for (const [name, versions] of cases) {
      const pkg = JSON.parse(original)
      if (versions) Object.assign(pkg.devDependencies, versions)
      fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2)+'\n')
      const result = spawnSync(process.execPath, [import.meta.filename, project, out, name], { stdio: 'inherit', env: process.env })
      if (result.status !== 0) throw Error(`configuration probe ${name} failed: ${result.status}`)
    }
  } finally { fs.writeFileSync(pkgPath, original) }
  const records = cases.map(([name]) => JSON.parse(fs.readFileSync(path.join(out, name+'.json'), 'utf8')))
  console.log(JSON.stringify(records.map(r => ({ mode:r.mode, configure_ms:r.configure_ms, selection_ms:r.selection_ms, projects:r.projects.length, files:r.files.length, configHash:r.configHash, filesHash:r.filesHash, replayEligible:r.replayEligible })), null, 2))
  console.log('Equal outputs are diagnostic evidence only; this probe never permits replay.')
}
