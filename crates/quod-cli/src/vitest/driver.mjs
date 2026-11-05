// quod vitest: the driver. `quod vitest` runs it as vitest's main process:
//
//   node driver.mjs <plan.json> <out.json> <vitest args...>
//
// It records what the main process reads (config, plugins, the tools
// themselves: inputs of every test file), starts vitest through its Node
// API, and inside vitest:
//   - lets vitest find the test files (its own include/exclude/projects),
//     then runs only those the plan does not replay, plus the audits;
//   - before the run's end, reports each replayed file's recorded result
//     to vitest's reporters, so the summary, the exit code and any JSON or
//     JUnit output are a full run's;
//   - after it, writes each ran file's result and record (its worker's
//     module graph and reads) for the Rust side to key and store.
import { performance } from 'node:perf_hooks'
const t0 = performance.now()
import { createRecorder, patchEnv, patchFs, patchLoader, real, settledKeys, writeJson, VERSION } from './record.mjs'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import nodeModule from 'node:module'
import { pathToFileURL } from 'node:url'
import crypto from 'node:crypto'
import zlib from 'node:zlib'

const [planPath, outPath, ...vitestArgs] = process.argv.slice(2)
const plan = JSON.parse(real.readFileSync(planPath, 'utf8'))
// A standby render loads vitest beside a run's tests: on idle CPU only.
if (plan.standby) {
  try {
    os.setPriority(19)
  } catch {}
}
const stateDir = plan.stateDir
const dumpDir = path.join(stateDir, 'dump')
const resultsDir = path.join(stateDir, 'results')
for (const d of [dumpDir, resultsDir]) real.mkdirSync(d, { recursive: true })
// (a render runs beside a recording run: the dumps are the recording's)
if (!plan.render) {
  for (const f of real.readdirSync(dumpDir)) {
    try {
      fs.unlinkSync(path.join(dumpDir, f))
    } catch {}
  }
  // The workers' coverage mapping cache (coverage.mjs), kept bounded.
  try {
    if (real.readdirSync(path.join(stateDir, 'covmap')).length > 20000) fs.rmSync(path.join(stateDir, 'covmap'), { recursive: true, force: true })
  } catch {}
}

const codeDir = import.meta.dirname
// node's compile cache (V8 code of every module node loads itself: vitest,
// vite, the plugins, the environment, externalized dependencies) in the
// state dir, so it travels with the ledger. The main process enables it
// here, the workers through the environment they inherit. V8 checks the
// source hash of every entry: a stale entry is recompiled, never used.
// Opt-in (QUOD_VITEST_COMPILE_CACHE=1): measured same-job on ufo it cost
// ~40 ms a run, since the critical path's copy of the state dir is thrown
// away and each run pays the cache writes. Never in record runs: they
// measure V8 coverage, which the cache must not be able to touch.
if (plan.record === false && process.env.QUOD_VITEST_COMPILE_CACHE === '1' && typeof nodeModule.enableCompileCache === 'function') {
  const cc = path.join(stateDir, 'compile-cache')
  try {
    const st = nodeModule.enableCompileCache(cc)
    if (st?.directory) process.env.NODE_COMPILE_CACHE = st.directory
  } catch {}
}
// The critical path (`record: false`) runs the files plain: no hooks, no
// setup file, no coverage; the shadow run (`record: true`) records them.
let record = plan.record !== false
const r = createRecorder({ stateDir, codeDir, mode: 'main' })
r.on = record
// QUOD_VITEST_DISABLE (read before the env is watched): main (no main
// process recorder), setup (no setup file: nothing recorded) or preload (no
// toString wrapper): measuring the recorder's cost only.
const offs = new Set((process.env.QUOD_VITEST_DISABLE ?? '').split(','))
if (record && !offs.has('main')) {
  patchFs(r)
  patchEnv(r)
  patchLoader(r)
}
process.env.QUOD_VITEST_DUMP = dumpDir
process.env.QUOD_VITEST_STATE = stateDir
// The files whose workers take V8 coverage (plan.cov; null: every file):
// only a run that can witness a record again needs it (setup.mjs).
if (record && Array.isArray(plan.cov)) {
  const list = path.join(dumpDir, '.cov-files.json')
  real.writeFileSync(list, JSON.stringify(plan.cov))
  process.env.QUOD_VITEST_COV_FILES = list
} else delete process.env.QUOD_VITEST_COV_FILES
const log = (...a) => process.env.QUOD_VITEST_QUIET || console.error('quod vitest:', ...a)

// vitest, from the project
const cwd = process.cwd()
const req = nodeModule.createRequire(path.join(cwd, 'package.json'))
const vitestPkgJson = req.resolve('vitest/package.json')
const vitestDir = path.dirname(vitestPkgJson)
const vitestPkg = JSON.parse(real.readFileSync(vitestPkgJson, 'utf8'))
const nodeEntry = vitestPkg.exports?.['./node']
const nodeRel = typeof nodeEntry === 'string' ? nodeEntry : nodeEntry?.import ?? nodeEntry?.default
const V = await import(pathToFileURL(path.join(vitestDir, nodeRel)).href)

