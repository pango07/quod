// Run only in the GitHub gate. No reusable cache is written by this probe.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import net from 'node:net'
import {createRequire} from 'node:module'
import {fileURLToPath, pathToFileURL} from 'node:url'
import {spawn, spawnSync} from 'node:child_process'
import {build} from 'esbuild'

const [projectArg, outArg] = process.argv.slice(2)
const project = path.resolve(projectArg), out = path.resolve(outArg)
assert(!fs.existsSync(out)); fs.mkdirSync(out, {recursive: true})
// Vitest resolves project patterns and CLI filters relative to cwd, just as
// its ordinary CLI does. Setting root alone does not reproduce that context.
process.chdir(project)
const here = path.dirname(fileURLToPath(import.meta.url)), require = createRequire(import.meta.url)
const fromProject = createRequire(path.join(project, 'package.json'))
const workerd = path.join(path.dirname(require.resolve('workerd/package.json')), 'bin/workerd')
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const write = (name, value) => fs.writeFileSync(path.join(out, name), JSON.stringify(value, null, 2) + '\n')
const base = {epoch: 1700000000000, seed: 42, env: {NODE_ENV: 'test'}}
const results = {scope: 'Managed runtime boundary and unchanged-test feasibility; no cache', cacheEnabled: false,
  node: process.version, image: `${process.env.ImageOS}/${process.env.ImageVersion}`,
  workerdSha256: sha(fs.readFileSync(workerd)), phases: []}
write('result.json', results)

