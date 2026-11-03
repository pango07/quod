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
import { createRecorder, patchEnv, patchFs, patchLoader, real, writeJson, VERSION } from './record.mjs'
import fs from 'node:fs'
import path from 'node:path'
import nodeModule from 'node:module'
import { pathToFileURL } from 'node:url'

const [planPath, outPath, ...vitestArgs] = process.argv.slice(2)
const plan = JSON.parse(real.readFileSync(planPath, 'utf8'))
const stateDir = plan.stateDir
const dumpDir = path.join(stateDir, 'dump')
const resultsDir = path.join(stateDir, 'results')
for (const d of [dumpDir, resultsDir]) real.mkdirSync(d, { recursive: true })
for (const f of real.readdirSync(dumpDir)) {
  try {
    fs.unlinkSync(path.join(dumpDir, f))
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
const record = plan.record !== false
const r = createRecorder({ stateDir, codeDir, mode: 'main' })
r.on = record
// QUOD_VITEST_DISABLE=main: measuring the main process recorder's cost only.
if (record && !(process.env.QUOD_VITEST_DISABLE ?? '').split(',').includes('main')) {
  patchFs(r)
  patchEnv(r)
  patchLoader(r)
}
process.env.QUOD_VITEST_DUMP = dumpDir
process.env.QUOD_VITEST_STATE = stateDir
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
const audits = new Set(plan.audit ?? [])

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

process.env.QUOD_VITEST_ROOT = ctx.config.root
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
  const cwdSlash = cwd.endsWith('/') ? cwd : `${cwd}/`
  for (const p of ctx.projects) {
    if (p.config.root !== cwd && !p.config.root.startsWith(cwdSlash)) no(`project root ${p.config.root} is outside ${cwd}`)
    if (p.config.dir && p.config.dir !== cwd && !p.config.dir.startsWith(cwdSlash)) no(`test dir ${p.config.dir} is outside ${cwd}`)
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

// Projects Quod can record: isolated node workers. Others always run.
const recorded = new Set()
for (const p of ctx.projects) {
  const c = p.config
  const ok = !c.browser?.enabled && c.isolate !== false && !String(c.pool).startsWith('vm') && !c.typecheck?.enabled
  if (ok) {
    if (record) {
      c.setupFiles = [setupPath, ...c.setupFiles]
      c.execArgv = [...(c.execArgv ?? []), '--require', path.join(codeDir, 'preload.cjs')]
    }
    recorded.add(p.name)
  } else log(`project "${p.name}" is not recorded (browser, vm pool, typecheck or not isolated): its files always run`)
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
  for (const s of all) {
    const id = s.taskId
    const res = replay[id]
    if (res && !audits.has(id) && recorded.has(s.project.name) && !s.testLines?.length && !s.testNamePattern) {
      replayed.push({ spec: s, id, result: res })
    } else {
      run.push(s)
      ran.set(id, s)
    }
  }
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

// Replays reported just before the run's end.
const testRun = ctx._testRun
const origEnd = testRun.end.bind(testRun)
let endModules = []
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
let expected = null
if (plan.expectReport) {
  try {
    expected = new Map()
    for (const t of JSON.parse(real.readFileSync(plan.expectReport, 'utf8')).testResults) {
      const tests = t.assertionResults.map(
        (x) => `${[...x.ancestorTitles, x.title].join(' > ')}=${x.status === 'passed' || x.status === 'failed' ? x.status : 'other'}`,
      )
      expected.set(t.name, JSON.stringify(tests.sort()))
    }
  } catch (e) {
    expected = new Map() // unreadable: nothing verifies, so nothing that ran records
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
const workerMs = { setup: 0, dump: 0, cov: 0 }
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
  for (const [k, n] of Object.entries(d?.unmapped ?? {})) unmapped[k] = (unmapped[k] ?? 0) + n
  if (d?.unmappedSample?.length && !unmappedSample) unmappedSample = d.unmappedSample[0]
}
const interrupted = ctx.isCancelling || crashed
for (const m of endModules) {
  const id = m.task.id
  if (!ran.has(id)) continue
  const file = m.task
  const st = file.result?.state
  const failed = st === 'fail' || testStates(file).some(([, s]) => s === 'fail')
  const state = failed ? 'fail' : st === 'pass' ? 'pass' : st === 'skip' || file.mode === 'skip' ? 'skip' : (st ?? 'none')
  const d = dumps.get(id)
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
    const ws = d.writes.filter((w) => w.startsWith(wsRoot) && !w.includes('/node_modules/'))
    if (ws.length) unobservable = [...unobservable, `writes the workspace (${path.relative(ctx.config.root, ws[0])})`]
    // Function-level data only for source the file reached as modules (a
    // file it also read, or loaded natively, counts whole), not itself;
    // a file with a function that maps nowhere gets none.
    if (!d.cov) covWhy = d.usedToString ? 'reads function source (toString)' : 'no coverage'
    if (d.cov) {
      cov = {}
      for (const [f, fns] of Object.entries(d.cov)) {
        if (f !== file.filepath && !d.keys.includes(`f:${f}`) && !d.keys.includes(`e:${f}`)) cov[f] = fns
      }
      // Belt and braces: a result (test names, messages, logs) that quotes
      // the source of a file it reached gets no narrowing for any file.
      const text = strip(file) // back references (task.file, suite) make the task circular
      const snippets = [...text.matchAll(/[^"\\]{0,24}(?:=>|function\b)[^"\\]{0,24}/g)].slice(0, 50).map((m) => m[0].trim()).filter((x) => x.length >= 8)
      if (snippets.length) {
        for (const f of Object.keys(cov)) {
          let src = ''
          try {
            src = real.readFileSync(f, 'utf8')
          } catch {}
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
  if (record && (state === 'pass' || state === 'skip')) real.writeFileSync(path.join(stateDir, resultRel), strip(file))
  // The shadow's safety net: a file records only if it ran here as it did on
  // the critical path of the same commit (uninstrumented); else recording
  // changed its behaviour, and it is quarantined.
  // A file the report does not have (or an unreadable report) cannot be
  // verified: it is not recorded (and runs next time), but not quarantined.
  // An audit is verified against its own record instead (below): the
  // critical path replayed it, so the report has it only as a replay, or
  // not at all after a rendered full replay.
  if (record && expected && !auditMismatch.includes(id) && !audits.has(id)) {
    const want = expected.get(file.filepath)
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
  ranOut.push({ id, project: file.projectName ?? '', file: file.filepath, state, duration: file.result?.duration ?? 0, keys, unobservable, result: resultRel, cov, covWhy })
}

// `--report`: every test's name and state per file, in the shape of vitest's
// JSON report, whatever reporters the run has (the shadow's --expect-report).
if (plan.report) {
  const status = (st) => (st === 'pass' ? 'passed' : st === 'fail' ? 'failed' : st === 'skip' || st === 'todo' ? 'skipped' : 'pending')
  const testResults = endModules.map((m) => ({
    name: m.task.filepath,
    assertionResults: testStates(m.task).map(([title, st]) => ({ ancestorTitles: [], title, status: status(st) })),
  }))
  writeJson(plan.report, { testResults })
}

// The main process's reads: inputs of every file, except the source the
// workers asked it to transform (each file's own module graph has those).
const global = []
for (const k of r.keys) {
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
  timings: { init: tInit - t0, run: tRun - tInit, record: performance.now() - tRun, workerSetupSum: workerMs.setup, workerDumpSum: workerMs.dump, workerCovSum: workerMs.cov },
  unmapped,
})
for (const [p, frame] of r.pjWhole) log(`${path.relative(cwd, p)} kept whole (read by ${frame})`)
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