// The plan holds only if the tools are the ones it was made for.
const facts = { node: process.version, platform: `${process.platform}-${process.arch}`, vitest: vitestPkg.version, recorder: VERSION }
let replay = plan.replay ?? {}
if (plan.expect && Object.keys(facts).some((k) => plan.expect[k] !== facts[k])) {
  log('the tools changed since the ledger was recorded; running every file')
  replay = {}
}
let audits = new Set(plan.audit ?? [])

let args = vitestArgs
if (args[0] === 'run') args = args.slice(1)
const { filter, options } = V.parseCLI(['vitest', 'run', ...args])
const filters = filter.filter((f) => f !== 'run')
if (options.exclude) {
  options.cliExclude = [options.exclude].flat()
  delete options.exclude
}
options.run = true
options.watch = false
process.env.TEST = 'true'
process.env.VITEST = 'true'
process.env.NODE_ENV ??= 'test'

const setupPath = path.join(codeDir, 'setup.mjs')
const ctx = await V.createVitest('test', options, {}, {})
const tInit = performance.now()

// A render (a standby beside a recording run) never renders coverage: a
// report is made from fresh and recorded contributions every run. It stops
// here, before it starts vitest, whose provider would clean the reports
// directory the recording run writes.
if (plan.render && ctx.config.coverage?.enabled) {
  writeJson(outPath, { facts, exitCode: 0, interrupted: false, global: [], ran: [], replayed: [], allFiles: null, render: { ok: false, reason: 'coverage is on', outputs: [] }, auditMismatch: [] })
  process.exit(0)
}

