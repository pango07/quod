// Syntax certification only. This module does not grant replay permission.
// The runtime must additionally bind the native decoder, the fresh read value,
// the actual pure parser and its invocation, and rule out observable escapes.
import { parse } from './acorn.mjs'

const functionNode = n => ['FunctionDeclaration','FunctionExpression','ArrowFunctionExpression'].includes(n?.type)
const identifier = (n, name) => n?.type === 'Identifier' && (name === undefined || n.name === name)
const literal = (n, value) => n?.type === 'Literal' && n.value === value
const directCall = n => n?.type === 'CallExpression' && !n.optional && !n.arguments.some(a=>a.type==='SpreadElement')
const member = (n, object, property) => n?.type === 'MemberExpression' && !n.computed && !n.optional && identifier(n.object,object) && identifier(n.property,property)
const span = n => ({start:n.start,end:n.end})

export function certifyAsyncJsonReads(source) {
  const none = { reads:[], parsers:[] }
  if (typeof source !== 'string' || source.length > 2_000_000) return none
  let tree
  try { tree=parse(source,{ecmaVersion:'latest',sourceType:'module'}) } catch { return none }
  const nodes=[], parents=new Map()
  function walk(n,parent=null) {
    if (!n || typeof n!=='object') return
    if (n.type) {nodes.push(n); parents.set(n,parent)}
    for (const value of Object.values(n)) {
      if (Array.isArray(value)) for (const child of value) walk(child,n)
      else if (value && typeof value==='object') walk(value,n)
    }
  }
  walk(tree)
  function inside(n,scope) {return scope.start<=n.start && n.end<=scope.end}
  function bindingContains(n,name) {
    if (!n) return false
    if (identifier(n,name)) return true
    if (n.type==='RestElement') return bindingContains(n.argument,name)
    if (n.type==='AssignmentPattern') return bindingContains(n.left,name)
    if (n.type==='ArrayPattern') return n.elements.some(e=>bindingContains(e,name))
    if (n.type==='ObjectPattern') return n.properties.some(p=>bindingContains(p.type==='RestElement'?p.argument:p.value,name))
    return false
  }
  // A conservative source-wide rejection avoids mistaking a lexical String
  // for the native global. Unknown or shadowed bindings simply do not qualify.
  const shadowsString=nodes.some(n=>n.type==='VariableDeclarator' && bindingContains(n.id,'String') ||
    functionNode(n) && (bindingContains(n.id,'String') || n.params.some(p=>bindingContains(p,'String'))) ||
    ['ClassDeclaration','ClassExpression'].includes(n.type) && identifier(n.id,'String') ||
    n.type==='CatchClause' && bindingContains(n.param,'String') ||
    ['ImportSpecifier','ImportDefaultSpecifier','ImportNamespaceSpecifier'].includes(n.type) && identifier(n.local,'String'))

  function constantInScope(name,at) {
    for (let scope=parents.get(at);scope;scope=parents.get(scope)) {
      if (!['BlockStatement','Program'].includes(scope.type)) continue
      for (const statement of scope.body) {
        if (statement.type!=='VariableDeclaration' || statement.kind!=='const' || statement.end>at.start) continue
        if (statement.declarations.some(d=>identifier(d.id,name))) return true
      }
    }
    return false
  }
  function enclosingFunction(n) {
    for (let p=parents.get(n);p;p=parents.get(p)) if (functionNode(p)) return p
    return null
  }
  function reference(n) {
    const p=parents.get(n)
    if (p?.type==='MemberExpression' && p.property===n && !p.computed) return false
    if (p?.type==='Property' && p.key===n && !p.computed && !p.shorthand) return false
    return true
  }

  const parsers=[]
  for (const fn of nodes.filter(functionNode)) {
    if (fn.async || fn.generator || fn.params.length!==2 || !fn.params.every(p=>identifier(p)) || fn.params[0].name===fn.params[1].name) continue
    const body=fn.body.type==='BlockStatement'
      ? fn.body.body.length===1 && fn.body.body[0].type==='ReturnStatement' && fn.body.body[0].argument
      : fn.body
    if (!directCall(body) || !member(body.callee,'JSON','parse') || body.arguments.length!==1 || !identifier(body.arguments[0],fn.params[1].name)) continue
    parsers.push({call:span(body),function:span(fn)})
  }
  if (shadowsString) return {reads:[],parsers}

  const reads=[]
  for (const declaration of nodes) {
    if (declaration.type!=='VariableDeclaration' || declaration.kind!=='const' || declaration.declarations.length!==1) continue
    const d=declaration.declarations[0], convert=d.init, awaited=convert?.arguments?.[0], read=awaited?.argument
    if (!identifier(d.id) || !directCall(convert) || !identifier(convert.callee,'String') || convert.arguments.length!==1 || awaited.type!=='AwaitExpression') continue
    if (!directCall(read) || !identifier(read.callee) || read.arguments.length!==1 || !identifier(read.arguments[0])) continue
    if (!constantInScope(read.callee.name,declaration) || !constantInScope(read.arguments[0].name,declaration)) continue
    const block=parents.get(declaration), loop=parents.get(block), fn=enclosingFunction(block)
    if (block?.type!=='BlockStatement' || loop?.type!=='ForOfStatement' || loop.await || loop.body!==block || !fn?.async) continue
    const loopDecl=loop.left
    if (loopDecl.type!=='VariableDeclaration' || loopDecl.kind!=='const' || loopDecl.declarations.length!==1 || !identifier(loopDecl.declarations[0].id)) continue
    const place=loopDecl.declarations[0].id.name
    // Direct eval could read the private raw binding without naming it in AST.
    if (nodes.some(n=>inside(n,block) && directCall(n) && identifier(n.callee,'eval'))) continue
    const index=block.body.indexOf(declaration)
    for (const branch of block.body.slice(index+1)) {
      const test=branch.test, body=branch.consequent?.body
      if (branch.type!=='IfStatement' || branch.alternate || test?.type!=='BinaryExpression' || test.operator!=='===' || !identifier(test.left,place) || !literal(test.right,'package.json')) continue
      if (branch.consequent.type!=='BlockStatement' || body.length<2 || body.at(-1).type!=='ContinueStatement' || body.at(-1).label) continue
      const first=body[0]
      if (first.type!=='VariableDeclaration' || first.kind!=='const' || first.declarations.length!==1 || !identifier(first.declarations[0].id)) continue
      const load=first.declarations[0].init?.argument
      if (first.declarations[0].init?.type!=='AwaitExpression' || !directCall(load) || !identifier(load.callee) || load.arguments.length!==2 || !identifier(load.arguments[0],read.arguments[0].name) || !identifier(load.arguments[1],d.id.name)) continue
      if (!constantInScope(load.callee.name,branch)) continue
      const rawRefs=nodes.filter(n=>inside(n,block) && identifier(n,d.id.name) && reference(n))
      let valid=true
      for (const ref of rawRefs) {
        if (ref===d.id || ref===load.arguments[1]) continue
        // Only the observed later trim/loader paths are accepted; the first
        // branch's terminal continue makes them unreachable after its parse.
        if (ref.start<branch.end || enclosingFunction(ref)!==fn) {valid=false;break}
        const p=parents.get(ref), call=parents.get(p)
        if (member(p,d.id.name,'trim') && directCall(call) && call.callee===p && !call.arguments.length) {
          const comparison=parents.get(call)
          if (comparison?.type==='BinaryExpression' && comparison.operator==='===' && comparison.left===call && literal(comparison.right,'')) continue
        }
        if (directCall(p) && identifier(p.callee,load.callee.name) && p.arguments.length===2 && identifier(p.arguments[0],read.arguments[0].name) && p.arguments[1]===ref) continue
        valid=false;break
      }
      if (!valid) continue
      reads.push({read:span(read),convert:span(convert),loaderCall:span(load),raw:d.id.name,scope:span(block)})
    }
  }
  return {reads,parsers}
}
