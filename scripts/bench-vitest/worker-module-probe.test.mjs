import test from 'node:test'
import assert from 'node:assert/strict'
import vm from 'node:vm'
import {createHash} from 'node:crypto'
import {fingerprint} from './worker-input-probe.mjs'
import {observeCompiledScripts,observeModuleEvaluation} from './worker-module-probe.mjs'

const sha=s=>createHash('sha256').update(s,'utf16le').digest('hex')
test('observes exact evaluator code, preserves promise/throw identity, and restores methods',()=>{
  const promise=Promise.resolve(7),failure=Error('original failure')
  class Evaluator {
    runInlinedModule(context,code,module){assert.equal(this.tag,1);return promise}
    runExternalModule(id){throw failure}
  }
  const original=Evaluator.prototype.runInlinedModule,p=observeModuleEvaluation(Evaluator)
  const e=new Evaluator();e.tag=1
  const code='private-source-literal',module={id:'/module',file:'/module',url:'/module',meta:{tmp:'/temporary'}}
  assert.equal(e.runInlinedModule({},code,module),promise)
  assert.throws(()=>e.runExternalModule('/external'),error=>error===failure)
  const report=p.report()
  assert.deepEqual(report.errors,[]);assert.equal(report.events[0].code.sha256,sha(code))
  assert.deepEqual(report.events[0].metaFields,{available:true,fields:{tmp:fingerprint('/temporary')}})
  assert.equal(report.events[1].kind,'external');assert(!JSON.stringify(report).includes(code))
  assert.equal(report.replayEligible,false)
  p.close();assert.equal(Evaluator.prototype.runInlinedModule,original)
})
test('unknown module identity is explicit without invoking diagnostic getters or proxy traps',()=>{
  let calls=0
  class Evaluator {runInlinedModule(){return 11}runExternalModule(){return 12}}
  const p=observeModuleEvaluation(Evaluator),e=new Evaluator()
  assert.equal(e.runInlinedModule({},'code',{get id(){calls++;return 'private'}}),11)
  assert.equal(e.runInlinedModule({},'code',new Proxy({},{getOwnPropertyDescriptor(){calls++;return undefined}})),11)
  assert.equal(calls,0);assert.equal(p.report().errors.length,2);assert.equal(p.report().events.length,0)
  p.close()
})
test('V8 source hashes retain exact bytes when vm scripts are bound, without claiming complete compilation coverage',()=>{
  const p=observeCompiledScripts(),url='quod-compiled-module-fixture',a='1 + 1',b='40 + 2'
  const first=new vm.Script(a,{filename:url}),second=new vm.Script(b,{filename:url})
  assert.equal(first.runInThisContext(),2)
  const expected=fingerprint({url,language:'JavaScript',isModule:false,hasSourceURL:false,failedParse:false}).hash
  const hashes=()=>p.report().scripts.filter(s=>s.identity.hash===expected).map(s=>s.code.sha256)
  // Node 24's Inspector does not report the unbound second vm.Script yet.
  // An error-free observation must not be interpreted as complete coverage.
  assert.deepEqual(hashes(),[sha(a)])
  assert.equal(second.runInThisContext(),42)
  assert.deepEqual(hashes(),[sha(a),sha(b)])
  const report=p.report();p.close()
  assert.deepEqual(report.errors,[])
  assert.equal(report.completeCompilationCoverage,false)
  assert.equal(report.replayEligible,false)
})
test('failed compilation is observed explicitly',()=>{
  const p=observeCompiledScripts(),code='const = invalid'
  assert.throws(()=>new vm.Script(code,{filename:'quod-invalid-module-fixture'}),SyntaxError)
  const report=p.report();p.close()
  assert.equal(report.failedToParse,1)
  assert(report.scripts.some(s=>s.code.sha256===sha(code))||report.errors.length>0)
  assert.equal(report.replayEligible,false)
})
test('distinct lone-surrogate source strings remain distinct in evaluator and debugger observations',()=>{
  const a='"\ud800".charCodeAt(0)',b='"\ud801".charCodeAt(0)'
  assert.deepEqual(Buffer.from(a),Buffer.from(b),'UTF-8 would lose the distinction')
  class Evaluator {
    runInlinedModule(context,code){return vm.runInThisContext(code,{filename:'quod-surrogate-module-fixture'})}
    runExternalModule(){}
  }
  const evaluator=observeModuleEvaluation(Evaluator),compiled=observeCompiledScripts()
  try {
    const instance=new Evaluator(),module={id:'/surrogate',meta:{}}
    assert.equal(instance.runInlinedModule({},a,module),0xd800)
    assert.equal(instance.runInlinedModule({},b,module),0xd801)
    const codes=evaluator.report().events.map(e=>e.code)
    assert.deepEqual(codes.map(c=>c.sha256),[sha(a),sha(b)])
    assert(codes.every(c=>c.encoding==='utf16le'&&c.bytes===a.length*2))
    const expected=fingerprint({url:'quod-surrogate-module-fixture',language:'JavaScript',isModule:false,hasSourceURL:false,failedParse:false}).hash
    const scripts=compiled.report().scripts.filter(s=>s.identity.hash===expected)
    assert.deepEqual(scripts.map(s=>s.code.sha256),[sha(a),sha(b)])
    assert.deepEqual(compiled.report().errors,[])
    assert.equal(compiled.report().replayEligible,false)
  }finally{evaluator.close();compiled.close()}
})
test('Wasm observations hash bytecode rather than the empty JavaScript source',()=>{
  // (module (func (export "f") (result i32) i32.const 41))
  const a=Buffer.from([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,7,5,1,1,102,0,0,10,6,1,4,0,65,41,11])
  const b=Buffer.from(a);b[b.length-2]=42
  const p=observeCompiledScripts()
  try {
    assert.equal(new WebAssembly.Instance(new WebAssembly.Module(a)).exports.f(),41)
    assert.equal(new WebAssembly.Instance(new WebAssembly.Module(b)).exports.f(),42)
    const report=p.report(),codes=report.scripts.filter(s=>s.language==='WebAssembly').map(s=>s.code)
    for(const bytes of [a,b])assert(codes.some(c=>c.encoding==='bytes'&&c.bytes===bytes.length&&c.sha256===createHash('sha256').update(bytes).digest('hex')))
    assert.deepEqual(report.errors,[])
    assert.equal(report.completeCompilationCoverage,false)
    assert.equal(report.replayEligible,false)
  }finally{p.close()}
})