// Coverage (`--coverage` or `coverage.enabled` in the config). Each file
// that runs hands its worker's coverage to the provider in the main process
// (`onAfterSuiteRun`: V8's raw script coverage, or istanbul's per-file
// counters); the provider merges them into the report and checks the
// thresholds. A record keeps that hand-off per test file (results/<id>.cov.json.gz,
// with the coverage configuration's fingerprint in results/<id>.covfp); a
// replay hands the recorded one to the provider before it generates the
// report, so vitest's own provider makes every report and threshold check
// from the same contributions as a full run. A file without a recorded
// contribution of this configuration runs. What Quod cannot carry (another
// provider or version, thresholds that rewrite the config) runs every file
// plain.
const covCfg = ctx.config.coverage
let covOn = !!covCfg?.enabled
let covPlain = null
if (covOn) {
  // Quod's own early precise coverage (an experiment) would change V8's
  // mode under the v8 provider: never with coverage on.
  delete process.env.QUOD_VITEST_COV_EARLY
  const name = covCfg.provider ?? 'v8'
  const major = Number(vitestPkg.version.split('.')[0])
  if (name !== 'v8' && name !== 'istanbul') covPlain = `coverage provider ${name}`
  else if (!(major >= 3 && major <= 5)) covPlain = `coverage with vitest ${vitestPkg.version}`
  else if (covCfg.thresholds?.autoUpdate) covPlain = 'coverage.thresholds.autoUpdate (it rewrites the config)'
  else if (!codeDir.includes(`${path.sep}node_modules${path.sep}`)) covPlain = 'coverage with Quod outside node_modules'
  else if (ctx.config.shard) covPlain = 'coverage with --shard'
  if (covPlain) {
    log(`${covPlain} is not supported: running every file plain`)
    replay = {}
    audits = new Set()
    record = false
    r.on = false
    covOn = false
  }
}
// The configuration's fingerprint, once the provider resolved it: its name,
// version, every option (include, exclude, all, thresholds, reporters...),
// vitest's version and the root.
let covFp = null
let covFpText = ''
// The configuration a record was made under (diagnostics: what changed).
const covConfigFile = path.join(stateDir, 'results', 'coverage-config.json')
let covMismatchLogged = false
const covMismatch = () => {
  if (covMismatchLogged) return
  covMismatchLogged = true
  try {
    const was = JSON.parse(real.readFileSync(covConfigFile, 'utf8'))
    const now = JSON.parse(covFpText)
    const diff = []
    for (const k of new Set([...Object.keys(was), ...Object.keys(now)])) {
      if (k === 'opts') {
        for (const o of new Set([...Object.keys(was.opts ?? {}), ...Object.keys(now.opts ?? {})])) {
          const x = was.opts?.[o]
          const y = now.opts?.[o]
          if (JSON.stringify(x) === JSON.stringify(y)) continue
          if (Array.isArray(x) && Array.isArray(y)) {
            const gone = x.filter((v) => !y.some((w) => JSON.stringify(w) === JSON.stringify(v)))
            const added = y.filter((v) => !x.some((w) => JSON.stringify(w) === JSON.stringify(v)))
            diff.push(`coverage.${o} (-${JSON.stringify(gone).slice(0, 300)} +${JSON.stringify(added).slice(0, 300)})`)
          } else diff.push(`coverage.${o} (${JSON.stringify(x)?.slice(0, 200)} -> ${JSON.stringify(y)?.slice(0, 200)})`)
        }
      } else if (JSON.stringify(was[k]) !== JSON.stringify(now[k])) diff.push(k)
    }
    if (diff.length) log(`coverage: the configuration changed since the record: ${diff.join(', ')}`)
  } catch {}
}
const covFingerprint = () => {
  if (covFp !== null) return covFp
  try {
    const p = ctx.coverageProvider
    let ver = p?.version ?? null
    try {
      ver ??= JSON.parse(real.readFileSync(req.resolve(`@vitest/coverage-${p.name}/package.json`), 'utf8')).version
    } catch {}
    // processingConcurrency is the machine's CPU count: how fast the report
    // is made, not what it says.
    // The include and exclude globs as sets: vitest assembles exclude from
    // the projects in an order that varies run to run (pinia), and a glob
    // list's order does not change what it matches.
    const { processingConcurrency, ...opts } = ctx.config.coverage ?? {}
    for (const k of ['include', 'exclude']) {
      if (Array.isArray(opts[k])) opts[k] = [...new Set(opts[k].map((x) => (typeof x === 'string' ? x : JSON.stringify(x))))].sort()
    }
    const text = JSON.stringify({ v: 1, provider: p?.name ?? null, ver, vitest: vitestPkg.version, root: ctx.config.root, opts }, (k, v) =>
      typeof v === 'function' ? `fn:${v.toString()}` : v instanceof RegExp ? `re:${v}` : v,
    )
    covFp = p && ver ? crypto.createHash('sha256').update(text).digest('hex') : ''
    covFpText = text
  } catch {
    covFp = ''
  }
  return covFp
}
let covConfigWritten = false
const covPaths = (id) => [path.join(stateDir, 'results', `${id}.cov.json.gz`), path.join(stateDir, 'results', `${id}.covfp`)]
// A replayed file's recorded contribution, or null (then it runs).
const loadCov = (id) => {
  const fp = covFingerprint()
  if (!fp) return null
  const [gz, fpFile] = covPaths(id)
  try {
    if (real.readFileSync(fpFile, 'utf8') !== fp) {
      covMismatch()
      return null
    }
    const meta = JSON.parse(zlib.gunzipSync(real.readFileSync(gz)).toString('utf8'))
    // vitest 3 transforms a module the run did not load in ssr mode when it
    // makes the report: a web-mode record would map onto other code.
    if (meta.transformMode !== undefined && meta.transformMode !== 'ssr') return null
    return meta
  } catch {
    return null
  }
}
const replayCov = new Map() // id -> the recorded onAfterSuiteRun payload
const covByFile = new Map() // `${project}\0${name}` -> task id, for the files that run
const captured = new Map() // id -> payloads this run's workers handed over
const covShared = new Set() // ids whose coverage came in one payload with other files
let covWrapped = false
let covTmp = null // the provider's temp directory, with a trailing separator
const wrapProvider = (p) => {
  if (!p || covWrapped || !covOn) return
  covWrapped = true
  if (typeof p.coverageFilesDirectory === 'string') covTmp = p.coverageFilesDirectory + path.sep
  const after = p.onAfterSuiteRun.bind(p)
  p.onAfterSuiteRun = (meta0) => {
    let meta = meta0
    // vitest 5 hands over a file the worker wrote (removed after the
    // report): kept by content.
    if (record && typeof meta?.coverage === 'string') {
      try {
        meta = { ...meta0, coverage: JSON.parse(real.readFileSync(meta0.coverage, 'utf8')), quodFile: true }
      } catch {
        meta = { ...meta0, coverage: null }
      }
    }
    if (record && meta?.coverage) {
      const ids = (meta.testFiles ?? []).map((f) => covByFile.get(`${meta.projectName ?? ''}\0${f}`))
      for (const id of ids) {
        if (!id) continue
        if (ids.length !== 1) covShared.add(id)
        const l = captured.get(id) ?? []
        l.push(meta)
        captured.set(id, l)
      }
    }
    return after(meta0)
  }
  // The report is made fresh every run from every file's contribution: what
  // the provider reads and writes then is not an input of any test file.
  const gen = p.generateCoverage.bind(p)
  p.generateCoverage = async (...a) => {
    let n = 0
    for (const meta of replayCov.values()) {
      if (meta.quodFile) {
        // as the worker would have: a file in the provider's directory
        const { quodFile, ...m } = meta
        const f = path.join(p.coverageFilesDirectory, `quod-replay-${n++}.json`)
        real.writeFileSync(f, JSON.stringify(m.coverage))
        after({ ...m, coverage: f })
      } else after(meta)
    }
    if (replayCov.size) log(`coverage: the recorded coverage of ${replayCov.size} replayed files goes into the report`)
    const was = r.on
    r.on = false
    try {
      return await gen(...a)
    } finally {
      r.on = was
    }
  }
  if (typeof p.clean === 'function') {
    const clean = p.clean.bind(p)
    p.clean = async (...a) => {
      const was = r.on
      r.on = false
      try {
        return await clean(...a)
      } finally {
        r.on = was
      }
    }
  }
}
if (covOn) {
  const init = ctx.initCoverageProvider.bind(ctx)
  ctx.initCoverageProvider = async (...a) => {
    const was = r.on
    const res = await init(...a)
    r.on = was
    wrapProvider(ctx.coverageProvider)
    // as resolved, before the run can touch it
    if (ctx.coverageProvider) covFingerprint()
    return res
  }
  wrapProvider(ctx.coverageProvider)
  if (ctx.coverageProvider) covFingerprint()
  const report = ctx.reportCoverage.bind(ctx)
  ctx.reportCoverage = async (...a) => {
    const was = r.on
    r.on = false
    try {
      return await report(...a)
    } finally {
      r.on = was
    }
  }
}

