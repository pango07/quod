// GitHub-only, unwired diagnostic. Never admits a receipt or executes cached code.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {createRequire} from 'node:module'
import {fileURLToPath} from 'node:url'
import {spawnSync} from 'node:child_process'
import {canonicalManagedJSON as json, createCertifiedManagedCompiler, compileAndCaptureCertifiedBatch,
  captureResolvedCompileCertificate, validateResolvedCompileCertificates,
  readValidatedCompileIdentity} from './resolved-compile-certificate.mjs'

assert.equal(process.env.GITHUB_ACTIONS, 'true', 'remote-only batch diagnostic')
process.env.TEST = 'true'; process.env.VITEST = 'true'; process.env.NODE_ENV ??= 'test'
const self = fileURLToPath(import.meta.url), here = path.dirname(self)
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const write = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', {flag: 'wx'})
const errorData = error => ({name: String(error.name), message: String(error.message),
  stack: String(error.stack), batchFenceTrace: error.batchFenceTrace || [],
  publicationFields: ['results', 'built', 'certificate', 'proof'].filter(key => Object.hasOwn(error, key))})
const batchArgs = (project, requests) => ({project, adapterRoot: here, requestsJSON: json(requests)})
const fenceTrace = count => ['before-constructor', 'after-constructor/before-compile:0',
  ...Array.from({length: count}, (_, index) => ['after-compile:' + index + '/before-capture:' + index,
    index === count - 1 ? 'after-capture:' + index + '/before-publication'
      : 'after-capture:' + index + '/before-compile:' + (index + 1)]).flat()]
const validationRequest = (certificate, request) => ({certificateBytes: certificate.bytes,
  trustedCertificateSha256: certificate.sha256, options: request.options, testFile: request.testFile})
async function rejection(operation, contains) {
  let error
  try {await operation()} catch (caught) {error = errorData(caught)}
  assert(error && (!contains || error.message.includes(contains)), 'expected specific rejection: ' + contains)
  return error
}