async function freePort() {
  const server = net.createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise(resolve => server.close(resolve))
  return port
}
async function execute(name, bundle) {
  const dir = path.join(out, name); fs.mkdirSync(dir)
  fs.writeFileSync(path.join(dir, 'worker.mjs'), bundle)
  fs.writeFileSync(path.join(dir, 'deny.mjs'), `export default {fetch(){console.error('QUOD_DENIED_OUTBOUND');throw new Error('outbound denied')}};\n`)
  const port = await freePort()
  fs.writeFileSync(path.join(dir, 'config.capnp'), `using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
  services = [
    (name = "main", worker = (modules = [(name = "worker.mjs", esModule = embed "worker.mjs")],
      compatibilityDate = "2026-07-01", compatibilityFlags = ["nodejs_als"], globalOutbound = "deny")),
    (name = "deny", worker = (modules = [(name = "deny.mjs", esModule = embed "deny.mjs")],
      compatibilityDate = "2026-07-01", globalOutbound = "deny"))
  ],
  sockets = [(name = "http", address = "127.0.0.1:${port}", http = (), service = "main")]
);\n`)
  const log = fs.openSync(path.join(dir, 'runtime.log'), 'w'), started = performance.now()
  const child = spawn(workerd, ['serve', path.join(dir, 'config.capnp')], {
    stdio: ['ignore', log, log], env: {PATH: process.env.PATH, TZ: 'UTC', LANG: 'C.UTF-8'},
  })
  let exit
  const exited = new Promise(resolve => child.once('exit', (code, signal) => { exit = {code, signal}; resolve(exit) }))
  try {
    let ready = false
    for (let i = 0; i < 80 && !exit; i++) {
      try { const r = await fetch(`http://127.0.0.1:${port}/health`, {signal: AbortSignal.timeout(500)}); ready = r.ok } catch {}
      if (ready) break
      await new Promise(resolve => setTimeout(resolve, 100))
    }
    assert(ready, 'workerd did not become ready; inspect runtime.log')
    const response = await fetch(`http://127.0.0.1:${port}/run`, {signal: AbortSignal.timeout(180000)})
    const text = await response.text(); fs.writeFileSync(path.join(dir, 'response.json'), text)
    assert(response.ok, `workerd returned HTTP ${response.status}`)
    const data = JSON.parse(text)
    const row = {name, seconds: (performance.now() - started) / 1000, responseSha256: sha(text), data}
    results.phases.push(row); write('result.json', results)
    console.log(JSON.stringify({phase: name, seconds: row.seconds, ok: data.ok, unsupported: data.boundary?.unsupported}))
    return data
  } finally {
    if (!exit) child.kill('SIGTERM')
    await exited; fs.closeSync(log)
  }
}
const globs = []
async function compile(entry, options, testFile) {
  const ts = testFile ? fromProject('typescript') : null
  const start = performance.now()
  const compiled = await build({
    entryPoints: [path.join(here, entry)], absWorkingDir: project, bundle: true,
    write: false, platform: 'browser', format: 'esm', target: 'es2022', metafile: true,
    define: {__QUOD_CONFIG__: JSON.stringify(options)},
    plugins: [{name: 'managed-module-boundary', setup(builder) {
      builder.onResolve({filter: /^__QUOD_TEST_ENTRY__$/}, () => ({path: testFile}))
      builder.onResolve({filter: /^quod:capabilities$/}, () => ({path: path.join(here, 'prelude.mjs')}))
      builder.onResolve({filter: /^node:/}, args => {
        if (args.path === 'node:async_hooks') return {path: args.path, external: true}
        throw new Error('unsupported native module: ' + args.path)
      })
      builder.onResolve({filter: /^(vitest(?:\/.*)?|vite\/module-runner|chai|tinybench|expect-type)$/}, args => ({path:
        args.path === 'vitest'
          ? path.join(path.dirname(fromProject.resolve('vitest/package.json')), 'dist/index.js')
          : fromProject.resolve(args.path)}))
      builder.onLoad({filter: /\.[cm]?[jt]sx?$/}, args => {
        if (!ts || !args.path.startsWith(project + '/')) return
        let contents = fs.readFileSync(args.path, 'utf8')
        if (!contents.includes('import.meta.glob')) return
        const tree = ts.createSourceFile(args.path, contents, ts.ScriptTarget.Latest, true)
        const edits = []
        function visit(node) {
          if (ts.isCallExpression(node) && node.expression.getText(tree) === 'import.meta.glob') {
            assert.equal(node.arguments.length, 1, 'glob options unsupported')
            assert(ts.isStringLiteral(node.arguments[0]), 'only literal glob is supported')
            const pattern = node.arguments[0].text, dir = path.dirname(args.path)
            const names = [...fs.globSync(pattern, {cwd: dir})].map(p => './' + p.replaceAll('\\', '/')).sort()
            globs.push({source: path.relative(project, args.path), pattern, names})
            const replacement = '{' + names.map(name => `${JSON.stringify(name)}:()=>__quodDeny(${JSON.stringify('lazy module ' + name)})`).join(',') + '}'
            edits.push({start: node.getStart(tree), end: node.getEnd(), replacement})
          }
          ts.forEachChild(node, visit)
        }
        visit(tree)
        for (const edit of edits.sort((a, b) => b.start - a.start)) contents = contents.slice(0, edit.start) + edit.replacement + contents.slice(edit.end)
        if (edits.length) contents = `import {deny as __quodDeny} from 'quod:capabilities';\n` + contents
        return {contents, loader: /\.tsx?$/.test(args.path) ? 'ts' : 'js'}
      })
    }}],
  })
  const bundle = compiled.outputFiles[0].text
  const imports = Object.values(compiled.metafile.outputs).flatMap(o => o.imports)
  assert(imports.every(i => i.path === 'node:async_hooks'), 'bundle is not closed')
  return {bundle, metadata: compiled.metafile, seconds: (performance.now() - start) / 1000, sha256: sha(bundle)}
}