process.env.QUOD_VITEST_ROOT = ctx.config.root
// vitest 3's per-run directories of transformed modules (record.mjs `skip`).
const tmpDirs = ctx.projects.map((p) => p.tmpDir).filter((d) => typeof d === 'string' && path.isAbsolute(d))
if (tmpDirs.length) process.env.QUOD_VITEST_SKIP = [...new Set(tmpDirs)].join('\n')
// The repository the project is in (the nearest .git up from its root): in
// a monorepo whose packages each run vitest (npm or yarn workspaces), a
// write to a sibling package is a write to the workspace too.
let wsRoot = ctx.config.root
for (let d = ctx.config.root; ; d = path.dirname(d)) {
  if (real.existsSync(path.join(d, '.git'))) {
    wsRoot = d
    break
  }
  if (path.dirname(d) === d) break
}
wsRoot += '/'
// For the workers: a directory a test file creates outside it is its own.
process.env.QUOD_VITEST_WS = wsRoot

// A large run (the plan says so) uses every CPU, not vitest's default of
// one fewer: the main process idles while workers run. Only when neither
// the config nor the arguments chose a number.
if (plan.workers && !ctx.config.maxWorkers && ctx.projects.every((p) => !p.config.maxWorkers)) {
  ctx.config.maxWorkers = plan.workers
}

// Render mode (a shadow run, after it recorded): replay every file and keep
// what the reporters print and write, for a later critical path that
// replays every file to emit without starting vitest. Only reporters whose
// every effect is stdout, stderr, an outputFile, or GitHub's step summary;
// the files they write go to the render dir, and where they belong is kept.
let render = null
if (plan.render) {
  render = { ok: true, reason: '', outputs: [] }
  const no = (why) => {
    if (render.ok) Object.assign(render, { ok: false, reason: why })
  }
  const known = new Set(['DefaultReporter', 'VerboseReporter', 'DotReporter', 'TreeReporter', 'JsonReporter', 'JUnitReporter', 'TapReporter', 'TapFlatReporter', 'GithubActionsReporter'])
  const fileOf = { JsonReporter: 'json', JUnitReporter: 'junit' }
  // A project's root and test dir as vitest resolves them: `dir` may be
  // kept as written (`tests`), relative to the project's root, and a
  // relative root is relative to the cwd.
  const here = path.resolve(cwd)
  const cwdSlash = here.endsWith('/') ? here : `${here}/`
  const inside = (p) => p === here || p.startsWith(cwdSlash)
  for (const p of ctx.projects) {
    const root = path.resolve(here, p.config.root)
    if (!inside(root)) no(`project root ${root} is outside ${here}`)
    const dir = p.config.dir ? path.resolve(root, p.config.dir) : ''
    if (dir && !inside(dir)) no(`test dir ${dir} is outside ${here}`)
  }
  if (ctx.config.coverage?.enabled) no('coverage is on')
  const of = ctx.config.outputFile
  const outFiles = typeof of === 'string' ? { json: of, junit: of } : { ...(of ?? {}) }
  let n = 0
  for (const rep of ctx.reporters) {
    const name = rep?.constructor?.name
    if (!known.has(name)) {
      no(`reporter ${name ?? typeof rep} is not one Quod can render`)
      continue
    }
    if (rep.options?.outputFile) no(`reporter ${name} has its own outputFile`)
    const kind = fileOf[name]
    if (kind && outFiles[kind]) {
      const file = `out-${n++}`
      render.outputs.push({ file, to: path.resolve(ctx.config.root, outFiles[kind]), append: false, env: null })
      outFiles[kind] = path.join(plan.render, file)
    }
    if (name === 'GithubActionsReporter' && rep.options?.jobSummary?.enabled && rep.options.jobSummary.outputPath) {
      const file = `out-${n++}`
      const to = rep.options.jobSummary.outputPath
      render.outputs.push({ file, to, append: true, env: to === process.env.GITHUB_STEP_SUMMARY ? 'GITHUB_STEP_SUMMARY' : null })
      rep.options.jobSummary.outputPath = path.join(plan.render, file)
    }
  }
  if (typeof of === 'string') {
    // one path for whichever single file reporter is in use
    const used = Object.keys(fileOf).filter((k) => ctx.reporters.some((r) => r?.constructor?.name === k))
    if (used.length > 1) no('one outputFile for two file reporters')
    ctx.config.outputFile = used.length ? outFiles[fileOf[used[0]]] : of
  } else if (of) ctx.config.outputFile = outFiles
}

// A standby render (started beside a recording run) waits here, vitest
// loaded, for the records to replay: the go file Quod writes once the
// recording is in, or the end of Quod (it kills this).
if (plan.standby) {
  const ppid = process.ppid
  let go = null
  while (!go) {
    if (real.existsSync(plan.standby)) go = JSON.parse(real.readFileSync(plan.standby, 'utf8'))
    else if (process.ppid !== ppid) process.exit(0)
    else await new Promise((res) => setTimeout(res, 5))
  }
  if (go.abort) process.exit(0)
  replay = go.replay ?? {}
  if (go.expect && Object.keys(facts).some((k) => go.expect[k] !== facts[k])) replay = {}
}

