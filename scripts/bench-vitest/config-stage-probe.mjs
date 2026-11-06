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
const resolverProbe = process.env.QUOD_CONFIG_RESOLVER_PROBE === '1'
const booleanProbe = process.env.QUOD_CONFIG_BOOLEAN_PROBE === '1'
const queryProbe = process.env.QUOD_CONFIG_QUERY_PROBE === '1'
const querySites = process.env.QUOD_CONFIG_QUERY_SITES === '1'
const asyncSites = process.env.QUOD_CONFIG_ASYNC_SITES === '1'
// Runtime metadata diagnostics use the current checkout, execute the full
// suite, and observe only the main process. They never produce a ledger.
const runtimeSites = process.env.QUOD_CONFIG_RUNTIME_SITES === '1'
const asyncStatSites = process.env.QUOD_CONFIG_ASYNC_STAT_SITES === '1'
const runSuite = booleanProbe && (!querySites || runtimeSites)
const workspacePkg = path.join(project, 'packages/convex-helpers/package.json')
const workspaceOriginal = fs.existsSync(workspacePkg) ? fs.readFileSync(workspacePkg, 'utf8') : null
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
  let recorder = null, namesProof = null, settledKeys = null, booleanProof = null, statProof = null
  const booleanReads = new Map()
  const readSites = new Map()
  const metadataSites = new Map()
  let asyncReads = null
  const asyncStats = asyncStatSites ? (await import('./async-stat-probe.mjs')).observeAsyncStat(project) : null
  if (process.env.QUOD_CONFIG_NAMES_PROOF === '1' || resolverProbe) {
    let recorderUrl = new URL('../../crates/quod-cli/src/vitest/record.mjs', import.meta.url)
    if (resolverProbe && !queryProbe) {
      const source = fs.readFileSync(recorderUrl, 'utf8')
      const guard = source.includes("if (p == null || !p.endsWith('/package.json') || resolverCaller()) return")
        ? "if (p == null || !p.endsWith('/package.json') || resolverCaller()) return"
        : "if (resolverCaller() && (!r.booleanProof || !r.pjReads.has(p))) return"
      if (!source.includes(guard)) throw Error('resolver diagnostic guard changed')
      fs.mkdirSync(out, {recursive:true})
      recorderUrl = pathToFileURL(path.join(out, 'resolver-recorder.mjs'))
      // Observe additional field reads; the broad pj: keys are never removed.
      // The probe does not run or replay tests, and produces no ledger.
      let diagnostic = source.replace(guard, "if (p == null || !p.endsWith('/package.json')) return")
      if (booleanProbe) {
        const getter = 'const get = () => {'
        if (!diagnostic.includes(getter)) throw Error('field diagnostic getter changed')
        diagnostic = diagnostic.replace(getter, getter + '\n        r.observeBoolean?.(p, next)')
      }
      fs.writeFileSync(recorderUrl, diagnostic)
    }
    if (asyncSites) {
      const { instrumentAsyncReads } = await import('./async-read-probe.mjs')
      const source = fs.readFileSync(recorderUrl, 'utf8')
      fs.mkdirSync(out, {recursive:true})
      recorderUrl = pathToFileURL(path.join(out, 'async-observer-recorder.mjs'))
      fs.writeFileSync(recorderUrl, instrumentAsyncReads(source))
    }
    const R = await import(recorderUrl.href)
    if (!resolverProbe || queryProbe) namesProof = (await import('../../crates/quod-cli/src/vitest/names-proof.mjs')).createNamesProof()
    if (queryProbe) statProof = (await import('../../crates/quod-cli/src/vitest/boolean-proof.mjs')).createBooleanProof({internalFiles:[recorderUrl.href,...(asyncStatSites?[new URL('./async-stat-probe.mjs',import.meta.url).href]:[])]})
    asyncStats?.setConsumerProof(statProof)
    // Match the names-only policy in the real-job metadata probe.
    booleanProof = runtimeSites ? null : statProof
    recorder = R.createRecorder({ stateDir:out, mode:'main', namesProof, booleanProof, statProof })
    if (asyncSites) asyncReads = (await import('./async-read-probe.mjs')).observeAsyncReads(recorder, project, [pkgPath, workspacePkg], [recorderUrl.href])
    if (querySites) recorder.observeRead = (kind, p) => {
      if (kind === 'sm' || kind === 'sl') {
        const stack=new Error().stack.split('\n').slice(2,10).join('\n')
        const key=JSON.stringify([kind,path.relative(project,p),stack])
        metadataSites.set(key,(metadataSites.get(key) ?? 0)+1)
      }
      if (p !== pkgPath && p !== workspacePkg) return
      const stack=new Error().stack.split('\n').slice(2,9).join('\n')
      const key=JSON.stringify([kind,path.relative(project,p),stack])
      readSites.set(key,(readSites.get(key) ?? 0)+1)
    }
    if (booleanProbe && !queryProbe) {
      const {createBooleanProof} = await import('../../crates/quod-cli/src/vitest/boolean-proof.mjs')
      booleanProof = createBooleanProof({internalFiles:[recorderUrl.href, import.meta.url]})
      recorder.observeBoolean = (p, field) => {
        if (field.length !== 1 || field[0] !== 'exports') return
        let counts = booleanReads.get(p)
        if (!counts) booleanReads.set(p, counts = {boolean:0, whole:0, wholeSites:new Set()})
        const proved = booleanProof.test('exports')
        counts[proved ? 'boolean' : 'whole']++
        if (!proved && counts.wholeSites.size < 20) counts.wholeSites.add(new Error().stack.split('\n').slice(2,6).join('\n'))
      }
    }
    settledKeys = R.settledKeys
    R.patchFs(recorder)
  }
  // Match the product driver: hook fs before importing Vitest/Vite. Vite
  // captures fs.promises.readFile for its async configuration search during
  // module initialization; hooking later silently misses that observation.
  const V = await import(pathToFileURL(path.join(path.dirname(vitestPkgPath), rel)).href)
  const start = performance.now()
  // Hold Vitest's default timestamp-derived sequence seed constant only in
  // this diagnostic, so independent process starts are comparable.
  const probeSeed = 12345
  const ctx = await V.createVitest('test', { run: true, watch: false, sequence: { seed: probeSeed },
    ...(booleanProbe ? {reporters:['json'], outputFile:path.join(out, mode+'-tests.json')} : {}),
  }, {}, {})
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
    const selected = performance.now()
    if (runSuite) {
      // The complete suite, so deferred resolver/cache consumers also read
      // the watched objects. No test file is ever replayed in this probe.
      await ctx.start([])
      if (process.exitCode) throw Error('boolean diagnostic test run failed')
    }
    const proof = recorder ? {
      stats: namesProof?.stats,
      manifestKeys: [...settledKeys(recorder)].filter(k => [pkgPath,workspacePkg].some(p=>k.endsWith(`:${p}`) || k.includes(`:${p}\0`))),
      metadataKeys: querySites ? [...settledKeys(recorder)].filter(k => /^(sm|sl|st):/.test(k)).sort() : undefined,
      metadataSites: querySites ? [...metadataSites].map(([key,count])=>({site:JSON.parse(key),count})) : undefined,
      readCounts: queryProbe ? [pkgPath,workspacePkg].map(p=>({path:path.relative(project,p),reads:recorder.pjReads.get(p),parsed:recorder.pjParsed.get(p)})) : undefined,
      readSites: querySites ? [...readSites].map(([key,count])=>({site:JSON.parse(key),count})) : undefined,
      resolverReads: resolverProbe ? [...recorder.pjUse].filter(([p]) => !p.includes('/node_modules/'))
        .map(([p, fields]) => ({path:path.relative(project, p), fields:[...fields].sort().map(s => JSON.parse(s)), broadKeyRetained:recorder.keys.has(`pj:${p}`)})) : undefined,
      booleanReads: booleanProof ? [...booleanReads].map(([p, reads]) => ({path:path.relative(project,p), ...reads, wholeSites:[...reads.wholeSites]})) : undefined,
      booleanStats: booleanProof?.stats,
      statStats: statProof?.stats,
      asyncReads: asyncReads?.report(),
      asyncStats: asyncStats?.report(),
    } : null
    const record = { mode, probeSeed, vitest: vp.version, runtimeSites, observationScope:'main process only', configure_ms: configured-start, selection_ms: selected-configured, run_ms:performance.now()-selected, projects, files, proof,
      configHash: hash(projects.map(({name, workerConfig, context}) => ({name, workerConfig, context}))), filesHash: hash(files),
      replayEligible: false, unvalidated: ['plugin closures and outputs', 'transformed modules and resolution', 'global setup effects', 'runtime observations'] }
    fs.mkdirSync(out, { recursive: true })
    fs.writeFileSync(path.join(out, mode+'.json'), JSON.stringify(record, null, 2))
  } finally { await ctx.close(); namesProof?.close(); booleanProof?.close(); if(statProof!==booleanProof)statProof?.close(); asyncReads?.close(); asyncStats?.close() }
} else {
  let cases = resolverProbe ? [
    ['c3-before-exports', '03ffe84ef'], ['c4-exports', '6b17e1514'], ['c5-workspace-removed', '49a82e544'],
  ] : [
    ['before-both', { chalk: '4.1.2', commander: '10.0.1' }],
    ['after-chalk', { chalk: '6.0.1', commander: '10.0.1' }],
    ['after-both', null],
  ]
  if (querySites) cases=cases.slice(0,1)
  if (runtimeSites) cases=[['current-runtime', null]]
  try {
    for (const [name, versions] of cases) {
      if (runtimeSites) {
        // Keep the exact current manifests, including an absent workspace
        // manifest; this is not a synthetic historical manifest variant.
      } else if (resolverProbe) {
        const manifest = spawnSync('git', ['show', `${versions}:package.json`], {cwd:project, encoding:'utf8'})
        if (manifest.status !== 0) throw Error(`missing historical manifest: ${versions}`)
        fs.writeFileSync(pkgPath, manifest.stdout)
        const ws = spawnSync('git', ['show', `${versions}:packages/convex-helpers/package.json`], {cwd:project, encoding:'utf8'})
        if (ws.status === 0) fs.writeFileSync(workspacePkg, ws.stdout)
        else if (name === 'c5-workspace-removed') fs.rmSync(workspacePkg, {force:true})
        else throw Error(`missing expected workspace manifest: ${versions}`)
      } else {
        const pkg = JSON.parse(original)
        if (versions) Object.assign(pkg.devDependencies, versions)
        fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2)+'\n')
      }
      const result = spawnSync(process.execPath, [import.meta.filename, project, out, name], { stdio: 'inherit', env: process.env })
      if (result.status !== 0) throw Error(`configuration probe ${name} failed: ${result.status}`)
    }
  } finally {
    if (!runtimeSites) fs.writeFileSync(pkgPath, original)
    if (resolverProbe && !runtimeSites) {
      if (workspaceOriginal === null) fs.rmSync(workspacePkg, {force:true})
      else fs.writeFileSync(workspacePkg, workspaceOriginal)
    }
  }
  const records = cases.map(([name]) => JSON.parse(fs.readFileSync(path.join(out, name+'.json'), 'utf8')))
  console.log(JSON.stringify(records.map(r => ({ mode:r.mode, configure_ms:r.configure_ms, selection_ms:r.selection_ms, projects:r.projects.length, files:r.files.length, configHash:r.configHash, filesHash:r.filesHash, replayEligible:r.replayEligible, proof:r.proof })), null, 2))
  if (process.env.QUOD_CONFIG_NAMES_PROOF === '1') {
    for (const record of records) {
      if (!record.proof?.stats.ignored) throw Error(`names proof did not recognize the actual loaded configuration: ${record.mode}`)
      if (record.proof.manifestKeys.some(k => k.startsWith('f:'))) throw Error(`manifest still has a whole-file read: ${record.mode}`)
      if (record.proof.manifestKeys.some(k => JSON.parse(k.split('\0')[1]).some(p => p.length > 1))) throw Error(`copied version values remain inputs: ${record.mode}`)
    }
  }
  if (runSuite) {
    const states = cases.map(([name]) => {
      const report = JSON.parse(fs.readFileSync(path.join(out, name+'-tests.json'), 'utf8'))
      if (!report.success || !report.testResults?.length) throw Error(`incomplete test evidence: ${name}`)
      return report.testResults.map(f => [f.name, f.assertionResults.map(a => [a.fullName, a.status]).sort()]).sort()
    })
    if (states.some(s => JSON.stringify(s) !== JSON.stringify(states[0]))) throw Error('manifest diagnostic per-test states differed')
    console.log(`Full-suite diagnostic: ${states[0].length} files checked across ${cases.length} manifest states; no replay authorized.`)
  }
  if (queryProbe && !querySites) {
    for (const record of records) {
      if (!record.proof?.booleanStats?.statType || !record.proof?.booleanStats?.boolean) throw Error(`actual resolver proof was not exercised: ${record.mode}`)
      const keys=record.proof.manifestKeys
      if (keys.some(k=>(k.startsWith('f:') || k.startsWith('pj:')) && !(record.mode==='c5-workspace-removed' && k.endsWith(':'+workspacePkg)))) throw Error(`manifest has an unrefined observation: ${record.mode}`)
      if (!keys.some(k=>k.startsWith(`pjq:${pkgPath}\0`) && JSON.parse(k.split('\0')[1]).truthy.includes('exports'))) throw Error(`root manifest lacks truthiness query: ${record.mode}`)
    }
  }
  console.log('Equal outputs are diagnostic evidence only; this probe never permits replay.')
}
