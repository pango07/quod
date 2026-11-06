// Opt-in bounded proof. A field read may depend only on ToBoolean(value)
// when its actual executed expression feeds a boolean language context.
// No filenames, source maps, source-on-disk or matching outputs grant proof.
import { parse } from './acorn.mjs'
import { Session } from 'node:inspector'
import { getCallSites } from 'node:util'
import { fileURLToPath } from 'node:url'
import { unchangedIntrinsics } from './names-proof.mjs'
import fs from 'node:fs'

const ownFile = fileURLToPath(import.meta.url)
const plainFile = name => name?.startsWith('file:') ? fileURLToPath(name) : name
const nativeStats = fs.Stats
const nativeStatsPrototype = nativeStats.prototype
const statPrototypes = []
for (let p = nativeStatsPrototype; p && p !== Object.prototype; p = Object.getPrototypeOf(p)) {
  statPrototypes.push({object:p, parent:Object.getPrototypeOf(p), descriptors:Object.getOwnPropertyDescriptors(p)})
}
function pristineStats() {
  if (fs.Stats !== nativeStats || nativeStats.prototype !== nativeStatsPrototype) return false
  return statPrototypes.every(({object,parent,descriptors}) => Object.getPrototypeOf(object) === parent &&
    Reflect.ownKeys(object).length === Reflect.ownKeys(descriptors).length && Reflect.ownKeys(descriptors).every(k => {
      const a=descriptors[k], b=Object.getOwnPropertyDescriptor(object,k)
      return b && a.value===b.value && a.get===b.get && a.set===b.set && a.writable===b.writable && a.configurable===b.configurable && a.enumerable===b.enumerable
    }))
}

function inBooleanContext(n, parents) {
  const p = parents.get(n)
  if (!p) return false
  if (p.type === 'ChainExpression') return inBooleanContext(p, parents)
  if (p.type === 'LogicalExpression' && (p.operator === '&&' || p.operator === '||')) return inBooleanContext(p, parents)
  if (p.type === 'UnaryExpression' && p.operator === '!') return true
  return ['IfStatement', 'WhileStatement', 'DoWhileStatement', 'ForStatement', 'ConditionalExpression'].includes(p.type) && p.test === n
}