// Projects Quod can record: isolated node workers. Others always run.
const recorded = new Set()
for (const p of ctx.projects) {
  const c = p.config
  // With typecheck on, the type tests (pool `typescript`, tsc) always run;
  // the project's other files are recorded as usual.
  const ok = !c.browser?.enabled && c.isolate !== false && !String(c.pool).startsWith('vm') && !c.typecheck?.only
  if (ok) {
    if (record && !offs.has('setup')) {
      c.setupFiles = [setupPath, ...c.setupFiles]
      if (!offs.has('preload')) c.execArgv = [...(c.execArgv ?? []), '--require', path.join(codeDir, 'preload.cjs')]
    }
    recorded.add(p.name)
  } else log(`project "${p.name}" is not recorded (browser, vm pool, typecheck only or not isolated): its files always run`)
}

// Partition vitest's own list of test files.
let allFiles = null
const replayed = [] // { spec, id, result }
const ran = new Map() // id -> spec
const specsApi = ctx.specifications
const origRelevant = specsApi.getRelevantTestSpecifications.bind(specsApi)
specsApi.getRelevantTestSpecifications = async (f) => {
  r.on = false // the glob's directory walk is vitest's choice of files, decided here exactly
  const all = await origRelevant(f)
  r.on = record
  const run = []
  let covMissing = 0
  for (const s of all) {
    const id = s.taskId
    const res = replay[id]
    let ok = res && !audits.has(id) && recorded.has(s.project.name) && s.pool !== 'typescript' && !s.testLines?.length && !s.testNamePattern
    if (ok && covOn) {
      const c = loadCov(id)
      if (c) replayCov.set(id, c)
      else {
        ok = false
        covMissing++
      }
    } else if (ok && real.existsSync(covPaths(id)[1])) {
      // recorded under coverage (instrumented), replayed without: it runs
      ok = false
      covMissing++
    }
    if (ok) {
      replayed.push({ spec: s, id, result: res })
    } else {
      run.push(s)
      ran.set(id, s)
      if (covOn) {
        const name = path.relative(s.project.config.root, s.moduleId).split(path.sep).join('/')
        covByFile.set(`${s.project.name ?? ''}\0${name}`, id)
        covByFile.set(`${s.project.name ?? ''}\0${s.moduleId}`, id)
      }
    }
  }
  if (covMissing) log(`coverage: ${covMissing} files were recorded under another coverage configuration (or none): they run`)
  allFiles = all.map((s) => s.taskId)
  log(`${all.length} test files: running ${run.length}, replaying ${replayed.length}${audits.size ? ` (auditing ${[...audits].filter((a) => ran.has(a)).length})` : ''}`)
  return run
}

function rebuild(t, file, parent) {
  if (t.__s) t.suite = parent
  delete t.__s
  t.file = file
  for (const c of t.tasks ?? []) rebuild(c, file, t)
  return t
}
function strip(file) {
  const seen = new WeakSet()
  return JSON.stringify(file, function (k, v) {
    if (k === 'file') return undefined
    if (k === 'suite') return undefined
    if (v && typeof v === 'object') {
      if (seen.has(v)) return undefined
      seen.add(v)
      if (Array.isArray(v.tasks) && v.tasks.length) {
        for (const c of v.tasks) if (c && c.suite === v) c.__s = 1
      }
    }
    return v
  })
}
function testStates(file) {
  const out = []
  const walk = (t, names) => {
    const n = [...names, t.name]
    if (t.type === 'test') out.push([n.join(' > '), t.result?.state ?? t.mode])
    for (const c of t.tasks ?? []) walk(c, n)
  }
  for (const c of file.tasks ?? []) walk(c, [])
  return out.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0))
}

// Tests that took more than half their timeout: a pass a loaded runner can
// turn into a timeout (mistral-transcription-stream: 2.3 s of 5 s here,
// timed out later under load). The record is kept, never replayed, until
// a run with every test within half (the ledger's `near_timeout`). The
// timeout in force: the test's own, else its project's testTimeout.
function nearTimeout(file) {
  const proj = ctx.projects.find((p) => (p.name ?? p.config?.name ?? '') === (file.projectName ?? ''))
  const dflt = proj?.config?.testTimeout ?? ctx.config.testTimeout ?? 5000
  const out = []
  const walk = (t, names) => {
    const n = [...names, t.name]
    if (t.type === 'test') {
      const limit = t.timeout ?? dflt
      const took = t.result?.duration
      if (limit > 0 && Number.isFinite(limit) && took > limit / 2) out.push(`${n.join(' > ')}: ${Math.round(took)} of ${limit} ms`)
    }
    for (const c of t.tasks ?? []) walk(c, n)
  }
  for (const c of file.tasks ?? []) walk(c, [])
  return out
}

// Replays reported just before the run's end.
const testRun = ctx._testRun
const origEnd = testRun.end.bind(testRun)
let endModules = []
// The replayed files' recorded tasks, for --report.
const replayedTasks = []
testRun.end = async (specs, errors, coverage) => {
  const tr = performance.now()
  const extra = []
  for (const { spec, id, result } of replayed) {
    let file
    try {
      file = JSON.parse(real.readFileSync(path.join(stateDir, result), 'utf8'))
      rebuild(file, file, null)
    } catch (e) {
      log(`replay of ${spec.moduleId} failed to load (${e.message})`)
      ctx.state.catchError(new Error(`quod: replay of ${spec.moduleId} could not be loaded`), 'Quod Error')
      continue
    }
    file.id = id
    await ctx._reportFileTask(file)
    extra.push(spec)
    replayedTasks.push(file)
  }
  if (replayed.length) log(`reported ${extra.length} replayed files in ${(performance.now() - tr).toFixed(0)} ms`)
  const all = [...specs, ...extra]
  endModules = all.map((s) => s.testModule).filter(Boolean)
  return origEnd(all, errors, coverage)
}

