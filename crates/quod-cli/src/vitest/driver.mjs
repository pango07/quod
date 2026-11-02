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

const r = createRecorder({ stateDir, mode: 'main' })
patchFs(r)
patchEnv(r)
patchLoader(r)
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

const setupPath = path.join(stateDir, 'setup.mjs')
const ctx = await V.createVitest('test', options, {}, {})
const tInit = performance.now()

// Projects Quod can record: isolated node workers. Others always run.
const recorded = new Set()
for (const p of ctx.projects) {
  const c = p.config
  const ok = !c.browser?.enabled && c.isolate !== false && !String(c.pool).startsWith('vm') && !c.typecheck?.enabled
  if (ok) {
    c.setupFiles = [setupPath, ...c.setupFiles]
    recorded.add(p.name)
  } else log(`project "${p.name}" is not recorded (browser, vm pool, typecheck or not isolated): its files always run`)
}

// Partition vitest's own list of test files.
const replayed = [] // { spec, id, result }
const ran = new Map() // id -> spec
const specsApi = ctx.specifications
const origRelevant = specsApi.getRelevantTestSpecifications.bind(specsApi)
specsApi.getRelevantTestSpecifications = async (f) => {
  r.on = false // the glob's directory walk is vitest's choice of files, decided here exactly
  const all = await origRelevant(f)
  r.on = true
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
const moduleKeys = (mods) => {
  const w = createRecorder({ stateDir, mode: 'worker' })
  for (const m of mods) w.add('f', m)
  return w.keys
}
const transformed = new Set()
const dumps = new Map()
for (const id of ran.keys()) {
  let d = null
  try {
    d = JSON.parse(real.readFileSync(path.join(dumpDir, `${id}.json`), 'utf8'))
  } catch {}
  dumps.set(id, d)
  if (d) for (const m of d.modules) transformed.add(path.resolve(m))
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
  if (d) {
    const ks = new Set(d.keys)
    for (const k of moduleKeys(d.modules)) ks.add(k)
    keys = [...ks]
    unobservable = d.unobservable
    const root = ctx.config.root + '/'
    const ws = d.writes.filter((w) => w.startsWith(root) && !w.includes('/node_modules/'))
    if (ws.length) unobservable = [...unobservable, `writes the workspace (${path.relative(ctx.config.root, ws[0])})`]
  }
  const resultRel = `results/${id}.json`
  if (state === 'pass' || state === 'skip') real.writeFileSync(path.join(stateDir, resultRel), strip(file))
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
  ranOut.push({ id, project: file.projectName ?? '', file: file.filepath, state, duration: file.result?.duration ?? 0, keys, unobservable, result: resultRel })
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
const stack = [real.realpathSync(vitestDir)]
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
  auditMismatch,
  timings: { init: tInit - t0, run: tRun - tInit, record: performance.now() - tRun },
})
log(`vitest init ${(tInit - t0).toFixed(0)} ms, run ${(tRun - tInit).toFixed(0)} ms, record ${(performance.now() - tRun).toFixed(0)} ms`)
process.exit(process.exitCode ?? 0)