// The only accepted helper returns native statSync directly through an
// empty catch. The caller immediately consumes it through .isFile() or
// .isDirectory(), or tests whether stat succeeded in a boolean context,
// without exposing or inspecting the Stats object. A private const may
// also feed consecutive, type-only if tests. No arbitrary code can run
// between those tests, and no reference to the binding may escape them.
export function certifyStatTypeReads(source) {
  if (typeof source !== 'string' || source.length > 2_000_000) return {helpers:[], calls:[]}
  let tree
  try {tree=parse(source,{ecmaVersion:'latest',sourceType:'module'})} catch {return {helpers:[],calls:[]}}
  const nodes=[], parents=new Map()
  function visit(n, parent=null) {
    if (!n || typeof n!=='object') return
    if (n.type) {nodes.push(n); parents.set(n,parent)}
    for (const v of Object.values(n)) {
      if (Array.isArray(v)) for (const c of v) visit(c,n)
      else if (v && typeof v==='object') visit(v,n)
    }
  }
  visit(tree)
  const helpers=[]
  for (const f of nodes) {
    if (f.type!=='FunctionDeclaration' || !f.id || f.async || f.generator || f.params.length!==1 || f.params[0].type!=='Identifier' || f.body.body.length!==1) continue
    const t=f.body.body[0], ret=t.block?.body?.[0], c=ret?.argument
    if (t.type!=='TryStatement' || t.finalizer || !t.handler || t.handler.param || t.handler.body.body.length || t.block.body.length!==1 || ret.type!=='ReturnStatement') continue
    if (c?.type!=='CallExpression' || c.optional || c.callee.type!=='MemberExpression' || c.callee.computed || c.callee.optional || c.callee.object.type!=='Identifier' || c.callee.property.name!=='statSync') continue
    const [arg,opts]=c.arguments
    const opt=opts?.properties?.[0]
    if (c.arguments.length!==2 || arg.type!=='Identifier' || arg.name!==f.params[0].name || opts.type!=='ObjectExpression' || opts.properties.length!==1 || opt.type!=='Property' || opt.computed || opt.method || opt.kind!=='init' || opt.key.type!=='Identifier' || opt.key.name!=='throwIfNoEntry' || opt.value.type!=='Literal' || opt.value.value!==false) continue
    helpers.push({name:f.id.name,start:c.start,end:c.end})
  }
  const calls=[]
  function privateTypeBinding(call) {
    const binding=parents.get(call), declaration=parents.get(binding), block=parents.get(declaration)
    if (binding?.type!=='VariableDeclarator' || binding.init!==call || binding.id.type!=='Identifier' ||
        declaration?.type!=='VariableDeclaration' || declaration.kind!=='const' || declaration.declarations.length!==1 || block?.type!=='BlockStatement') return false
    const name=binding.id.name, allowed=new Set([binding.id])
    function condition(n) {
      if (n.type==='ChainExpression') return condition(n.expression)
      if (n.type==='Identifier' && n.name===name) { allowed.add(n); return true }
      if (n.type==='UnaryExpression' && n.operator==='!') return condition(n.argument)
      if (n.type==='LogicalExpression' && ['&&','||'].includes(n.operator)) return condition(n.left) && condition(n.right)
      if (n.type!=='CallExpression' || n.optional || n.arguments.length || n.callee.type!=='MemberExpression' ||
          n.callee.computed || !['isFile','isDirectory'].includes(n.callee.property.name) ||
          n.callee.object.type!=='Identifier' || n.callee.object.name!==name) return false
      allowed.add(n.callee.object)
      return true
    }
    function exits(n) {
      if (!n) return false
      if (['ReturnStatement','ThrowStatement'].includes(n.type)) return true
      if (n.type==='BlockStatement') return exits(n.body.at(-1))
      return n.type==='IfStatement' && exits(n.consequent) && exits(n.alternate)
    }
    function tests(n) {
      if (!condition(n.test)) return false
      return n.alternate?.type==='IfStatement' ? tests(n.alternate) : true
    }
    function trueBranchesExit(n) {
      return exits(n.consequent) && (n.alternate?.type==='IfStatement' ? trueBranchesExit(n.alternate) : !n.alternate || exits(n.alternate))
    }
    let prior=null
    for (let i=block.body.indexOf(declaration)+1; i<block.body.length; i++) {
      const n=block.body[i]
      if (n.type!=='IfStatement') break
      if (prior && !trueBranchesExit(prior) || !tests(n)) return false
      prior=n
    }
    if (!prior) return false
    // Includes nested closures, branch bodies, later statements, writes,
    // aliases, destructuring and even unused extra reads. Direct eval can
    // observe a lexical binding without a syntactic reference to its name.
    return !nodes.some(n=>n.start>=block.start && n.end<=block.end && n.type==='Identifier' &&
      (n.name==='eval' || n.name===name && !allowed.has(n)))
  }
  for (const n of nodes) {
    if (n.type==='CallExpression' && !n.optional && n.callee.type==='Identifier' && n.arguments.length===1 && n.arguments[0].type==='Identifier' &&
        helpers.some(h=>h.name===n.callee.name) && (inBooleanContext(n,parents) || privateTypeBinding(n))) {
      calls.push({name:n.callee.name,start:n.start,end:n.end})
    }
    if (n.type!=='CallExpression' || n.optional || n.arguments.length || n.callee.type!=='MemberExpression' || n.callee.computed || !['isFile','isDirectory'].includes(n.callee.property.name)) continue
    const call=n.callee.object
    if (call.type!=='CallExpression' || call.optional || call.callee.type!=='Identifier' || call.arguments.length!==1 || call.arguments[0].type!=='Identifier') continue
    if (helpers.some(h=>h.name===call.callee.name)) calls.push({name:call.callee.name,start:call.start,end:call.end})
  }
  return {helpers,calls}
}