let crashed = false
try {
  await ctx.start(filters.map((f) => path.normalize(f)))
} catch (e) {
  const name = e?.constructor?.name
  if (name !== 'FilesNotFoundError' && name !== 'GitNotFoundError') {
    crashed = true
    process.exitCode = 1
    ctx.logger.printError(e, { fullStack: true, type: 'Unhandled Error' })
  }
} finally {
  await ctx.close()
}
const tRun = performance.now()
r.on = false

// Results and records of the files that ran.
const ranOut = []
const auditMismatch = []
// The critical path's vitest JSON report (`--expect-report`): per file, its
// tests' names and states, in testStates' form.
// A file is the same file in each project that runs it (a config with
// several projects, e.g. one per dependency version): one entry per
// project and file. A report without projects (an older quod's) verifies
// only a file it lists once.
const fileKey = (project, file) => `${project ?? ''}\u0000${file}`
let expected = null
if (plan.expectReport) {
  try {
    expected = new Map()
    const byName = new Map()
    for (const t of JSON.parse(real.readFileSync(plan.expectReport, 'utf8')).testResults) {
      const tests = t.assertionResults.map(
        (x) => `${[...x.ancestorTitles, x.title].join(' > ')}=${x.status === 'passed' || x.status === 'failed' ? x.status : 'other'}`,
      )
      const v = JSON.stringify(tests.sort())
      if (typeof t.project === 'string') expected.set(fileKey(t.project, t.name), v)
      else byName.set(t.name, byName.has(t.name) ? null : v)
    }
    expected.byName = byName
  } catch (e) {
    expected = new Map() // unreadable: nothing verifies, so nothing that ran records
    expected.byName = new Map()
    log(`expect report ${plan.expectReport} unreadable (${e.message}): nothing that ran is recorded`)
  }
}
const moduleKeys = (mods) => {
  const w = createRecorder({ stateDir, codeDir, mode: 'worker' })
  for (const m of mods) w.add('f', m)
  return w.keys
}
const transformed = new Set()
const dumps = new Map()
const workerMs = { setup: 0, dump: 0, cov: 0, map: 0, keys: 0, ticks: 0 }
const workerSteps = {}
const tPost = {}
let tp = performance.now()
const lap = (k) => {
  const n = performance.now()
  tPost[k] = (tPost[k] ?? 0) + n - tp
  tp = n
}
const unmapped = {}
let unmappedSample = null
for (const id of ran.keys()) {
  let d = null
  try {
    d = JSON.parse(real.readFileSync(path.join(dumpDir, `${id}.json`), 'utf8'))
  } catch {}
  dumps.set(id, d)
  if (d) for (const m of d.modules) transformed.add(path.resolve(m))
  if (d?.ms) for (const k of Object.keys(workerMs)) workerMs[k] += d.ms[k] ?? 0
  for (const [k, v] of Object.entries(d?.ms?.steps ?? {})) workerSteps[k] = (workerSteps[k] ?? 0) + v
  for (const [k, n] of Object.entries(d?.unmapped ?? {})) unmapped[k] = (unmapped[k] ?? 0) + n
  if (d?.unmappedSample?.length && !unmappedSample) unmappedSample = d.unmappedSample[0]
}
lap('dumps')
const interrupted = ctx.isCancelling || crashed
const sources = new Map() // reached source files' text, read once
for (const m of endModules) {
  const id = m.task.id
  if (!ran.has(id)) continue
  const file = m.task
  const st = file.result?.state
  const failed = st === 'fail' || testStates(file).some(([, s]) => s === 'fail')
  const state = failed ? 'fail' : st === 'pass' ? 'pass' : st === 'skip' || file.mode === 'skip' ? 'skip' : (st ?? 'none')
  const d = dumps.get(id)
  let stripped = null
  const text = () => (stripped ??= strip(file)) // back references (task.file, suite) make the task circular
  let keys = null
  let unobservable = []
  let cov = null
  let covWhy = d ? null : 'no record'
  if (d) {
    const ks = new Set(d.keys)
    for (const k of moduleKeys(d.modules)) ks.add(k)
    // A watched path is read whole (recursively, content included) at the
    // end of the file. Under a directory the file itself created outside
    // the workspace (its own output, e.g. a temp dir) it is not an input
    // at all: the worker never lists those (record.mjs `made`).
    for (const w of d.watches ?? []) ks.add(`rc:${w}`)
    keys = [...ks]
    unobservable = d.unobservable
    // vitest 5's worker writes its coverage to the provider's temp directory
    // itself (handed over by name): vitest's file, not the test's output.
    const ws = d.writes.filter((w) => w.startsWith(wsRoot) && !w.includes('/node_modules/') && !(covTmp && w.startsWith(covTmp)))
    if (ws.length) unobservable = [...unobservable, `writes the workspace (${path.relative(ctx.config.root, ws[0])})`]
    // Function-level data only for source the file reached as modules (a
    // file it also read, or loaded natively, counts whole), not itself;
    // a file with a function that maps nowhere gets none.
    if (!d.cov) covWhy = d.covOn ? 'coverage is on' : d.covSkipped ? 'a first witness (coverage not taken)' : d.usedToString ? 'reads function source (toString)' : 'no coverage'
    if (d.cov) {
      cov = {}
      for (const [f, fns] of Object.entries(d.cov)) {
        if (f !== file.filepath && !d.keys.includes(`f:${f}`) && !d.keys.includes(`e:${f}`)) cov[f] = fns
      }
      // Belt and braces: a result (test names, messages, logs) that quotes
      // the source of a file it reached gets no narrowing for any file.
      const snippets = [...text().matchAll(/[^"\\]{0,24}(?:=>|function\b)[^"\\]{0,24}/g)].slice(0, 50).map((m) => m[0].trim()).filter((x) => x.length >= 8)
      if (snippets.length) {
        for (const f of Object.keys(cov)) {
          let src = sources.get(f)
          if (src === undefined) {
            try {
              src = real.readFileSync(f, 'utf8')
            } catch {
              src = ''
            }
            sources.set(f, src)
          }
          if (snippets.some((x) => src.includes(x))) {
            cov = null
            covWhy = 'its results quote reached source'
            break
          }
        }
      }
    }
  }
  const resultRel = `results/${id}.json`
  if (record && (state === 'pass' || state === 'skip')) {
    real.writeFileSync(path.join(stateDir, resultRel), text())
    // Its coverage contribution goes with the result, or none does (a
    // record without one never replays under coverage).
    const [gz, fpFile] = covPaths(id)
    for (const f of [fpFile, gz]) {
      try {
        fs.unlinkSync(f)
      } catch {}
    }
    const metas = captured.get(id)
    const fp = covOn ? covFingerprint() : ''
    if (fp && metas?.length === 1 && !covShared.has(id)) {
      real.writeFileSync(gz, zlib.gzipSync(JSON.stringify(metas[0]), { level: 6 }))
      real.writeFileSync(fpFile, fp)
      if (!covConfigWritten) {
        covConfigWritten = true
        real.writeFileSync(covConfigFile, covFpText)
      }
    } else if (covOn && dumps.get(id)) log(`coverage: ${file.filepath} handed over ${metas?.length ?? 0} coverage results${covShared.has(id) ? ' (with other files)' : ''}: not kept, it runs next time`)
  }
  // The shadow's safety net: a file records only if it ran here as it did on
  // the critical path of the same commit (uninstrumented); else recording
  // changed its behaviour, and it is quarantined.
  // A file the report does not have (or an unreadable report) cannot be
  // verified: it is not recorded (and runs next time), but not quarantined.
  // An audit is verified against its own record instead (below): the
  // critical path replayed it, so the report has it only as a replay, or
  // not at all after a rendered full replay.
  if (record && expected && !auditMismatch.includes(id) && !audits.has(id)) {
    const want = expected.get(fileKey(file.projectName, file.filepath)) ?? expected.byName.get(file.filepath) ?? undefined
    const got = JSON.stringify(testStates(file).map(([n, s]) => `${n}=${s === 'pass' ? 'passed' : s === 'fail' ? 'failed' : 'other'}`).sort())
    if (want === undefined) {
      keys = null
      log(`${file.filepath} is not in the critical path's report: not recorded`)
    } else if (want !== got) {
      auditMismatch.push(id)
      log(`recording changed behaviour: ${file.filepath} ran differently than on the critical path; quarantined`)
    }
  }
  if (audits.has(id) && plan.replay?.[id]) {
    try {
      const rec = JSON.parse(real.readFileSync(path.join(stateDir, plan.replay[id]), 'utf8'))
      const a = JSON.stringify(testStates(rec))
      const b = JSON.stringify(testStates(file))
      if (a !== b) {
        auditMismatch.push(id)
        log(`AUDIT MISMATCH: ${file.filepath} ran differently from its recorded result; quarantined`)
      }
    } catch {}
  }
  ranOut.push({ id, project: file.projectName ?? '', file: file.filepath, state, duration: fileTime(file), keys, unobservable, result: resultRel, cov, covWhy, tests: JSON.stringify(testStates(file)), nearTimeout: nearTimeout(file) })
}
lap('files')

