// Diagnostic only. Recover a bounded native-call argument from the actual
// paused script and Inspector data descriptors. Never evaluate application
// expressions or invoke getters. Unknown syntax/scope/identity stays unknown.
import {parse} from '../../crates/quod-cli/src/vitest/acorn.mjs'
import {createHash} from 'node:crypto'

export function observeNativeCallArguments(post) {
  const sources=new Map()
  // Pinned Node 24.18.0 puts paused scopes/receivers in "backtrace";
  // getProperties children inherit that group and Debugger.resume releases
  // it. Other runtimes keep the conservative per-handle cleanup. See the
  // native handle-lifetime regression before extending this version guard.
  const releaseOnResume=process.versions.node==='24.18.0'
  function source(scriptId) {
    if(sources.has(scriptId))return sources.get(scriptId)
    const text=post('Debugger.getScriptSource',{scriptId}).scriptSource
    if(typeof text!=='string'||text.length>2_000_000)throw Error('unsupported caller source')
    let tree
    try {tree=parse(text,{ecmaVersion:'latest',sourceType:'module',allowReturnOutsideFunction:true})}
    catch {tree=parse(text,{ecmaVersion:'latest',sourceType:'script',allowReturnOutsideFunction:true,allowAwaitOutsideFunction:true})}
    const calls=[],lines=[0]
    for(let i=0;i<text.length;i++)if(text[i]==='\n')lines.push(i+1)
    function visit(node) {
      if(!node||typeof node!=='object')return
      if(node.type==='CallExpression')calls.push(node)
      for(const value of Object.values(node)) {
        if(Array.isArray(value))value.forEach(visit)
        else if(value&&typeof value==='object')visit(value)
      }
    }
    visit(tree)
    const result={calls,lines,sha256:createHash('sha256').update(text).digest('hex')}
    sources.set(scriptId,result);return result
  }
  return (frame,nativeFunction,{pureAdditionalArguments=false,describeOwnAccessors=false}={})=>{
    const handles=new Set()
    let callerSourceSha256=null
    function properties(id) {
      if(!id)throw Error('missing runtime object')
      const r=post('Runtime.getProperties',{objectId:id,ownProperties:true,generatePreview:false})
      if(r.exceptionDetails)throw Error('runtime descriptors unavailable')
      if(!releaseOnResume)for(const p of [...r.result??[],...r.internalProperties??[],...r.privateProperties??[]]) {
        for(const v of [p.value,p.get,p.set])if(v?.objectId)handles.add(v.objectId)
      }
      return r.result
    }
    function data(p) {
      if(!p||p.get||p.set||!p.value)throw Error('argument is not an own data property')
      return p.value
    }
    function own(object,key) {
      if(!object?.objectId||object.subtype==='proxy')throw Error('proxy or unavailable property receiver')
      const p=properties(object.objectId).find(p=>p.name===key&&p.isOwn===true)
      return data(p)
    }
    function binding(name) {
      for(const scope of frame.scopeChain) {
        if(!['local','block','closure','module','script','catch'].includes(scope.type))throw Error('unsupported argument scope')
        const p=properties(scope.object.objectId).find(p=>p.name===name)
        if(p)return data(p)
      }
      throw Error('argument binding unavailable')
    }
    function value(node) {
      if(node.type==='Identifier')return binding(node.name)
      if(node.type==='ThisExpression')return frame.this
      if(node.type==='MemberExpression'&&!node.optional) {
        const key=!node.computed?(node.property.type==='Identifier'?node.property.name:null):
          node.property.type==='Literal'&&['string','number'].includes(typeof node.property.value)?String(node.property.value):null
        if(key===null)throw Error('computed argument property')
        return own(value(node.object),key)
      }
      if(node.type==='Literal'&&(node.value===null||['string','number','boolean'].includes(typeof node.value))) {
        return {type:node.value===null?'object':typeof node.value,subtype:node.value===null?'null':undefined}
      }
      throw Error('unsupported argument expression')
    }
    function pureSyntax(node) {
      if(!node)return false
      if(['Identifier','ThisExpression'].includes(node.type))return true
      if(node.type==='Literal')return node.value===null||['string','number','boolean'].includes(typeof node.value)
      // Creating a function does not execute its parameters or body. In an
      // object method the computed property key is checked separately below.
      if(['FunctionExpression','ArrowFunctionExpression'].includes(node.type))return true
      if(node.type==='ArrayExpression')return node.elements.every(e=>e===null||pureSyntax(e))
      if(node.type==='ObjectExpression')return node.properties.every(p=>p.type==='Property'&&
        (!p.computed||p.key.type==='Literal'&&pureSyntax(p.key))&&pureSyntax(p.value))
      // These operators cannot invoke conversion hooks. Numeric conversion
      // is allowed only for a literal primitive, never an arbitrary object.
      if(node.type==='UnaryExpression')return ['!','void','typeof'].includes(node.operator)?pureSyntax(node.argument):
        ['+','-','~'].includes(node.operator)&&node.argument.type==='Literal'&&pureSyntax(node.argument)
      return false
    }
    function validatePureBindings(node) {
      if(['Identifier','ThisExpression'].includes(node.type)) {
        const v=value(node)
        if(!v||v.type==='undefined')throw Error('argument value unavailable')
      }else if(node.type==='ObjectExpression') {
        for(const p of node.properties)validatePureBindings(p.value)
      }else if(node.type==='ArrayExpression') {
        for(const e of node.elements)if(e)validatePureBindings(e)
      }else if(node.type==='UnaryExpression')validatePureBindings(node.argument)
      // Function bodies and default parameter expressions are intentionally
      // not visited: they are not evaluated while constructing the function.
    }
    try {
      if(!frame?.location?.scriptId||!nativeFunction)throw Error('missing native call frame')
      const s=source(frame.location.scriptId);callerSourceSha256=s.sha256
      const line=s.lines[frame.location.lineNumber]
      if(line===undefined)throw Error('caller location unavailable')
      const offset=line+frame.location.columnNumber
      const calls=s.calls.filter(c=>c.callee.start<=offset&&offset<=c.callee.end)
      if(calls.length!==1)throw Error('ambiguous native call location')
      const call=calls[0]
      // A second argument could mutate the first binding after it was
      // evaluated. Bound functions may inject a different first argument.
      if(call.optional||!call.arguments.length||(!pureAdditionalArguments&&call.arguments.length!==1))throw Error('unsupported native call arguments')
      if(pureAdditionalArguments) {
        // The target itself still needs a recoverable binding. Later
        // arguments may construct literals/functions without executing user
        // code or replacing that binding. Check all syntax before any reads.
        if(!['Identifier','ThisExpression','Literal'].includes(call.arguments[0].type)||
          call.arguments.some(arg=>!pureSyntax(arg)))throw Error('native argument evaluation may have side effects')
        for(const arg of call.arguments)validatePureBindings(arg)
      }
      const callee=value(call.callee)
      if(callee.type!=='function'||!callee.objectId||callee.subtype==='proxy')throw Error('native callee unavailable')
      const match=post('Runtime.callFunctionOn',{objectId:callee.objectId,
        functionDeclaration:'function(other){return this===other}',arguments:[{objectId:nativeFunction}],
        returnByValue:true,silent:true})
      if(match.exceptionDetails||match.result?.value!==true)throw Error('native callee identity differs')
      // A getter can return a proxy, replace itself with an ordinary data
      // property, and finish before this pause. The current descriptor is
      // therefore not evidence of the value evaluated as the argument.
      if(call.arguments[0].type==='MemberExpression')throw Error('argument property may have changed during evaluation')
      const arg=value(call.arguments[0])
      // An optimized-away binding can appear as undefined. Do not infer a
      // harmless primitive from an unavailable/undefined runtime binding.
      if(!arg||arg.type==='undefined')throw Error('argument value unavailable')
      const kind=arg.subtype==='proxy'?'proxy':arg.objectId?'object':'primitive'
      let target
      if(describeOwnAccessors&&kind==='object') {
        const accessors=properties(arg.objectId).filter(p=>p.isOwn===true&&(p.get?.type==='function'||p.set?.type==='function'))
        target={ownAccessorCount:accessors.length,
          watchedAccessorKeys:accessors.map(p=>p.name).filter(k=>['seed','tmp','invalidate'].includes(k))}
      }
      return {available:true,kind,...(target?{target}:{}),nativeIdentityMatched:true,callerSourceSha256,replayEligible:false}
    }catch(e){return {available:false,reason:e.message,callerSourceSha256,replayEligible:false}}
    finally {
      handles.delete(nativeFunction)
      for(const objectId of handles)try{post('Runtime.releaseObject',{objectId})}catch{}
    }
  }
}
