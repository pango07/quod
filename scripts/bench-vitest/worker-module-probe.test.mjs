import test from 'node:test'
import assert from 'node:assert/strict'
import vm from 'node:vm'
import {createHash} from 'node:crypto'
import {fingerprint} from './worker-input-probe.mjs'
import {observeCompiledScripts,observeModuleEvaluation} from './worker-module-probe.mjs'

const sha=s=>createHash('sha256').update(s).digest('hex')
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
test('V8 source hashes retain actual compiled bytes, including compiled but unexecuted scripts',()=>{
  const p=observeCompiledScripts(),url='quod-compiled-module-fixture',a='1 + 1',b='40 + 2'
  const first=new vm.Script(a,{filename:url}),second=new vm.Script(b,{filename:url})
  assert.equal(first.runInThisContext(),2)
  const expected=fingerprint({url,isModule:false,hasSourceURL:false,failedParse:false}).hash
  const report=p.report();p.close()
  assert.deepEqual(report.errors,[])
  const scripts=report.scripts.filter(s=>s.identity.hash===expected)
  assert.deepEqual(scripts.map(s=>s.code.sha256),[sha(a),sha(b)])
  assert.equal(report.replayEligible,false)
  assert.equal(second.runInThisContext(),42)
})
test('failed compilation is observed explicitly',()=>{
  const p=observeCompiledScripts(),code='const = invalid'
  assert.throws(()=>new vm.Script(code,{filename:'quod-invalid-module-fixture'}),SyntaxError)
  const report=p.report();p.close()
  assert.equal(report.failedToParse,1)
  assert(report.scripts.some(s=>s.code.sha256===sha(code))||report.errors.length>0)
  assert.equal(report.replayEligible,false)
})