// `--report`: every test's name and state per file, in the shape of vitest's
// JSON report, whatever reporters the run has (the shadow's --expect-report).
if (plan.report) {
  const status = (st) => (st === 'pass' ? 'passed' : st === 'fail' ? 'failed' : st === 'skip' || st === 'todo' ? 'skipped' : 'pending')
  // Replayed files too, with their recorded results: a shadow that has to
  // run a file this run replayed (its ledger is another) can still verify
  // it, as an audit would.
  const tasks = new Map()
  for (const t of replayedTasks) tasks.set(fileKey(t.projectName, t.filepath), t)
  for (const m of endModules) tasks.set(fileKey(m.task.projectName, m.task.filepath), m.task)
  const testResults = [...tasks.values()].map((t) => ({
    name: t.filepath,
    project: t.projectName ?? '',
    assertionResults: testStates(t).map(([title, st]) => ({ ancestorTitles: [], title, status: status(st) })),
  }))
  writeJson(plan.report, { testResults })
}

// The main process's reads: inputs of every file, except the source the
// workers asked it to transform (each file's own module graph has those).
const global = []
for (const k of settledKeys(r)) {
  const i = k.indexOf(':')
  const p = k.slice(i + 1)
  if (k.startsWith('f:') && transformed.has(p)) continue
  global.push(k)
}
for (const p of ctx.projects) {
  for (const f of [p.vite?.config?.configFile, ...(p.vite?.config?.configFileDependencies ?? [])]) {
    if (f) global.push(`f:${path.resolve(f)}`)
  }
}
// vitest and its dependencies run in every worker before Quod's setup file
// can watch: the packages, transitively.
const pkgs = new Set()
const stack = record ? [real.realpathSync(vitestDir)] : []
for (const p of ctx.projects) {
  const env = p.config.environment
  if (env && env !== 'node') {
    const name = { jsdom: 'jsdom', 'happy-dom': 'happy-dom', 'edge-runtime': '@edge-runtime/vm' }[env] ?? env
    try {
      stack.push(real.realpathSync(path.dirname(req.resolve(`${name}/package.json`))))
    } catch {}
  }
}
while (stack.length) {
  const d = stack.pop()
  if (pkgs.has(d)) continue
  pkgs.add(d)
  let pj
  try {
    pj = JSON.parse(real.readFileSync(path.join(d, 'package.json'), 'utf8'))
  } catch {
    continue
  }
  for (const n of Object.keys({ ...pj.dependencies, ...pj.optionalDependencies, ...pj.peerDependencies })) {
    let dir = d
    for (;;) {
      const c = path.join(dir, 'node_modules', n)
      if (real.existsSync(path.join(c, 'package.json'))) {
        stack.push(real.realpathSync(c))
        break
      }
      const up = path.dirname(dir)
      if (up === dir) break
      dir = up
    }
  }
}
for (const d of pkgs) global.push(`p:${d}`)
lap('global')