export function certifyBooleanReads(source, property = 'exports') {
  if (typeof source !== 'string' || source.length > 2_000_000) return []
  let tree
  try { tree = parse(source, {ecmaVersion:'latest', sourceType:'module'}) }
  catch { return [] }
  const parents = new Map(), nodes = []
  function visit(n, parent = null) {
    if (!n || typeof n !== 'object') return
    if (typeof n.type === 'string') { parents.set(n, parent); nodes.push(n) }
    for (const v of Object.values(n)) {
      if (Array.isArray(v)) for (const c of v) visit(c, n)
      else if (v && typeof v === 'object') visit(v, n)
    }
  }
  visit(tree)
  // Restrict the receiver to a simple member chain. No calls, computed
  // properties or a repeated same-named field can overlap the site's span.
  function receiver(n) {
    if (n.type === 'Identifier') return true
    if (n.type === 'ChainExpression') return receiver(n.expression)
    return n.type === 'MemberExpression' && !n.computed &&
      n.property.name !== property && receiver(n.object)
  }
  return nodes.filter(n => n.type === 'MemberExpression' && !n.computed &&
    n.property.name === property && receiver(n.object) && inBooleanContext(n,parents))
    .map(n => ({start:n.start, end:n.end}))
}

export function createBooleanProof({ internalFiles = [] } = {}) {
  const skip = new Set([ownFile, ...internalFiles.map(plainFile)])
  const parsed = new Map(), sources = new Map()
  const stats = {boolean:0, whole:0, sources:0, statType:0, statWhole:0}
  let session = null, post = null, disabled = false
  try {
    session = new Session()
    session.connect()
    post = session.post.bind(session)
    session.on('Debugger.scriptParsed', ({params}) => {
      if (parsed.size < 20000) parsed.set(params.scriptId, params.isModule === true && params.startLine === 0 && params.startColumn === 0)
    })
    post('Debugger.enable', {}, err => { if (err) disabled = true })
  } catch { disabled = true }
  function frames() {
    if (disabled || typeof getCallSites !== 'function' || !unchangedIntrinsics()) return []
    try {
      return getCallSites(32, {sourceMap:false}).filter(f => {
        const name = plainFile(f.scriptName)
        return name && !name.startsWith('node:') && !skip.has(name)
      })
    } catch {return []}
  }
  function load(frame, property) {
    if (!frame || !parsed.get(frame.scriptId)) return null
    const key = `${frame.scriptId}:${property}`
    if (!sources.has(key)) {
      if (sources.size >= 128) return null
      sources.set(key, null)
      try {
        post('Debugger.getScriptSource', {scriptId:frame.scriptId}, (err, result) => {
          if (err || typeof result?.scriptSource !== 'string') return
          try {
            const source = result.scriptSource
            const sites = property==='@stat' ? certifyStatTypeReads(source) : certifyBooleanReads(source, property)
            const lines = [0]
            for (let i=0; i<source.length; i++) if (source[i] === '\n') lines.push(i+1)
            sources.set(key, {sites, lines}); stats.sources++
          } catch { /* Unproved, including parser resource limits. */ }
        })
      } catch { return null }
    }
    return sources.get(key)
  }
  function offset(frame, source) {
    const line=source?.lines[frame.lineNumber-1]
    return line===undefined || frame.columnNumber<1 ? -1 : line+frame.columnNumber-1
  }
  function check(property) {
    const frame=frames()[0], source=load(frame,property)
    if (!frame || !source) return false
    const line = source.lines[frame.lineNumber-1]
    if (line === undefined || frame.columnNumber < 1) return false
    const at = line + frame.columnNumber-1
    return source.sites.some(s => s.start <= at && at < s.end)
  }
  return {
    stats,
    test(property) {
      const proved = check(property)
      stats[proved ? 'boolean' : 'whole']++
      return proved
    },
    statType(args) {
      let proved=false
      if (typeof args[0]==='string' && args.length===2 && unchangedIntrinsics() && pristineStats()) {
        const [helper,caller]=frames()
        const source=load(helper,'@stat')
        if (helper && caller && source && helper.scriptId===caller.scriptId) {
          const at=offset(helper,source), calledAt=offset(caller,source)
          const h=source.sites.helpers.find(h=>h.start<=at && at<h.end)
          proved=!!h && source.sites.calls.some(c=>c.name===h.name && c.start<=calledAt && calledAt<c.end)
        }
      }
      stats[proved ? 'statType' : 'statWhole']++
      return proved
    },
    close() { disabled = true; try { session?.disconnect() } catch {} },
  }
}