// Every mutation/reservation case gets a new ordinary Node process. Its request
// is trusted controller data; no test source or retained bundle is imported.
async function childCase(requestFile) {
  const {project, good, bad, mode, out} = read(requestFile)
  assert(!fs.existsSync(out)); fs.mkdirSync(out)
  const result = {mode, passed: false, checks: [], publishedResults: 0}
  const check = (name, pass) => {result.checks.push({name, pass: Boolean(pass)}); assert(pass, name)}
  const batch = requests => compileAndCaptureCertifiedBatch(batchArgs(project, requests))
  let compile
  const build = () => compile(good.entry, good.options, good.testFile)
  const capture = built => captureResolvedCompileCertificate({project, built, ...good})
  const validate = seed => validateResolvedCompileCertificates({project, requests: [validationRequest(seed, good)]})
  let restore = () => {}
  try {
    check('project module was not evaluated before trusted compiler work', globalThis.__quodBatchModuleEvaluations === undefined)
    if (mode === 'cold-batch') {
      const output = await batch([good]); result.publishedResults = output.results.length
      check('batch can initialize the strictly pinned cold compiler epoch', output.results.length === 1 && output.diagnostics.fullFences === 4)
      compile = await createCertifiedManagedCompiler({project})
      check('cold batch build has actual private provenance', capture(output.results[0].built).bytes.equals(output.results[0].certificate.bytes))
    } else if (mode === 'cold-prior-initialization') {
      const operation = createCertifiedManagedCompiler({project})
      const declined = rejection(() => batch([good]), 'previous certified operation in flight')
      compile = await operation; await declined
      check('an already initializing public factory excludes the batch', true)
      const output = await batch([good]); result.publishedResults = output.results.length
      check('declined overlap does not poison completed public initialization', output.results.length === 1)
    } else if (mode === 'cold-unknown-tools') {
      createRequire(import.meta.url)('esbuild')
      result.failure = await rejection(() => batch([good]), 'compiler tools already loaded outside certified epoch')
      check('unreviewed prior tool load cannot initialize a batch epoch', result.publishedResults === 0)
      await rejection(() => createCertifiedManagedCompiler({project}), 'fresh process')
      check('failed cold reservation is poisoned', true)
    } else {
    compile = await createCertifiedManagedCompiler({project})
    if (mode === 'malformed') {
      let getterCalls = 0
      const accessor = {project, adapterRoot: here}
      Object.defineProperty(accessor, 'requestsJSON', {enumerable: true, get() {getterCalls++; return json([good])}})
      const valid = batchArgs(project, [good])
      const malformed = [accessor, {...valid, callback() {}}, {...valid, requestsJSON: '[ ' + json(good) + ']'},
        {...valid, requestsJSON: '[]'}, {...valid, requestsJSON: json(Array(513).fill(good))},
        {...valid, requestsJSON: ' '.repeat(16 * 1024 * 1024 + 1)},
        {...valid, requestsJSON: json([{...good, entry: 'preflight.mjs'}])},
        {...valid, requestsJSON: json([{...good, testFile: self}])},
        {...valid, requestsJSON: json([{...good, callback: 'not-a-function'}])}]
      for (const [index, value] of malformed.entries()) {
        await rejection(() => compileAndCaptureCertifiedBatch(value)); check('malformed request ' + index + ' rejects', true)
      }
      check('accessor is never invoked', getterCalls === 0)
      const output = await batch([good]); result.publishedResults = output.results.length
      check('pre-reservation declines do not poison valid batch', output.results.length === 1)
    } else if (mode.startsWith('prior-')) {
      let operation, built, seed
      if (mode !== 'prior-factory' && mode !== 'prior-compile') {built = await build(); seed = capture(built)}
      if (mode === 'prior-factory') operation = createCertifiedManagedCompiler({project})
      else if (mode === 'prior-compile') operation = build()
      else if (mode === 'prior-validation') operation = validate(seed)
      else if (mode === 'prior-capture') {
        let pending
        const argumentsValue = {project, built, ...good}
        Object.defineProperty(argumentsValue, 'testFile', {enumerable: true, get() {
          pending = rejection(() => batch([good]), 'previous certified operation in flight'); return good.testFile
        }})
        captureResolvedCompileCertificate(argumentsValue)
        await pending
      } else assert.fail('unknown prior-operation case')
      if (operation) {
        const declined = rejection(() => batch([good]), 'previous certified operation in flight')
        await operation; await declined
      }
      check('batch rejects an already active public ' + mode, true)
      const output = await batch([good]); result.publishedResults = output.results.length
      check('public operation finishes and rejected reservation does not poison', output.results.length === 1)
    } else if (mode === 'reentry') {
      const built = await build(), seed = capture(built), decisions = await validate(seed)
      assert(decisions[0].hit)
      const operation = batch([good])
      const attempts = [() => createCertifiedManagedCompiler({project}), () => build(), () => capture(built),
        () => validate(seed), () => readValidatedCompileIdentity(decisions[0].proof)]
      const declined = attempts.map(attempt => rejection(attempt, 'certified batch is reserved'))
      await Promise.all(declined)
      for (const index of attempts.keys()) check('reserved batch blocks public API ' + index, true)
      const output = await operation; result.publishedResults = output.results.length
      check('declined public reentry leaves original batch valid', output.results.length === 1)
    } else if (mode === 'concurrent-batch') {
      const operation = batch([good]), declined = rejection(() => batch([good]), 'previous certified operation in flight')
      const output = await operation; await declined; result.publishedResults = output.results.length
      check('second batch cannot overlap first reservation', output.results.length === 1)
    } else if (mode === 'source-resolution-glob') {
      const output = await batch([good]), item = output.results[0]
      const seed = item.certificate
      check('batch certificate actually validates before mutation', (await validate(seed))[0].hit)
      const directory = path.dirname(good.testFile), dep = path.join(directory, 'dep.ts'), original = fs.readFileSync(dep)
      const mutations = [
        ['source bytes', () => fs.writeFileSync(dep, 'export const value=8;\n'), () => fs.writeFileSync(dep, original)],
        ['higher-priority resolution', () => fs.writeFileSync(path.join(directory, 'dep.js'), 'export const value=9;\n'),
          () => fs.rmSync(path.join(directory, 'dep.js'))],
        ['lazy glob names', () => fs.writeFileSync(path.join(directory, 'extra.test.ts'), 'export const extra=1;\n'),
          () => fs.rmSync(path.join(directory, 'extra.test.ts'))],
      ]
      for (const [name, change, undo] of mutations) {
        try {
          change()
          const decision = (await validate(seed))[0]
          check(name + ' invalidates batch certificate', decision.hit === false && decision.next === 'full-compile')
          const actual = await build()
          check(name + ' fresh actual compile identity changes', actual.identitySha256 !== item.built.identitySha256)
          fs.writeFileSync(path.join(out, name.replaceAll(' ', '-') + '.mjs'), actual.bundle, {flag: 'wx'})
          write(path.join(out, name.replaceAll(' ', '-') + '-build.json'), {...actual, bundle: undefined})
          write(path.join(out, name.replaceAll(' ', '-') + '-decision.json'), decision)
        } finally {undo()}
        check(name + ' exact restoration recovers current proof', (await validate(seed))[0].hit)
      }
      result.publishedResults = 1
    } else {
      const issuedBuilt = await build(), issuedSeed = capture(issuedBuilt)
      const issuedProof = (await validate(issuedSeed))[0].proof
      const require = createRequire(import.meta.url)
      let mutate = () => {}
      if (mode.endsWith('environment')) {
        const previous = process.env.NODE_ENV
        mutate = () => {process.env.NODE_ENV = 'production'}
        restore = () => {if (previous === undefined) delete process.env.NODE_ENV; else process.env.NODE_ENV = previous}
      } else if (mode.endsWith('api')) {
        const file = createRequire(path.join(project, 'package.json')).resolve('typescript')
        const module = require.cache[fs.realpathSync(file)], previous = module.exports
        mutate = () => {module.exports = {...previous, createSourceFile() {throw new Error('UNTRUSTED_API')}}}
        restore = () => {module.exports = previous}
      } else if (mode === 'during-unknown-cache') {
        const key = path.join(project, '.quod-batch-unloaded-module.cjs')
        assert(!require.cache[key])
        mutate = () => {require.cache[key] = {exports: {unreviewed: true}}}
        restore = () => {delete require.cache[key]}
      } else if (mode === 'before-tool-bytes') {
        const file = require.resolve('esbuild'), bytes = fs.readFileSync(file)
        mutate = () => fs.writeFileSync(file, Buffer.concat([bytes, Buffer.from('\n// QUOD_BATCH_TOOL_MUTANT\n')]))
        restore = () => fs.writeFileSync(file, bytes)
      } else assert.equal(mode, 'late-compile-failure')
      let operation
      if (mode.startsWith('before-')) {mutate(); operation = batch([good])}
      else {
        operation = batch(mode === 'late-compile-failure' ? [good, bad] : [good])
        if (mode.startsWith('during-')) queueMicrotask(mutate)
      }
      const expectedReason = mode.endsWith('environment') ? 'compiler tool epoch changed'
        : mode.endsWith('api') ? 'loaded compiler exports changed'
          : mode === 'during-unknown-cache' ? 'batch loaded an unknown or replaced cached module'
            : mode === 'before-tool-bytes' ? 'unreviewed compiler tool file inventory'
              : 'only a named AsyncLocalStorage import is supported: node:fs'
      result.failure = await rejection(() => operation, expectedReason)
      restore(); restore = () => {}
      check('failed batch exposes no build/certificate/proof result', result.failure.publicationFields.length === 0)
      if (mode === 'late-compile-failure') check('late actual compile fails after first captured member',
        result.failure.batchFenceTrace.includes('after-capture:0/before-compile:1') &&
        !result.failure.batchFenceTrace.some(label => label.endsWith('/before-publication')))
      if (mode.startsWith('during-')) check('mutation is observed after actual reserved work starts',
        result.failure.batchFenceTrace.includes('after-constructor/before-compile:0'))
      for (const [index, operation] of [() => createCertifiedManagedCompiler({project}), () => build(),
        () => capture(issuedBuilt), () => validate(issuedSeed), () => readValidatedCompileIdentity(issuedProof),
        () => batch([good])].entries()) {
        await rejection(operation, index === 5 ? 'unpoisoned process' : 'fresh process')
        check('reserved failure poisons later certified API ' + index, true)
      }
    }
    }
    check('actual project test source remains unevaluated in trusted compiler process', globalThis.__quodBatchModuleEvaluations === undefined)
    result.passed = true
  } catch (error) {result.error = errorData(error); process.exitCode = 1}
  finally {restore(); write(path.join(out, 'result.json'), result)}
}