try {
  const fixture = await compile('preflight.mjs', base)
  write('preflight-build.json', {...fixture, bundle: undefined})
  const one = await execute('preflight-one', fixture.bundle)
  const two = await execute('preflight-two', fixture.bundle)
  assert(one.ok && two.ok, 'managed capability preflight failed')
  assert.deepEqual(one.random, two.random)
  assert.deepEqual(one.checks, two.checks)
  results.preflightPassed = true; write('result.json', results)

  const {createVitest} = await import(pathToFileURL(fromProject.resolve('vitest/node')).href)
  const ctx = await createVitest({root: project, watch: false, run: true, reporters: []})
  const file = 'packages/convex-helpers/server/stream.property.test.ts'
  let config
  try {
    const specs = await ctx.globTestSpecifications([path.join(project, file)])
    write('project-selection.json', {projects: ctx.projects.map(p => ({name: p.config.name, root: p.config.root})),
      selected: specs.map(s => ({file: s.moduleId, project: s.project.config.name}))})
    assert.equal(specs.length, 1, 'expected one actual Vitest specification for the expensive test')
    const p = specs[0].project
    assert.equal(p.config.root, path.join(project, 'packages/convex-helpers'))
    assert(!p.config.name?.startsWith('zod-'), 'a Zod-only project must not select this test')
    assert.equal(p.serializedConfig.setupFiles.length, 0)
    assert.equal(p.serializedConfig.snapshotSerializers.length, 0)
    assert(!p.serializedConfig.snapshotEnvironment)
    assert(!fs.existsSync(path.join(project, 'packages/convex-helpers/server/__snapshots__/stream.property.test.ts.snap')))
    const encoded = JSON.stringify(p.serializedConfig, (_key, value) => {
      assert(typeof value !== 'function', 'function-valued configuration unsupported')
      assert(!(value instanceof RegExp), 'regexp configuration needs explicit serialization')
      return value
    })
    config = JSON.parse(encoded.replaceAll(project, '/project'))
    // A declared managed shuffle seed replaces the ambient default Date.now seed.
    config.sequence.seed = base.seed
  } finally { await ctx.close() }
  const testFile = path.join(project, file)
  results.testSourceSha256 = sha(fs.readFileSync(testFile))
  const options = {...base, vitest: config, file: '/project/' + file}
  const compiled = await compile('runner.mjs', options, testFile)
  write('test-build.json', {...compiled, bundle: undefined, globs, options})
  const managed = await execute('managed', compiled.bundle)
  results.managedSupported = !managed.error && managed.boundary.unsupported.length === 0 && managed.boundary.unguardable.length === 0 && managed.boundary.timerErrors.length === 0
  results.managedPassed = Boolean(managed.ok)
  results.managedRows = managed.rows || []
} catch (error) {
  results.unsupportedOrSetupError = {name: error?.name, message: error?.message, stack: error?.stack}
  results.managedSupported = false
} finally {
  // The ordinary run is mandatory and is the fallback for unsupported attempts.
  const report = path.join(out, 'ordinary-report.json'), log = fs.openSync(path.join(out, 'ordinary.log'), 'w')
  const started = performance.now()
  const plain = spawnSync(process.execPath, ['node_modules/vitest/vitest.mjs', 'run',
    'packages/convex-helpers/server/stream.property.test.ts', '--reporter=json', '--outputFile.json=' + report],
    {cwd: project, env: process.env, stdio: ['ignore', log, log], timeout: 180000})
  fs.closeSync(log)
  results.ordinary = {exit: plain.status, seconds: (performance.now() - started) / 1000, error: plain.error?.message}
  if (fs.existsSync(report)) {
    const bytes = fs.readFileSync(report), data = JSON.parse(bytes)
    const rows = data.testResults.flatMap(f => f.assertionResults.map(a => [a.fullName, a.status])).sort()
    results.ordinary.reportSha256 = sha(bytes); results.ordinary.rows = rows
    if (results.managedRows) {
      const states = {pass: 'passed', fail: 'failed', skip: 'skipped', todo: 'todo'}
      const managedRows = results.managedRows.map(r => [r.name, states[r.state] || r.state]).sort()
      results.namesAndStatesAgree = JSON.stringify(rows) === JSON.stringify(managedRows)
    }
  }
  results.fallbackExecuted = !results.managedSupported
  write('result.json', results)
  console.log(JSON.stringify({...results, phases: results.phases.map(p => ({name: p.name, seconds: p.seconds, ok: p.data.ok})), managedRows: undefined, ordinary: {...results.ordinary, rows: undefined}}))
  assert.equal(results.ordinary.exit, 0, 'ordinary control failed')
  assert.equal(results.ordinary.rows?.length, 47, 'ordinary test count changed')
  assert.equal(results.preflightPassed, true, 'capability gate failed; no managed eligibility')
  if (results.managedSupported) assert.equal(results.namesAndStatesAgree, true, 'managed states differ')
}
