import test from 'node:test'
import assert from 'node:assert/strict'
import vm from 'node:vm'
import {issuedCompilationSources,validateIssuedCompilationSources} from './issued-compilation-sources.mjs'
import {compiledScriptRecord} from './worker-module-probe.mjs'
import {compiledScriptLog,dictionaryCompiledScriptLog} from './compiled-script-log.mjs'
import {blockCompiledScriptLog} from './compiled-block-log.mjs'
import {inspectPromiseObservers} from './promise-observers-probe.mjs'

const record=source=>compiledScriptRecord({url:'',scriptLanguage:'JavaScript'},{scriptSource:source})
function setup(make=compiledScriptLog,limits) {
  const sources=issuedCompilationSources(limits),log=make()
  return {sources,log,observe(source){const r=record(source);log.append(r);sources.observe(r)},
    verify(){const s=sources.report();validateIssuedCompilationSources(s,log.report());return s}}
}

test('issued matching retains chronology when the same source appears before and after a command',()=>{
  for(const make of [compiledScriptLog,dictionaryCompiledScriptLog,()=>blockCompiledScriptLog({blockSize:2})]) {
    const p=setup(make);p.observe('42');p.sources.issue('Runtime.evaluate','42');p.observe('42');p.observe('43');p.observe('42')
    p.sources.issue('Runtime.callFunctionOn','42');p.sources.issue('Runtime.evaluate','42')
    const r=p.verify();assert.equal(r.anonymousEvents,4);assert.equal(r.matchedEvents,2);assert.equal(r.unmatchedEvents,2)
    assert.deepEqual(r.runs,[[0,false,1],[0,true,1],[1,false,1],[0,true,1]])
    assert.deepEqual(r.sources[0].issued,[{method:'Runtime.evaluate',calls:2},{method:'Runtime.callFunctionOn',calls:1}])
    assert.equal(r.issuedCalls,3);assert.equal(r.replayEligible,false);assert.equal(r.ownershipEstablished,false)
  }
})

test('source text matching is exact for UTF-16, whitespace and NUL',()=>{
  const p=setup(),text=['"a b"','"a\0b"','"\ud800"','"\ud801"']
  for(const s of [text[0],text[2]])p.sources.issue('Runtime.evaluate',s)
  for(const s of text)p.observe(s)
  const r=p.verify();assert.equal(r.sources.length,4);assert.equal(r.matchedEvents,2);assert.equal(r.unmatchedEvents,2)
  assert(!JSON.stringify(r).includes('a b'));assert.equal(new Set(r.sources.map(s=>s.code.sha256)).size,4)
})

test('a matching application compilation never becomes an ownership certificate',()=>{
  const p=setup();p.sources.issue('Debugger.setBreakpointOnFunctionCall','42')
  p.observe('42');p.observe('42')
  const r=p.verify();assert.equal(r.matchedEvents,2);assert.equal(r.ownershipEstablished,false);assert.equal(r.replayEligible,false)
})

test('issued but uncompiled commands and non-anonymous scripts do not fabricate anonymous events',()=>{
  const p=setup();p.sources.issue('Runtime.evaluate','42')
  const named=compiledScriptRecord({url:'file:///same-source.js'},{scriptSource:'42'});p.log.append(named);p.sources.observe(named)
  const r=p.verify();assert.equal(r.anonymousEvents,0);assert.equal(r.issuedCalls,1);assert.equal(r.sources[0].matchedEvents,0)
})

test('overflow is explicit for source, run and retained-text limits',()=>{
  for(const limits of [{sourceLimit:1},{runLimit:1},{byteLimit:2}]) {
    const p=setup(compiledScriptLog,limits)
    p.sources.issue('Runtime.evaluate','42');p.observe('42');p.observe('43')
    const r=p.sources.report();assert.equal(r.overflow,true);assert.throws(()=>validateIssuedCompilationSources(r,p.log.report()))
  }
})

test('independent validation rejects missing events, reordered runs, rewritten sources and invented authority',()=>{
  const p=setup();p.sources.issue('Runtime.evaluate','42');p.observe('42');p.observe('43')
  const good=p.verify()
  for(const change of [
    r=>{r.runs.pop()},r=>{r.runs.reverse()},r=>{r.sources[0].code.sha256='0'.repeat(64)},
    r=>{r.anonymousEvents++},r=>{r.issuedCalls++},r=>{r.ownershipEstablished=true},
    r=>{r.sources[0].issued=[]},r=>{r.sources[0].matchedEvents++},r=>{r.limits.sourceLimit=1},
  ]) {const r=structuredClone(good);change(r);assert.throws(()=>validateIssuedCompilationSources(r,p.log.report()))}
})

test('reports are snapshots and invalid commands cannot register source',()=>{
  const p=setup();assert.throws(()=>p.sources.issue('Runtime.getProperties','42'));assert.throws(()=>p.sources.issue('Runtime.evaluate',42))
  p.sources.issue('Runtime.evaluate','42');p.observe('42')
  const r=p.verify();r.sources[0].code.sha256='0'.repeat(64);r.runs[0][2]=999;r.sources[0].issued[0].calls=999
  assert.equal(p.verify().matchedEvents,1)
})

test('live Inspector command sources reconcile with the unchanged complete compilation log',()=>{
  const registry=inspectPromiseObservers({compiledScripts:true})
  try {
    assert.equal(new Function('return 432107')(),432107)
    assert.equal(vm.runInThisContext('432108',{filename:'issued-source-named-fixture'}),432108)
    const r=registry.compiledScriptReport();assert.deepEqual(r.errors,[])
    validateIssuedCompilationSources(r.issuedSources,r)
    assert(r.issuedSources.matchedEvents>0);assert(r.issuedSources.unmatchedEvents>0)
    // Node 24.18's CreateDynamicFunction compiles the enclosing expression,
    // including parentheses, rather than Function.prototype.toString output.
    const code=record('(function anonymous(\n) {\nreturn 432107\n})').code
    const anonymous=r.issuedSources.sources.find(s=>s.code.sha256===code.sha256)
    assert(anonymous&&anonymous.unmatchedEvents===1&&!anonymous.issued.length)
  }finally{registry.close()}
})