writeJson(outPath, {
  facts,
  exitCode: process.exitCode ?? 0,
  interrupted,
  global: [...new Set(global)],
  ran: ranOut,
  replayed: replayed.map((x) => x.id),
  allFiles,
  render,
  auditMismatch,
  timings: {
    init: tInit - t0,
    run: tRun - tInit,
    record: performance.now() - tRun,
    ...Object.fromEntries(Object.entries(tPost).map(([k, v]) => [`post_${k}`, v])),
    ...Object.fromEntries(Object.entries(workerSteps).map(([k, v]) => [`workerStep_${k}`, v])),
    workerSetupSum: workerMs.setup,
    workerDumpSum: workerMs.dump,
    workerCovSum: workerMs.cov,
    workerMapSum: workerMs.map,
    workerKeysSum: workerMs.keys,
    workerTicks: workerMs.ticks,
    files: ran.size,
  },
  unmapped,
})
for (const [p, frame] of r.pjWhole) log(`${path.relative(cwd, p)} kept whole (read by ${frame})`)
for (const [p, use] of r.pjUse) if (!r.pjWhole.has(p)) log(`${path.relative(cwd, p)} keyed by its names and the fields read: ${[...use].sort().join(', ') || '(none)'}`)
for (const [frame, n] of r.envChild) log(`the environment copied ${n}x for a child process (${frame}): keyed as the tools' copy`)
for (const [k, frame] of r.envKept) log(`${k === '*' ? 'the whole environment' : k} keyed for every file (read by ${frame})`)
// The same for the workers: who in a test file's worker read CI-provider
// variables (a test, a library it loads, the environment), by reader.
{
  const byFrame = new Map()
  for (const d of dumps.values()) {
    for (const [k, frame] of d?.envKept ?? []) {
      const e = byFrame.get(frame) ?? { files: 0, names: new Set() }
      e.names.add(k)
      byFrame.set(frame, e)
    }
    for (const frame of new Set((d?.envKept ?? []).map((x) => x[1]))) byFrame.get(frame).files++
  }
  for (const [frame, e] of [...byFrame].sort((a, b) => b[1].files - a[1].files).slice(0, 5)) {
    log(`${e.files} files read CI variables (${[...e.names].slice(0, 4).join(', ')}${e.names.size > 4 ? ', ...' : ''}) through ${frame}`)
  }
}
if (Object.keys(unmapped).length) log(`source files without function-level data: ${JSON.stringify(unmapped)}`)
if (unmappedSample) log(`e.g. ${unmappedSample[0]} ends with: ${JSON.stringify(unmappedSample[1])}`)
log(`vitest init ${(tInit - t0).toFixed(0)} ms, run ${(tRun - tInit).toFixed(0)} ms, record ${(performance.now() - tRun).toFixed(0)} ms`)
process.exit(process.exitCode ?? 0)

// A file's time as a run spends it: its tests, plus collecting its module
// graph, its setup files, the worker's preparation and its environment
// (what a replay saves); vitest's per-file numbers when it has them.
function fileTime(file) {
  const n = (v) => (typeof v === 'number' && Number.isFinite(v) && v > 0 ? v : 0)
  return n(file.result?.duration) + n(file.collectDuration) + n(file.setupDuration) + n(file.prepareDuration) + n(file.environmentLoad)
}