function savePair(out, name, item, options, preparation) {
  const directory = path.join(out, name); fs.mkdirSync(directory)
  fs.writeFileSync(path.join(directory, 'worker.mjs'), item.built.bundle, {flag: 'wx'})
  fs.writeFileSync(path.join(directory, 'compile-certificate.json'), item.certificate.bytes, {flag: 'wx'})
  write(path.join(directory, 'test-build.json'), {...item.built, bundle: undefined, options, preparation})
  return directory
}
async function probe(projectArg, matrixArg, outArg, referenceArg) {
  assert(projectArg && matrixArg && outArg && referenceArg,
    'usage: batch-compile-certificate-probe.mjs PROJECT CURRENT_MATRIX EMPTY_OUTPUT VERIFIED_ORIGINAL_MATRIX')
  const project = fs.realpathSync(projectArg), matrix = fs.realpathSync(matrixArg), out = path.resolve(outArg)
  const reference = fs.realpathSync(referenceArg)
  assert(!fs.existsSync(out)); fs.mkdirSync(out)
  const result = {schema: 'quod-managed-certified-batch-probe-v1-diagnostic', passed: false,
    sourcePin: process.env.GITHUB_SHA, node: process.version, execArgv: process.execArgv,
    sourceFiles: ['compiler.mjs', 'resolved-compile-certificate.mjs', path.basename(self)].map(name =>
      ({name, sha256: sha(fs.readFileSync(path.join(here, name)))})),
    productionEnabled: false, receiptAdmission: false, dispatcherWired: false, checks: [], pairs: [], children: [],
    authority: 'actual current compilation and input-equivalence proof only',
    assumptions: 'trusted bootstrap/Node/builtin/compiler TCB; sampled nonadversarial workspace; no active TOCTOU claim',
    timingLimit: 'same-host fixed-order diagnostic; no full-job or window savings claim'}
  const save = () => fs.writeFileSync(path.join(out, 'result.json'), JSON.stringify(result, null, 2) + '\n')
  const check = (name, pass) => {result.checks.push({name, pass: Boolean(pass)}); save(); assert(pass, name)}
  const started = performance.now(), fixture = path.join(project, '.quod-batch-fixture')
  let fixtureCreated = false
  save()
  try {
    const strict = await createCertifiedManagedCompiler({project})
    const {prepareManagedMatrix} = await import('./preparation.mjs')
    const {makeMatrixExecutionIdentity, makeMatrixExecutionIdentityFromProof} = await import('./matrix-receipt.mjs')
    const suite = await prepareManagedMatrix({project, seed: 42}), previous = read(path.join(matrix, 'result.json'))
    check('current original native120 and managed108 matrix passed', previous.passed === true && previous.selected === 120 &&
      previous.coverage.nativeAssertions === 4131 && previous.coverage.managedPassing === 108 && previous.coverage.disagreements === 0)
    const nativeBytes = fs.readFileSync(path.join(matrix, 'native-selection.json'))
    const referenceBytes = fs.readFileSync(path.join(reference, 'native-selection.json'))
    const native = JSON.parse(nativeBytes), original = JSON.parse(referenceBytes)
    const stateRows = value => value.executed.flatMap(file => file.rows.map(row =>
      json([file.project, file.file, file.pool, row.name, row.state]))).sort()
    const fileRows = value => value.executed.map(file =>
      json([file.project, file.file, file.pool, file.relativeModuleId, file.state])).sort()
    check('all4131 actual native original project/name/states remain unchanged',
      native.completed === true && native.exitCode === 0 && native.executed.length === 120 &&
      json(stateRows(native)) === json(stateRows(original)))
    check('all120 actual native API file states and repeated projects remain unchanged', json(fileRows(native)) === json(fileRows(original)))
    result.nativeComparison = {currentSelectionSha256: sha(nativeBytes), originalSelectionSha256: sha(referenceBytes),
      projectStateSha256: sha(json(stateRows(native))), projectFileStateSha256: sha(json(fileRows(native)))}
    const prepared = new Map(suite.prepared.map(row => [row.instanceId, row]))
    const candidates = previous.instances.filter(row => row.decision === 'managed').map(row => {
      const candidate = prepared.get(row.instanceId); assert(candidate)
      const options = {epoch: 1700000000000, seed: 42, env: {NODE_ENV: 'test'}, vitest: candidate.config,
        file: '/project/' + candidate.file, providedContext: candidate.providedContext,
        ...(candidate.zodAlias ? {zodAlias: candidate.zodAlias} : {})}
      const actual = read(path.join(matrix, row.evidenceDirectory, 'test-build.json'))
      assert.equal(json(options), json(actual.options)); assert.equal(json(candidate.identity), json(actual.preparation))
      return {row, candidate, options, actual, request: {entry: 'runner.mjs', options, testFile: candidate.absolute}}
    })
    check('all actual project/file identities survive new preparation', suite.selected.length === 120 && candidates.length === 108 &&
      new Set(candidates.map(value => value.row.instanceId)).size === 108)
    const roots = new Set(['standardSchema.test.ts', 'server/stream.property.test.ts', 'server/stream.test.ts',
      'server/zod4.functions.test.ts', 'server/stream.distinct.test.ts'])
    const chosen = candidates.filter(({candidate}) => candidate.project === 'convex-helpers'
      ? roots.has(candidate.file.replace('packages/convex-helpers/', ''))
      : /^zod-4\.[0-6]$/.test(candidate.project) && candidate.file === 'packages/convex-helpers/standardSchema.test.ts')
    assert.equal(chosen.length, 12)
    const strictPairs = [], strictStart = performance.now()
    for (const item of chosen) {
      const built = await strict('runner.mjs', item.options, item.candidate.absolute)
      const certificate = captureResolvedCompileCertificate({project, built, ...item.request})
      strictPairs.push({built, certificate})
      savePair(out, 'strict-' + item.row.instanceId, {built, certificate}, item.options, item.candidate.identity)
    }
    result.strict12Seconds = (performance.now() - strictStart) / 1000; save()
    const twelveStart = performance.now(), twelve = await compileAndCaptureCertifiedBatch(batchArgs(project, chosen.map(row => row.request)))
    result.batch12Seconds = (performance.now() - twelveStart) / 1000; result.batch12Diagnostics = twelve.diagnostics
    check('batch12 retains all26 constructor and adjacent operation full fences', twelve.results.length === 12 &&
      twelve.diagnostics.fullFences === 26 && json(twelve.diagnostics.trace) === json(fenceTrace(12)))
    for (const [index, item] of twelve.results.entries()) {
      const expected = strictPairs[index]
      check('strict versus batch12 full raw pair ' + chosen[index].row.instanceId,
        item.built.bundle === expected.built.bundle && json(item.built.identity) === json(expected.built.identity) &&
        item.built.identitySha256 === expected.built.identitySha256 && item.certificate.bytes.equals(expected.certificate.bytes))
      savePair(out, 'batch12-' + chosen[index].row.instanceId, item, chosen[index].options, chosen[index].candidate.identity)
    }
    const fullStart = performance.now(), full = await compileAndCaptureCertifiedBatch(batchArgs(project, candidates.map(row => row.request)))
    result.batch108Seconds = (performance.now() - fullStart) / 1000; result.batch108Diagnostics = full.diagnostics; save()
    check('batch108 retains all218 constructor and adjacent operation full fences', full.results.length === 108 &&
      full.diagnostics.fullFences === 218 && json(full.diagnostics.trace) === json(fenceTrace(108)))
    const validations = [], directories = []
    for (const [index, item] of full.results.entries()) {
      const expected = candidates[index]
      check('current matrix versus batch108 raw bundle and full identity ' + expected.row.instanceId,
        fs.readFileSync(path.join(matrix, expected.row.evidenceDirectory, 'managed/worker.mjs')).equals(Buffer.from(item.built.bundle)) &&
        json(expected.actual.identity) === json(item.built.identity) && expected.actual.identitySha256 === item.built.identitySha256)
      // Actual strict capture requires private issued-build provenance. A copy
      // of these diagnostic fields is explicitly insufficient.
      const captured = captureResolvedCompileCertificate({project, built: item.built, ...expected.request})
      check('batch privately issued actual build recaptures exact strict certificate ' + index, captured.bytes.equals(item.certificate.bytes))
      await rejection(() => captureResolvedCompileCertificate({project, built: {...item.built}, ...expected.request}), 'actual certified compiler build required')
      item.built.options = expected.options; item.built.preparation = expected.candidate.identity
      directories.push(savePair(out, 'batch108-' + expected.row.instanceId, item, expected.options, expected.candidate.identity))
      validations.push(validationRequest(item.certificate, expected.request))
    }
    const validationStart = performance.now(), decisions = await validateResolvedCompileCertificates({project, requests: validations})
    result.validation108Seconds = (performance.now() - validationStart) / 1000
    const preflightBuild = read(path.join(matrix, 'preflight-build.json')), preflight = read(path.join(matrix, 'preflight-one/response.json'))
    const fingerprint = sha(json({preflightIdentity: preflightBuild.identitySha256,
      inventory: preflight.boundary.inventory, symbols: preflight.boundary.symbolInventory}))
    const adapterSha256 = sha(fs.readFileSync(path.join(here, 'matrix-native.mjs')))
    for (const [index, decision] of decisions.entries()) {
      const item = full.results[index], expected = candidates[index]
      check('current complete resolver and opaque proof validate batch108 member ' + index, decision.hit === true &&
        json(readValidatedCompileIdentity(decision.proof).identity) === json(item.built.identity))
      const runtimePolicy = {...previous.runtimePolicy, nativeFingerprintSha256: fingerprint,
        supervisor: {...previous.runtimePolicy.supervisor,
          watchdogMs: expected.candidate.file.endsWith('/stream.property.test.ts') ? 180000 : 30000}}
      const policy = {runtimePolicy, host: previous.host,
        supervisorPolicy: {receiptValidatorSha256: sha(fs.readFileSync(path.join(here, 'matrix-receipt.mjs'))),
          experimentSha256: sha(fs.readFileSync(self)), conformanceAdapterSha256: adapterSha256,
          candidateSelectorSha256: sha(fs.readFileSync(path.join(here, 'matrix-candidates.mjs')))},
        nativePolicy: {adapterSha256, nodeExecutableSha256: runtimePolicy.supervisor.nodeExecutableSha256,
          nodeVersion: process.version, vitestVersion: '5.0.0', host: previous.host}}
      const actual = makeMatrixExecutionIdentity({built: item.built, prepared: expected.candidate, ...policy})
      const fromProof = await makeMatrixExecutionIdentityFromProof({proof: decision.proof, options: expected.options,
        prepared: expected.candidate, ...policy})
      check('batch build and current opaque proof give identical whole execution key ' + index, json(actual) === json(fromProof))
      write(path.join(directories[index], 'validated.json'), decision)
      write(path.join(directories[index], 'execution-identity.json'), actual)
      result.pairs.push({instanceId: expected.row.instanceId, file: expected.candidate.file, project: expected.candidate.project,
        bundleSha256: item.built.sha256, identitySha256: item.built.identitySha256,
        certificateSha256: item.certificate.sha256, executionKey: actual.key})
    }
    assert(!fs.existsSync(fixture)); fs.mkdirSync(fixture); fixtureCreated = true
    fs.writeFileSync(path.join(fixture, 'dep.ts'), 'export const value=7;\n')
    fs.writeFileSync(path.join(fixture, 'fixture.test.ts'), 'import {test,expect} from "vitest"; import {value} from "./dep.js"; globalThis.__quodBatchModuleEvaluations=(globalThis.__quodBatchModuleEvaluations||0)+1; const files=import.meta.glob("./*.test.ts"); test("batch real source",()=>{expect(value).toBe(7);expect(Object.keys(files).length).toBe(1)});\n')
    fs.writeFileSync(path.join(fixture, 'bad.test.ts'), 'import {readFileSync} from "node:fs"; console.log(readFileSync("/no-host-input"));\n')
    // Keep the negative outside the literal glob: its unsupported import must
    // be reached only by the late second compile, not first-member scanning.
    fs.renameSync(path.join(fixture, 'bad.test.ts'), path.join(fixture, 'bad.ts'))
    const root = candidates.find(value => value.candidate.project === 'convex-helpers')
    const good = {entry: 'runner.mjs', options: {...root.options, file: '/project/.quod-batch-fixture/fixture.test.ts'},
      testFile: path.join(fixture, 'fixture.test.ts')}
    const bad = {entry: 'runner.mjs', options: {...root.options, file: '/project/.quod-batch-fixture/bad.ts'}, testFile: path.join(fixture, 'bad.ts')}
    for (const mode of ['cold-batch', 'cold-prior-initialization', 'cold-unknown-tools', 'malformed', 'prior-factory', 'prior-compile', 'prior-validation', 'prior-capture', 'reentry',
      'concurrent-batch', 'late-compile-failure', 'before-tool-bytes', 'before-api', 'before-environment',
      'during-api', 'during-environment', 'during-unknown-cache', 'source-resolution-glob']) {
      const requestFile = path.join(out, mode + '-request.json'), directory = path.join(out, mode)
      write(requestFile, {project, good, bad, mode, out: directory})
      const childStart = performance.now(), child = spawnSync(process.execPath, [self, '--case', requestFile],
        {cwd: project, env: process.env, timeout: 120000, killSignal: 'SIGKILL', maxBuffer: 1024 * 1024})
      fs.writeFileSync(path.join(out, mode + '-stdout.log'), child.stdout || Buffer.alloc(0))
      fs.writeFileSync(path.join(out, mode + '-stderr.log'), child.stderr || Buffer.alloc(0))
      const evidence = fs.existsSync(path.join(directory, 'result.json')) ? read(path.join(directory, 'result.json')) : null
      result.children.push({mode, status: child.status, signal: child.signal, error: child.error ? errorData(child.error) : null,
        seconds: (performance.now() - childStart) / 1000, passed: evidence?.passed === true})
      check('fresh-process boundary control ' + mode, !child.error && child.status === 0 && child.signal === null && evidence?.passed === true)
    }
    result.passed = true
  } catch (error) {result.error = errorData(error); process.exitCode = 1}
  finally {
    if (fixtureCreated) fs.rmSync(fixture, {recursive: true})
    result.seconds = (performance.now() - started) / 1000; save()
  }
  console.log(json({schema: result.schema, passed: result.passed, checks: result.checks.length,
    pairs: result.pairs.length, children: result.children.length, seconds: result.seconds}))
}

