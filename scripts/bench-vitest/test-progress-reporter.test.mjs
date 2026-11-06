import test from 'node:test'
import assert from 'node:assert/strict'
import {testProgressReporter} from './test-progress-reporter.mjs'

const task=(id,project='first',state='passed')=>({id,fullName:'suite > '+id,
  module:{moduleId:'/project/shared.test.ts'},project:{config:{name:project}},result:()=>({state})})

test('incremental receipts retain interleaved project identities and final states',()=>{
  const rows=[],p=testProgressReporter('/project',r=>rows.push(r),{now:(()=>{let n=0;return()=>n++})()})
  const a=task('a'),b=task('b','second','skipped'),c=task('c','first','failed')
  // A fast ready callback can already expose a completed result. The event
  // remains ready, rather than inferring completion from that mutable view.
  p.reporter.onTestCaseReady(a);p.reporter.onTestCaseReady(b)
  p.reporter.onTestCaseResult(b);p.reporter.onTestCaseReady(c)
  p.reporter.onTestCaseResult(a);p.reporter.onTestCaseResult(c)
  p.reporter.onTestRunEnd([{},{}],[],'failed')
  assert.deepEqual(rows.map(r=>[r.event,r.id??null,r.project??null,r.state??null]),[
    ['ready','a','first',null],['ready','b','second',null],['result','b','second','skipped'],
    ['ready','c','first',null],['result','a','first','passed'],['result','c','first','failed'],
    ['run-end',null,null,null]])
  assert.deepEqual(rows.map(r=>r.sequence),[0,1,2,3,4,5,6])
  assert.deepEqual(rows.map(r=>r.elapsedMs),[1,2,3,4,5,6,7])
  assert(rows.slice(0,6).every(r=>r.file==='shared.test.ts'))
  assert.equal(p.report().results,3);assert.equal(p.report().ready,3)
  assert.equal(p.report().ended,true);assert.equal(p.report().failure,null)
  assert.equal(p.report().replayEligible,false)
})

test('a partial phase retains started and completed cases without claiming an end',()=>{
  const rows=[],p=testProgressReporter('/project',r=>rows.push(r))
  p.reporter.onTestCaseReady(task('finished'));p.reporter.onTestCaseResult(task('finished'))
  p.reporter.onTestCaseReady(task('still-running'))
  assert.equal(rows.length,3);assert.equal(p.report().ended,false)
  assert.equal(p.report().ready,2);assert.equal(p.report().results,1)
})

test('bounded storage, invalid callbacks and write failures remain explicitly incomplete',()=>{
  const bounded=testProgressReporter('/project',()=>{},{eventLimit:1})
  bounded.reporter.onTestCaseReady(task('a'));bounded.reporter.onTestCaseResult(task('a'))
  assert.equal(bounded.report().events,1);assert.equal(bounded.report().overflow,true)
  const failed=testProgressReporter('/project',()=>{throw Error('private writer detail')})
  failed.reporter.onTestCaseReady(task('a'))
  assert.equal(failed.report().failure,'progress write failed')
  assert(!JSON.stringify(failed.report()).includes('private writer detail'))
  const invalid=testProgressReporter('/project',()=>{})
  invalid.reporter.onTestCaseResult(task('a','first','pending'))
  assert.equal(invalid.report().events,0);assert.equal(invalid.report().failure,'unsupported progress result')
})
