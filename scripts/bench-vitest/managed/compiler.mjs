// Research compiler. Preparation always runs; this module grants no replay.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {builtinModules, createRequire, isBuiltin} from 'node:module'
import {fileURLToPath} from 'node:url'
import {build, version as esbuildVersion} from 'esbuild'

const here = path.dirname(fileURLToPath(import.meta.url))
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const reserved = 'quod:compiler-capabilities'
const nativeModules = new Set(builtinModules.map(name => name.replace(/^node:/, '')))
const isNative = name => name.startsWith('node:') || nativeModules.has(name) || isBuiltin(name)
const isALS = name => name === 'node:async_hooks' || name === 'async_hooks'
const privateSpecifier = name => name.startsWith('quod:') || name.startsWith('__QUOD_')
const inside = (root, file) => file === root || file.startsWith(root + path.sep)
const slash = value => value.replaceAll('\\', '/')

// Keys must be independent of insertion order. Values that JSON would silently
// omit, coerce or invoke (including toJSON) are not admitted configuration.
export function canonicalManagedJSON(value) {
  function visit(item) {
    if (item === null || typeof item === 'string' || typeof item === 'boolean') return item
    if (typeof item === 'number') {
      assert(Number.isFinite(item) && !Object.is(item, -0), 'non-canonical configuration number')
      return item
    }
    assert(typeof item === 'object', 'configuration must contain JSON values only')
    if (Array.isArray(item)) {
      assert.equal(Object.getOwnPropertySymbols(item).length, 0, 'symbol configuration keys unsupported')
      assert.equal(Object.getOwnPropertyNames(item).length, item.length + 1, 'sparse/custom arrays unsupported')
      const result = []
      Object.setPrototypeOf(result, null)
      for (let index = 0; index < item.length; index++) {
        const descriptor = Object.getOwnPropertyDescriptor(item, String(index))
        assert(descriptor?.enumerable && 'value' in descriptor, 'configuration array accessors unsupported')
        result[index] = visit(descriptor.value)
      }
      return result
    }
    assert(Object.getPrototypeOf(item) === Object.prototype || Object.getPrototypeOf(item) === null,
      'configuration must contain plain objects')
    assert.equal(Object.getOwnPropertySymbols(item).length, 0, 'symbol configuration keys unsupported')
    const result = Object.create(null)
    for (const key of Object.getOwnPropertyNames(item).sort()) {
      const descriptor = Object.getOwnPropertyDescriptor(item, key)
      assert(descriptor.enumerable && 'value' in descriptor, 'configuration accessors/non-enumerable keys unsupported')
      result[key] = visit(descriptor.value)
    }
    return result
  }
  return JSON.stringify(visit(value))
}

/**
 * The returned compile(entry, options, testFile?) replaces the original probe's
 * compile function. entry is one of the reviewed adapters in adapterRoot.
 * Result metadata retains esbuild's original paths for artifact inspection;
 * identity is the canonical, hashable preparation record.
 */
