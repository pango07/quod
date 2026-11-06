// Diagnostic-only static guard. Find private top-level bindings whose sole
// references are direct calls. This does not certify runtime provenance or
// native-capability exclusion; callers still verify the initial identity.
export function privateModuleCallees(tree) {
  if(tree.sourceType!=='module')return new Set()
  const candidates=new Map()
  for(const statement of tree.body)if(statement.type==='VariableDeclaration') {
    for(const declaration of statement.declarations)if(declaration.id.type==='Identifier'&&declaration.init) {
      candidates.set(declaration.id.name,declaration.id)
    }
  }
  let dynamic=false
  function visit(node,parent=null) {
    if(!node||typeof node!=='object')return
    if(node.type==='Identifier') {
      if(node.name==='eval')dynamic=true
      const declaration=candidates.get(node.name)
      if(declaration&&node!==declaration&&!(parent?.type==='CallExpression'&&parent.callee===node&&!parent.optional))candidates.delete(node.name)
    }
    if(node.type==='DebuggerStatement'||node.type==='WithStatement')dynamic=true
    for(const value of Object.values(node)) {
      if(Array.isArray(value))for(const child of value)visit(child,node)
      else if(value&&typeof value==='object')visit(value,node)
    }
  }
  visit(tree)
  return dynamic?new Set():new Set(candidates.keys())
}
