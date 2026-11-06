// Diagnostic only. Recover a bounded native-call argument from the actual
// paused script and Inspector data descriptors. An optional diagnostic uses
// side-effect-checked evaluation of a bare, declared module identifier only.
// Arbitrary expressions/getters and unknown syntax/scope/identity stay unknown.
import {parse} from '../../crates/quod-cli/src/vitest/acorn.mjs'
import {createHash} from 'node:crypto'

export function observeNativeCallArguments(post,{identityComparison=null,moduleBindingEvaluation=false,moduleBindingLookup=false}={}) {
  const sources=new Map()
  const propertyRequests={},propertyEntries={}
  let skippedModuleScopes=0,moduleBindingEvaluations=0,moduleBindingEvaluationErrors=0,moduleBindingLookups=0,moduleBindingLookupErrors=0
  let compiledIdentityComparisons=0
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
    const calls=[],lines=[0],declaredNames=new Set(),moduleNames=new Set()
    for(let i=0;i<text.length;i++)if(text[i]==='\n')lines.push(i+1)
    function pattern(node,names=declaredNames) {
      if(!node)return
      if(node.type==='Identifier')names.add(node.name)
      else if(node.type==='RestElement')pattern(node.argument,names)
      else if(node.type==='AssignmentPattern')pattern(node.left,names)
      else if(node.type==='ArrayPattern')node.elements.forEach(n=>pattern(n,names))
      else if(node.type==='ObjectPattern')for(const p of node.properties)pattern(p.type==='RestElement'?p.argument:p.value,names)
    }
    function visit(node) {
      if(!node||typeof node!=='object')return
      if(node.type==='CallExpression')calls.push(node)
      // Overapproximate bindings, including nested scopes. False positives
      // only cause an ordinary descriptor read. Strict ES modules cannot
      // acquire extra module bindings through direct eval. Never apply this
      // exclusion to a script, closure, local, or unknown scope.
      if(node.type==='VariableDeclarator')pattern(node.id)
      if(['FunctionDeclaration','FunctionExpression','ArrowFunctionExpression'].includes(node.type)) {
        pattern(node.id);node.params.forEach(n=>pattern(n))
      }
      if(['ClassDeclaration','ClassExpression'].includes(node.type))pattern(node.id)
      if(node.type==='ImportDeclaration')for(const specifier of node.specifiers)pattern(specifier.local)
      if(node.type==='CatchClause')pattern(node.param)
      for(const value of Object.values(node)) {
        if(Array.isArray(value))value.forEach(visit)
        else if(value&&typeof value==='object')visit(value)
      }
    }
    visit(tree)
    if(tree.sourceType==='module')for(const statement of tree.body) {
      const declaration=['ExportNamedDeclaration','ExportDefaultDeclaration'].includes(statement.type)?statement.declaration:statement
      if(!declaration)continue
      if(declaration.type==='VariableDeclaration')for(const d of declaration.declarations)pattern(d.id,moduleNames)
      if(['FunctionDeclaration','ClassDeclaration'].includes(declaration.type))pattern(declaration.id,moduleNames)
      if(declaration.type==='ImportDeclaration'&&!declaration.phase)for(const specifier of declaration.specifiers)pattern(specifier.local,moduleNames)
    }
    const result={calls,lines,declaredNames,moduleNames,
      locations:new Map(),sha256:createHash('sha256').update(text).digest('hex')}
    sources.set(scriptId,result);return result
  }
  const observe=(frame,nativeFunction,{pureAdditionalArguments=false,describeOwnAccessors=false}={})=>{
    const handles=new Set(),pauseProperties=new Map()
    let callerSourceSha256=null,evaluatedBinding=false
    function properties(id,kind='object') {
      if(!id)throw Error('missing runtime object')
      // In native identity mode this invocation uses descriptor reads and
      // native ID queries only. Reuse a paused scope's descriptors within
      // this invocation, never across calls/resumes or JS identity evaluation.
      if(identityComparison&&!evaluatedBinding&&pauseProperties.has(id))return pauseProperties.get(id)
      const r=post('Runtime.getProperties',{objectId:id,ownProperties:true,generatePreview:false})
      if(r.exceptionDetails)throw Error('runtime descriptors unavailable')
      propertyRequests[kind]=(propertyRequests[kind]??0)+1
      propertyEntries[kind]=(propertyEntries[kind]??0)+(r.result?.length??0)
      if(!releaseOnResume)for(const p of [...r.result??[],...r.internalProperties??[],...r.privateProperties??[]]) {
        for(const v of [p.value,p.get,p.set])if(v?.objectId)handles.add(v.objectId)
      }
      if(identityComparison&&!evaluatedBinding)pauseProperties.set(id,r.result)
      return r.result
    }
    function data(p) {
      if(!p||p.get||p.set||!p.value)throw Error('argument is not an own data property')
      return p.value
    }
    function binding(name) {
      for(const scope of frame.scopeChain) {
        if(!['local','block','closure','module','script','catch'].includes(scope.type))throw Error('unsupported argument scope')
        if(scope.type==='module'&&typeof scope.startLocation?.scriptId==='string') {
          const module=source(scope.startLocation.scriptId)
          if(!module.declaredNames.has(name)){skippedModuleScopes++;continue}
          if(moduleBindingLookup&&releaseOnResume&&module.moduleNames.has(name)) {
            // In pinned V8, ScopeIterator::ScopeObject creates a fresh
            // null-prototype object containing lexical values. The only
            // accessors it installs represent unavailable bindings and throw.
            // Read one binding from that fresh scope object, never from an
            // application object, and never retain its value across pauses.
            // Side-effect checks and any exception fail closed. No globals,
            // user expressions or application getters are evaluated here.
            evaluatedBinding=true;pauseProperties.clear();moduleBindingLookups++
            try {
              const r=post('Runtime.callFunctionOn',{objectId:scope.object.objectId,
                functionDeclaration:'function(name){ return this[name] }',arguments:[{value:name}],
                throwOnSideEffect:true,silent:true,returnByValue:false,objectGroup:'backtrace'})
              if(r.exceptionDetails||!r.result?.type)throw Error('module scope lookup unavailable')
              return r.result
            }catch(e){moduleBindingLookupErrors++;throw e}
          }
          // Only a declared module binding qualifies, after checking every
          // preceding supported scope. No global/member/property expression
          // is evaluated. Module bindings are lexical data/live imports, not
          // object getters. Keep this experiment pinned to the tested runtime.
          if(moduleBindingEvaluation&&releaseOnResume&&module.moduleNames.has(name)) {
            evaluatedBinding=true;pauseProperties.clear();moduleBindingEvaluations++
            try {
              const r=post('Debugger.evaluateOnCallFrame',{callFrameId:frame.callFrameId,expression:name,
                throwOnSideEffect:true,silent:true,returnByValue:false,objectGroup:'backtrace'})
              if(r.exceptionDetails||!r.result?.type)throw Error('module binding evaluation unavailable')
              return r.result
            }catch(e){moduleBindingEvaluationErrors++;throw e}
          }
        }
        const p=properties(scope.object.objectId,scope.type).find(p=>p.name===name)
        if(p)return data(p)
      }
      throw Error('argument binding unavailable')
    }
    function value(node) {
      if(node.type==='Identifier')return binding(node.name)
      if(node.type==='ThisExpression')return frame.this
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
      // Cache syntax lookup only. Repeated calls from a library helper must
      // still resolve their current bindings and target on every pause.
      let calls=s.locations.get(offset)
      if(!calls){calls=s.calls.filter(c=>c.callee.start<=offset&&offset<=c.callee.end);s.locations.set(offset,calls)}
      if(calls.length!==1)throw Error('ambiguous native call location')
      const call=calls[0]
      // A property getter can return a bound intrinsic and replace itself
      // (or its owner binding) with the unbound intrinsic before this pause.
      // Matching the current property would then assign the wrong argument
      // positions. Only a supported lexical binding or `this` is recoverable.
      if(!['Identifier','ThisExpression'].includes(call.callee.type))throw Error('native callee expression may have changed during evaluation')
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
      if(identityComparison) {
        const match=identityComparison(callee.objectId,nativeFunction)
        if(!match.available)throw Error(match.reason??'native identity comparison unavailable')
        if(match.same!==true)throw Error('native callee identity differs')
      }else {
        compiledIdentityComparisons++
        const match=post('Runtime.callFunctionOn',{objectId:callee.objectId,
          functionDeclaration:'function(other){return this===other}',arguments:[{objectId:nativeFunction}],
          returnByValue:true,silent:true})
        if(match.exceptionDetails||match.result?.value!==true)throw Error('native callee identity differs')
      }
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
  observe.diagnostics=()=>({propertyRequests:{...propertyRequests},propertyEntries:{...propertyEntries},
    skippedModuleScopes,moduleBindingEvaluation,moduleBindingEvaluations,moduleBindingEvaluationErrors,
    moduleBindingLookup,moduleBindingLookups,moduleBindingLookupErrors,compiledIdentityComparisons,replayEligible:false})
  return observe
}