export function createManagedCompiler({project: projectArg, adapterRoot = here}) {
  const project = fs.realpathSync(path.resolve(projectArg))
  const root = fs.realpathSync(path.resolve(adapterRoot))
  const fromProject = createRequire(path.join(project, 'package.json'))
  const ts = fromProject('typescript')
  const compilerSha256 = sha(fs.readFileSync(fileURLToPath(import.meta.url)))
  const prelude = fs.realpathSync(path.join(root, 'prelude.mjs'))
  const entries = new Set(['runner.mjs', 'preflight.mjs', 'native-outbound.mjs', 'boundary-fixture.mjs'])
  const vitestRoot = path.dirname(fromProject.resolve('vitest/package.json'))
  const moduleName = file => inside(project, file) ? '/project/' + slash(path.relative(project, file))
    : inside(root, file) ? '/quod-adapter/' + slash(path.relative(root, file)) : '/external/' + slash(file)
  const readTree = (file, contents) => {
    const tree = ts.createSourceFile(file, contents, ts.ScriptTarget.Latest, true,
      /\.tsx$/.test(file) ? ts.ScriptKind.TSX : /\.jsx$/.test(file) ? ts.ScriptKind.JSX
        : /\.[cm]?ts$/.test(file) ? ts.ScriptKind.TS : ts.ScriptKind.JS)
    assert.equal(tree.parseDiagnostics.length, 0, 'module parse failed: ' + moduleName(file))
    return tree
  }
  const unparen = node => {
    while (ts.isParenthesizedExpression(node)) node = node.expression
    return node
  }
  const literal = node => {
    node = unparen(node)
    return ts.isStringLiteralLike(node) ? node.text : null
  }
  const importMeta = node => ts.isMetaProperty(node) && node.keywordToken === ts.SyntaxKind.ImportKeyword
  const metaMember = (node, name) => {
    node = unparen(node)
    return ts.isPropertyAccessExpression(node) && importMeta(unparen(node.expression)) && node.name.text === name ||
      ts.isElementAccessExpression(node) && importMeta(unparen(node.expression)) && literal(node.argumentExpression) === name
  }
  const evaluator = node => {
    node = unparen(node)
    if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.CommaToken) return evaluator(node.right)
    if (ts.isIdentifier(node)) return ['eval', 'Function', 'AsyncFunction', 'GeneratorFunction', 'AsyncGeneratorFunction'].includes(node.text)
    if (ts.isPropertyAccessExpression(node)) return ['eval', 'Function', 'AsyncFunction', 'GeneratorFunction', 'AsyncGeneratorFunction'].includes(node.name.text)
    if (ts.isElementAccessExpression(node)) return ['eval', 'Function', 'AsyncFunction', 'GeneratorFunction', 'AsyncGeneratorFunction'].includes(literal(node.argumentExpression))
    return false
  }

  return async function compile(entry, options, testFileArg) {
    assert(entries.has(entry), 'unreviewed managed entry adapter: ' + entry)
    const entryFile = fs.realpathSync(path.join(root, entry))
    const testFile = testFileArg ? fs.realpathSync(testFileArg) : null
    assert(!testFile || (inside(project, testFile) && !inside(root, testFile)), 'test entry must be inside project')
    const optionsJSON = canonicalManagedJSON(options)
    const started = performance.now(), globs = [], adapters = [], transforms = [], sources = [], resolutions = [], injected = new Set()
    const trusted = new Set([entryFile, prelude])
    const publicALS = new Set()
    const compiled = await build({
      entryPoints: [entryFile], absWorkingDir: project, bundle: true,
      write: false, platform: 'browser', format: 'esm', target: 'es2022', metafile: true,
      define: {__QUOD_CONFIG__: optionsJSON},
      // An unhandled require/import warning is a preparation failure, rather
      // than a dynamically resolved module accidentally reaching execution.
      logOverride: {'unsupported-dynamic-import': 'error', 'unsupported-require-call': 'error'},
      plugins: [{name: 'managed-closed-module-boundary', setup(builder) {
        const ordinaryResolution = async args => {
          let resolved
          if (/^(vitest(?:\/.*)?|vite\/module-runner|chai|tinybench|expect-type)$/.test(args.path)) {
            resolved = {path: args.path === 'vitest' ? path.join(vitestRoot, 'dist/index.js') : fromProject.resolve(args.path)}
          } else {
            resolved = await builder.resolve(args.path, {importer: args.importer, resolveDir: args.resolveDir,
              kind: args.kind, namespace: 'file', pluginData: {managedDefaultResolution: true}})
            if (resolved.errors.length) return resolved
            assert(!resolved.external, 'unreviewed external module: ' + args.path)
            assert(!resolved.suffix, 'module query/hash loaders unsupported: ' + args.path)
          }
          return {path: fs.realpathSync(resolved.path)}
        }
        const privateAdapterAllowed = (importer, target, specifier, kind) =>
          kind === 'entry-point' && target === entryFile ||
          trusted.has(importer) && target === prelude && specifier === './prelude.mjs'
        builder.onResolve({filter: /.*/}, async args => {
          if (args.pluginData?.managedDefaultResolution) return
          if (args.namespace === 'quod-managed-als') {
            assert.equal(args.path, 'quod:als-implementation')
            return {path: prelude}
          }
          const importer = args.importer && fs.realpathSync(args.importer)
          if (args.path === reserved) {
            assert(injected.has(importer), 'private managed helper import is not generated')
            return {path: prelude}
          }
          if (args.path === '__QUOD_TEST_ENTRY__') {
            assert(importer === entryFile && entry === 'runner.mjs' && testFile && args.kind === 'dynamic-import',
              'private test entry import is not a reviewed adapter')
            return {path: testFile}
          }
          assert(!privateSpecifier(args.path), 'private managed module namespace')
          if (isNative(args.path)) {
            if (importer === prelude && args.path === 'node:async_hooks' && args.kind === 'import-statement') {
              return {path: args.path, external: true}
            }
            assert(isALS(args.path) && args.kind === 'import-statement' && publicALS.has(importer),
              'unsupported native module: ' + args.path)
            return {path: 'public-async-local-storage', namespace: 'quod-managed-als'}
          }
          const resolved = await ordinaryResolution(args)
          if (resolved.errors?.length) return resolved
          const real = resolved.path
          if (inside(root, real)) {
            assert(privateAdapterAllowed(importer, real, args.path, args.kind),
              'project module cannot import a private managed adapter: ' + args.path)
          }
          resolutions.push({source: importer ? moduleName(importer) : null,
            specifier: args.kind === 'entry-point' ? moduleName(real) : args.path,
            kind: args.kind, target: moduleName(real)})
          return {path: real}
        })
        builder.onLoad({filter: /.*/, namespace: 'quod-managed-als'}, () => ({
          contents: 'export {ManagedAsyncLocalStorage as AsyncLocalStorage} from "quod:als-implementation";\n', loader: 'js',
        }))
        builder.onLoad({filter: /\.[cm]?[jt]sx?$/, namespace: 'file'}, async args => {
          const file = fs.realpathSync(args.path)
          let contents = fs.readFileSync(file, 'utf8')
          const sourceSha256 = sha(contents), source = moduleName(file)
          // Inspect original source before esbuild can remove type-only or
          // unused imports, and before a reviewed framework adapter rewrites
          // anything. A symlink/package alias to an adapter is private too.
          const original = readTree(file, contents), originalImports = []
          function inspectOriginal(node) {
            let specifier, kind
            if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier) {
              assert(!node.attributes && !node.assertClause, 'static import attributes unsupported')
              specifier = literal(node.moduleSpecifier); kind = 'import-statement'
            } else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference)) {
              specifier = literal(node.moduleReference.expression); kind = 'require-call'
            } else if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
              if (node.arguments.length === 1) { specifier = literal(node.arguments[0]); kind = 'dynamic-import' }
            } else if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'require') {
              if (node.arguments.length === 1) { specifier = literal(node.arguments[0]); kind = 'require-call' }
            }
            if (specifier !== undefined && specifier !== null) {
              const testEntryImport = specifier === '__QUOD_TEST_ENTRY__' && file === entryFile &&
                entry === 'runner.mjs' && testFile && kind === 'dynamic-import'
              assert(testEntryImport || !privateSpecifier(specifier), 'original source requests private managed namespace')
              if (!testEntryImport) {
                if (isNative(specifier)) {
                  const clause = ts.isImportDeclaration(node) && node.importClause
                  assert(isALS(specifier) && kind === 'import-statement' && clause && !clause.name &&
                    clause.namedBindings && ts.isNamedImports(clause.namedBindings) &&
                    clause.namedBindings.elements.length > 0 && clause.namedBindings.elements.every(element =>
                      (element.propertyName || element.name).text === 'AsyncLocalStorage'),
                  'only a named AsyncLocalStorage import is supported: ' + specifier)
                  if (file !== prelude) publicALS.add(file)
                } else originalImports.push({specifier, kind})
              }
            }
            ts.forEachChild(node, inspectOriginal)
          }
          inspectOriginal(original)
          for (const {specifier, kind} of originalImports) {
            const resolved = await ordinaryResolution({path: specifier, importer: file, resolveDir: path.dirname(file), kind})
            assert(!resolved.errors?.length, 'original module cannot be resolved: ' + specifier)
            assert(!inside(root, resolved.path) || privateAdapterAllowed(file, resolved.path, specifier, kind),
              'original source resolves to a private managed adapter: ' + specifier)
          }
          if (file === path.join(vitestRoot, 'dist/chunks/spy.DPLvp2J7.js')) {
            assert.equal(sourceSha256, '537be954493014020a2c07df3e6cbc9e8eb3c5524911b19a43b8b4a3db1f4c0c')
            contents = contents.replace('new FinalizationRegistry((ref) => {\n\tREGISTERED_MOCKS.delete(ref);\n})', '{register() {}}')
              .replace('new WeakRef(mock)', '{deref: () => mock}')
            assert(!contents.includes('FinalizationRegistry') && !contents.includes('WeakRef'))
            adapters.push({source, sourceSha256, policy: 'Retain Vitest private mock references for this file lifetime', adaptedSha256: sha(contents)})
          }
          if (file === path.join(vitestRoot, 'dist/chunks/index.OVGXnVRj.js')) {
            assert.equal(sourceSha256, '57c89e884bab20623afc06a58119c70c4f2e06397ae63e961ad0a0810c9a94b5')
            const probe = 'isPresent.Intl = _global.Intl && typeof _global.Intl === "object";'
            assert.equal(contents.split(probe).length, 2)
            contents = contents.replace(probe, 'isPresent.Intl = false; // Quod managed: Intl unsupported')
            adapters.push({source, sourceSha256, policy: 'Disable optional fake-timer Intl integration; test Intl remains denied', adaptedSha256: sha(contents)})
          }
          const tree = readTree(file, contents), edits = [], identifiers = new Set()
          function collect(node) {
            if (ts.isIdentifier(node)) identifiers.add(node.text)
            ts.forEachChild(node, collect)
          }
          collect(tree)
          let helper = '__quodCompilerDeny_' + sha(source).slice(0, 12)
          while (identifiers.has(helper)) helper += '_'
          const reject = (node, reason) => {
            edits.push({start: node.getStart(tree), end: node.getEnd(), replacement: `${helper}(${JSON.stringify(reason)})`})
            transforms.push({source, offset: node.getStart(tree), policy: reason})
          }
          function visit(node) {
            // Original source may not request the namespace later injected by
            // this compiler. The same rule covers import and export syntax.
            if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier) {
              const specifier = literal(node.moduleSpecifier)
              assert(specifier !== reserved, 'project source requests private compiler capabilities')
              if (file !== prelude && isALS(specifier)) {
                const clause = ts.isImportDeclaration(node) && node.importClause
                assert(clause && !clause.name && clause.namedBindings && ts.isNamedImports(clause.namedBindings) &&
                  clause.namedBindings.elements.length > 0 && clause.namedBindings.elements.every(element =>
                    (element.propertyName || element.name).text === 'AsyncLocalStorage'),
                'only a named AsyncLocalStorage import is supported')
                publicALS.add(file)
              }
            }
            if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
              assert(node.arguments.length === 1, 'dynamic import attributes/options unsupported')
              const specifier = literal(node.arguments[0])
              if (specifier === null) { reject(node, 'computed module import'); return }
              assert(specifier !== reserved, 'project source requests private compiler capabilities')
            }
            if (!trusted.has(file) && (ts.isCallExpression(node) || ts.isNewExpression(node)) && evaluator(node.expression)) {
              reject(node, 'dynamic code evaluation'); return
            }
            if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'require' &&
                (node.arguments.length !== 1 || literal(node.arguments[0]) === null)) {
              reject(node, 'computed require'); return
            }
            if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) &&
                ts.isIdentifier(node.expression.expression) && node.expression.expression.text === 'require') {
              reject(node, 'require property operation'); return
            }
            if (ts.isCallExpression(node) && metaMember(node.expression, 'glob')) {
              assert.equal(node.arguments.length, 1, 'glob options unsupported')
              assert(ts.isStringLiteral(node.arguments[0]), 'only literal glob is supported')
              const pattern = node.arguments[0].text, dir = path.dirname(file)
              assert(pattern.startsWith('./') || pattern.startsWith('../'), 'glob must be explicitly relative')
              const matches = [...fs.globSync(pattern, {cwd: dir})].map(name => {
                const real = fs.realpathSync(path.resolve(dir, name))
                assert(inside(project, real) && !inside(root, real) && fs.statSync(real).isFile(),
                  'glob resolved outside project files')
                const relative = slash(name)
                return {name: relative.startsWith('../') || relative.startsWith('./') ? relative : './' + relative,
                  target: moduleName(real)}
              }).sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0)
              const names = matches.map(match => match.name)
              globs.push({source, pattern, names, matches})
              const replacement = '{' + names.map(name => `${JSON.stringify(name)}:()=>${helper}(${JSON.stringify('lazy module ' + name)})`).join(',') + '}'
              edits.push({start: node.getStart(tree), end: node.getEnd(), replacement})
              transforms.push({source, offset: node.getStart(tree), policy: 'closed lazy glob names'})
              return
            }
            // Do not expose the runtime's module metadata object: aliases,
            // computed access and destructuring can all recover resolve().
            // The one supported glob form above is completely substituted.
            if (importMeta(node)) { reject(node, 'ambient module metadata'); return }
            ts.forEachChild(node, visit)
          }
          visit(tree)
          for (const edit of edits.sort((a, b) => b.start - a.start)) contents = contents.slice(0, edit.start) + edit.replacement + contents.slice(edit.end)
          if (edits.length) {
            assert(!trusted.has(file), 'reviewed runtime adapter unexpectedly requires compiler denial transform')
            injected.add(file)
            contents = `import {deny as ${helper}} from ${JSON.stringify(reserved)};\n` + contents
          }
          sources.push({source, sourceSha256, compiledSourceSha256: sha(contents)})
          return {contents, loader: /\.tsx$/.test(file) ? 'tsx' : /\.[cm]?ts$/.test(file) ? 'ts' : /\.jsx$/.test(file) ? 'jsx' : 'js'}
        })
        builder.onLoad({filter: /\.json$/, namespace: 'file'}, args => {
          const file = fs.realpathSync(args.path), contents = fs.readFileSync(file, 'utf8')
          sources.push({source: moduleName(file), sourceSha256: sha(contents), compiledSourceSha256: sha(contents)})
          return {contents, loader: 'json'}
        })
      }}],
    })
    assert.equal(compiled.outputFiles.length, 1, 'managed preparation must produce one bundle')
    const bundle = compiled.outputFiles[0].text
    const imports = Object.values(compiled.metafile.outputs).flatMap(output => output.imports)
    assert(imports.every(item => item.external && item.path === 'node:async_hooks'), 'bundle output is not closed')
    const outputTree = readTree('managed-output.mjs', bundle), externalImports = []
    function inspect(node) {
      assert(!importMeta(node), 'ambient module metadata remains in final bundle')
      // A copied native require can avoid the direct-call shape (for
      // example `const load = require; load(name)`). No such identifier is
      // needed by esbuild's closed, named __commonJS wrapper functions.
      assert(!(ts.isIdentifier(node) && node.text === 'require'), 'native require reference remains in final bundle')
      assert(!(ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword), 'dynamic import remains in final bundle')
      if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
        if (node.moduleSpecifier) {
          assert(ts.isImportDeclaration(node) && literal(node.moduleSpecifier) === 'node:async_hooks', 'unreviewed final module import/export')
          externalImports.push(literal(node.moduleSpecifier))
        }
      }
      if (ts.isCallExpression(node) || ts.isNewExpression(node)) {
        assert(!evaluator(node.expression), 'dynamic evaluator remains in final bundle')
        assert(!(ts.isIdentifier(node.expression) && /^(?:require|__require\d*)$/.test(node.expression.text)), 'native/dynamic require remains in final bundle')
      }
      ts.forEachChild(node, inspect)
    }
    inspect(outputTree)
    const sort = rows => rows.sort((a, b) => {
      const left = canonicalManagedJSON(a), right = canonicalManagedJSON(b)
      return left < right ? -1 : left > right ? 1 : 0
    })
    sort(globs); sort(adapters); sort(sources); sort(transforms); sort(resolutions)
    const sha256 = sha(bundle)
    const identity = {
      schema: 'quod-managed-compile-v1-research', compilerSha256, esbuildVersion, typescriptVersion: ts.version,
      policy: {platform: 'browser', format: 'esm', target: 'es2022', external: ['node:async_hooks'],
        dynamicCode: 'private-sticky-denial-plus-runtime-constructor-guards',
        importMeta: 'literal-relative-glob-names-only; all-other-access-sticky-denied', preparationAlwaysRuns: true},
      entry: moduleName(entryFile), testEntry: testFile && moduleName(testFile), bundleSha256: sha256,
      configurationSha256: sha(optionsJSON), moduleNameSetsSha256: sha(canonicalManagedJSON(globs)),
      sources, resolutions, adapters, globs, transforms,
    }
    return {bundle, metadata: compiled.metafile, seconds: (performance.now() - started) / 1000, sha256,
      globs, adapters, identity, identitySha256: sha(canonicalManagedJSON(identity)),
      closure: {finalBundleParsed: true, remainingDynamicImports: 0, remainingDynamicRequires: 0,
        externalImports, privateAdapterImports: 'reviewed entry only',
        nativeImports: 'prelude ALS only; named public ALS uses restricted facade'}}
  }
}
