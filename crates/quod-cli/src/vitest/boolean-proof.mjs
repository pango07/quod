// Diagnostic prototype. A field read may depend only on ToBoolean(value)
// when its actual executed expression feeds a boolean language context.
// No filenames, source maps, source-on-disk or matching outputs grant proof.
import { parse } from './acorn.mjs'
import { Session } from 'node:inspector'
import { getCallSites } from 'node:util'
import { fileURLToPath } from 'node:url'
import { unchangedIntrinsics } from './names-proof.mjs'

const ownFile = fileURLToPath(import.meta.url)
const plainFile = name => name?.startsWith('file:') ? fileURLToPath(name) : name

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
  function booleanContext(n) {
    const p = parents.get(n)
    if (!p) return false
    if (p.type === 'ChainExpression') return booleanContext(p)
    if (p.type === 'LogicalExpression' && (p.operator === '&&' || p.operator === '||')) return booleanContext(p)
    if (p.type === 'UnaryExpression' && p.operator === '!') return true
    return ['IfStatement', 'WhileStatement', 'DoWhileStatement', 'ForStatement', 'ConditionalExpression'].includes(p.type) && p.test === n
  }
  return nodes.filter(n => n.type === 'MemberExpression' && !n.computed &&
    n.property.name === property && receiver(n.object) && booleanContext(n))
    .map(n => ({start:n.start, end:n.end}))
}

export function createBooleanProof({ internalFiles = [] } = {}) {
  const skip = new Set([ownFile, ...internalFiles.map(plainFile)])
  const parsed = new Map(), sources = new Map()
  const stats = {boolean:0, whole:0, sources:0}
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
  function check(property) {
    if (disabled || typeof getCallSites !== 'function' || !unchangedIntrinsics()) return false
    let frame
    try {
      frame = getCallSites(32, {sourceMap:false}).find(f => {
        const name = plainFile(f.scriptName)
        return name && !name.startsWith('node:') && !skip.has(name)
      })
    } catch { return false }
    if (!frame || !parsed.get(frame.scriptId)) return false
    const key = `${frame.scriptId}:${property}`
    if (!sources.has(key)) {
      if (sources.size >= 128) return false
      sources.set(key, null)
      try {
        post('Debugger.getScriptSource', {scriptId:frame.scriptId}, (err, result) => {
          if (err || typeof result?.scriptSource !== 'string') return
          try {
            const source = result.scriptSource
            const sites = certifyBooleanReads(source, property)
            const lines = [0]
            for (let i=0; i<source.length; i++) if (source[i] === '\n') lines.push(i+1)
            sources.set(key, {sites, lines}); stats.sources++
          } catch { /* Unproved, including parser resource limits. */ }
        })
      } catch { return false }
    }
    const source = sources.get(key), line = source?.lines[frame.lineNumber-1]
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
    close() { disabled = true; try { session?.disconnect() } catch {} },
  }
}
