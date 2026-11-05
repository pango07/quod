// A bounded proof for Object.keys({ ...privateJson.dependencies, ... }).
// This never changes application code. V8 supplies the actual executed
// ESM source, identified by scriptId; filenames/source maps are not proof.
// Unknown syntax, effects, or inspector support retain value dependencies.
import { parse } from './acorn.mjs'
import { Session } from 'node:inspector'
import { getCallSites } from 'node:util'
import { fileURLToPath } from 'node:url'

const NativeObject = Object
const nativeKeys = Object.keys
const sameValue = Object.is
const descriptor = Object.getOwnPropertyDescriptor
const ownKeys = Reflect.ownKeys
const globalObject = globalThis
const ownFile = fileURLToPath(import.meta.url)
const recordFile = fileURLToPath(new URL('./record.mjs', import.meta.url))
const fields = new Set(['dependencies', 'devDependencies', 'peerDependencies', 'optionalDependencies'])
// Parsing/proving must not invoke application replacements of the language
// intrinsics. Snapshot before configuration loads; any alteration falls back.
const intrinsics = [Object, Array, String, Number, RegExp, Map, Set, Function,
  Object.prototype, Array.prototype, String.prototype, Number.prototype,
  RegExp.prototype, Map.prototype, Set.prototype, Function.prototype, Reflect]
const globalNames = ['Object', 'Array', 'String', 'Number', 'RegExp', 'Map', 'Set', 'Function', 'Reflect']
const globalValues = globalNames.map(name => descriptor(globalObject, name)?.value)
const intrinsicShapes = intrinsics.map(object => ({object, keys:ownKeys(object),
  descriptors:ownKeys(object).map(key => descriptor(object, key))}))
function unchangedIntrinsics() {
  for (let i = 0; i < globalNames.length; i++) {
    if (descriptor(globalObject, globalNames[i])?.value !== globalValues[i]) return false
  }
  for (let n = 0; n < intrinsicShapes.length; n++) {
    const s = intrinsicShapes[n], keys = ownKeys(s.object)
    if (keys.length !== s.keys.length) return false
    for (let i = 0; i < keys.length; i++) {
      if (keys[i] !== s.keys[i]) return false
      const was = s.descriptors[i], now = descriptor(s.object, keys[i])
      if (!now || !sameValue(now.value, was.value) || now.get !== was.get || now.set !== was.set ||
          now.writable !== was.writable || now.enumerable !== was.enumerable || now.configurable !== was.configurable) return false
    }
  }
  return true
}
const isMember = (n, object, property) => n?.type === 'MemberExpression' && !n.computed && !n.optional && n.object.type === 'Identifier' && n.object.name === object && n.property.name === property

function nodesOf(root) {
  const nodes = []
  const visit = n => {
    if (!n || typeof n !== 'object') return
    if (typeof n.type === 'string') nodes.push(n)
    for (const [k, v] of Object.entries(n)) {
      if (k === 'loc') continue
      if (Array.isArray(v)) for (const c of v) visit(c)
      else if (v && typeof v === 'object') visit(v)
    }
  }
  visit(root)
  return nodes
}
function binds(n, name) {
  if (!n) return false
  if (n.type === 'Identifier') return n.name === name
  if (n.type === 'RestElement') return binds(n.argument, name)
  if (n.type === 'AssignmentPattern') return binds(n.left, name)
  if (n.type === 'ArrayPattern') return n.elements.some(e => binds(e, name))
  if (n.type === 'ObjectPattern') return n.properties.some(p => binds(p.type === 'RestElement' ? p.argument : p.value, name))
  return false
}