function inspectControls(out) {
  const expected = [['compiler', 55, 'passed'], ['resolved-targeted', 213, 'pass'], ['legacy-ledger', 152, 'passed'],
    ['dispatcher', 142, 'passed'], ['dispatcher/input-proof-controls.json', 18, 'pass']]
  const checked = expected.map(([relative, count, field]) => {
    const file = path.join(out, relative.endsWith('.json') ? relative : relative + '/result.json')
    const bytes = fs.readFileSync(file), value = JSON.parse(bytes)
    assert.equal(value.checks.length, count, relative + ' exact original check inventory')
    assert(value.checks.every(check => check[field] === true), relative + ' has a failed original check')
    if (relative !== 'compiler') assert.equal(value.passed, true)
    if (relative === 'dispatcher') assert.equal(value.caseCount, 13)
    return {file: path.relative(out, file), bytes: bytes.length, sha256: sha(bytes), checks: count}
  })
  write(path.join(out, 'original-controls.json'), {schema: 'quod-managed-batch-original-controls-v1', passed: true, checked})
}
function inspectBounds(out) {
  let paths = 0, bytes = 0
  const files = []
  function walk(file) {
    assert(++paths <= 19999, 'retained evidence total path cap exceeded; one path reserved for inventory')
    const stat = fs.lstatSync(file)
    if (stat.isDirectory()) for (const name of fs.readdirSync(file).sort()) walk(path.join(file, name))
    else {
      assert(stat.isFile() && stat.size <= 32 * 1024 * 1024, 'bounded regular evidence required')
      bytes += stat.size; assert(bytes <= 500000000 - 1024 * 1024, 'retained evidence expanded cap exceeded')
      files.push({name: path.relative(out, file).split(path.sep).join('/'), bytes: stat.size, sha256: sha(fs.readFileSync(file))})
    }
  }
  walk(out)
  const inventory = Buffer.from(JSON.stringify({schema: 'quod-managed-batch-evidence-inventory-v1',
    pathsBeforeInventory: paths, bytesBeforeInventory: bytes, inventoryReservedPaths: 1, inventoryReservedBytes: 1024 * 1024,
    maximumPaths: 20000, maximumExpandedBytes: 500000000, maximumFileBytes: 32 * 1024 * 1024, files}, null, 2) + '\n')
  assert(inventory.length <= 1024 * 1024, 'serialized evidence inventory exceeds its reserved byte budget')
  fs.writeFileSync(path.join(out, 'inventory.json'), inventory, {flag: 'wx'})
}
if (process.argv[2] === '--case') await childCase(process.argv[3])
else if (process.argv[2] === '--controls') inspectControls(path.resolve(process.argv[3]))
else if (process.argv[2] === '--bounds') inspectBounds(path.resolve(process.argv[3]))
else await probe(...process.argv.slice(2))