// Pure syntax analysis, exported for adversarial regression coverage.
export function certifyNamesOnly(source) {
  if (typeof source !== 'string' || source.length > 2_000_000) return []
  let tree
  try { tree = parse(source, { ecmaVersion: 'latest', sourceType: 'module' }) }
  catch { return [] }
  const nodes = nodesOf(tree)
  // Direct eval can reach private module bindings without a syntactic use.
  if (nodes.some(n => n.type === 'Identifier' && n.name === 'eval' || n.type === 'WithStatement')) return []
  // Accept the global Object only; rejecting shadowing anywhere is conservative.
  if (nodes.some(n =>
    n.type === 'VariableDeclarator' && binds(n.id, 'Object') ||
    /^(Function|ArrowFunction)/.test(n.type) && (binds(n.id, 'Object') || n.params?.some(p => binds(p, 'Object'))) ||
    /^Class(Declaration|Expression)$/.test(n.type) && binds(n.id, 'Object') ||
    n.type === 'CatchClause' && binds(n.param, 'Object') ||
    /^Import.*Specifier$/.test(n.type) && binds(n.local, 'Object')
  )) return []
  const certificates = []
  // Only private top-level declarations. A function's parameter or local
  // with the same name is an additional use and makes this binding fail.
  for (const [index, statement] of tree.body.entries()) {
    if (statement.type !== 'VariableDeclaration' || statement.declarations.length !== 1) continue
    let next = tree.body[index + 1]
    if (next?.type === 'ExportNamedDeclaration') next = next.declaration
    if (next?.type !== 'VariableDeclaration' || next.declarations.length !== 1 || next.declarations[0].id.type !== 'Identifier') continue
    // No user code may run between parse's return and reading Object.keys.
    // Accept a chain such as Object.keys({...}).filter(...).sort(...),
    // whose first evaluated call is the keys call. Reject awaits, sequences,
    // wrappers, computed members and intervening statements. This prevents
    // a transient replacement/getter from swapping the callee before copy.
    const firstCall = n => {
      if (n?.optional) return null
      if (n?.type === 'MemberExpression' && !n.computed) return firstCall(n.object)
      if (n?.type === 'CallExpression') return firstCall(n.callee) ?? n
      return null
    }
    const immediate = firstCall(next.declarations[0].init)
    for (const d of statement.declarations) {
      if (d.id.type !== 'Identifier' || d.init?.type !== 'CallExpression' || d.init.optional || !isMember(d.init.callee, 'JSON', 'parse') || d.init.arguments.length !== 1) continue
      const name = d.id.name
      const allowed = new Set([d.id])
      const sites = []
      for (const n of [immediate].filter(Boolean)) {
        if (n.type !== 'CallExpression' || n.optional || !isMember(n.callee, 'Object', 'keys') || n.arguments.length !== 1) continue
        const arg = n.arguments[0]
        if (arg.type !== 'ObjectExpression' || !arg.properties.length) continue
        const sources = arg.properties.map(p => p.type === 'SpreadElement' ? p.argument : null)
        if (!sources.every(s => s?.type === 'MemberExpression' && !s.computed && !s.optional && s.object.type === 'Identifier' && s.object.name === name && fields.has(s.property.name))) continue
        for (const s of sources) allowed.add(s.object)
        sites.push({ start:n.start, end:n.end, fields:[...new Set(sources.map(s => s.property.name))] })
      }
      if (!sites.length || nodes.some(n => n.type === 'Identifier' && n.name === name && !allowed.has(n))) continue
      certificates.push({ start:d.init.start, end:d.init.end, sites })
    }
  }
  return certificates
}

export function createNamesProof() {
  const parsed = new Map(), sources = new Map()
  let session = null, started = false, disabled = false
  const stats = { bound:0, ignored:0, fallback:0 }
  const frame = () => {
    if (typeof getCallSites !== 'function') return null
    try {
      for (const f of getCallSites(32, { sourceMap:false })) {
        const file = f.scriptName?.startsWith('file:') ? fileURLToPath(f.scriptName) : f.scriptName
        if (!file || file.startsWith('node:') || file === ownFile || file === recordFile) continue
        return typeof f.scriptId === 'string' ? f : null
      }
    } catch {}
    return null
  }
  const start = () => {
    if (started || disabled) return
    started = true
    try {
      session = new Session()
      session.connect() // in-process inspector; no listening port
      session.on('Debugger.scriptParsed', ({params}) => {
        if (parsed.size < 20000) parsed.set(params.scriptId, { module:params.isModule === true, line:params.startLine, column:params.startColumn })
      })
      session.post('Debugger.enable', {}, err => { if (err) disabled = true })
    } catch { disabled = true }
  }
  const source = f => {
    if (!f || disabled) return null
    start()
    const meta = parsed.get(f.scriptId)
    if (!meta?.module || meta.line !== 0 || meta.column !== 0) return null
    if (!sources.has(f.scriptId)) {
      if (sources.size >= 128) return null
      sources.set(f.scriptId, null)
      try {
        session.post('Debugger.getScriptSource', {scriptId:f.scriptId}, (err, result) => {
          if (err || typeof result?.scriptSource !== 'string') return
          const text = result.scriptSource
          const certificates = certifyNamesOnly(text)
          const lines = [0]
          for (let i=0; i<text.length; i++) if (text[i] === '\n') lines.push(i+1)
          sources.set(f.scriptId, { certificates, lines, length:text.length })
        })
      } catch {}
    }
    // Async or unavailable replies simply cannot prove this read now.
    return sources.get(f.scriptId)
  }
  const offset = (f,s) => {
    const start = s?.lines[f.lineNumber-1]
    return start === undefined || f.columnNumber < 1 ? -1 : start + f.columnNumber-1
  }
  const nativeCall = () => {
    const o = descriptor(globalObject, 'Object')
    const k = descriptor(NativeObject, 'keys')
    return o?.value === NativeObject && k?.value === nativeKeys
  }
  return {
    stats,
    bind() {
      if (!nativeCall() || !unchangedIntrinsics()) { stats.fallback++; return null }
      const f = frame(), s = source(f), at = offset(f ?? {}, s)
      const c = s?.certificates.find(c => c.start <= at && at < c.end)
      if (!c) { stats.fallback++; return null }
      stats.bound++
      return { scriptId:f.scriptId, source:s, sites:c.sites }
    },
    seal(binding, pristine) {
      if (binding) binding.valid = unchangedIntrinsics() && pristine(binding.sites[0].fields) && nativeCall()
    },
    ignores(binding, path, pristine) {
      if (!binding?.valid || disabled || path.length !== 2 || !fields.has(path[0]) || !nativeCall()) return false
      const f = frame()
      if (!f || f.scriptId !== binding.scriptId) return false
      const at = offset(f, binding.source)
      const site = binding.sites.find(s => s.start <= at && at < s.end && s.fields.includes(path[0]))
      if (!site || !pristine(site.fields)) return false
      stats.ignored++
      return true
    },
    close() {
      disabled = true
      try { session?.disconnect() } catch {}
      session = null
      parsed.clear(); sources.clear()
    },
  }
}
