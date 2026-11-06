import {test} from 'node:test'
import assert from 'node:assert/strict'
import {fingerprint,observeWorkerInputs} from './worker-input-probe.mjs'

test('fingerprints preserve scalar distinctions, insertion order and graph identity',()=>{
  const vals=[undefined,null,false,0,-0,NaN,Infinity,-Infinity,'0',0n,[],{},Object.create(null)]
  assert.equal(new Set(vals.map(v=>fingerprint(v).hash)).size,vals.length)
  assert.notEqual(fingerprint({a:1,b:2}).hash,fingerprint({b:2,a:1}).hash)
  const a={};a.self=a;const b={};b.self=b
  assert.deepEqual(fingerprint(a),fingerprint(b))
  const shared={x:1}
  assert.notEqual(fingerprint([shared,shared]).hash,fingerprint([{x:1},{x:1}]).hash)
})
test('unknown values are explicit and accessors, proxies and serializers are not invoked',()=>{
  let calls=0
  const getter={get secret(){calls++;return 'private'}}
  const proxy=new Proxy({},{ownKeys(){calls++;return []}})
  for(const v of [getter,proxy,new Date(),new Map(),()=>{},Symbol(),{toJSON(){calls++}}]) {
    assert.equal(fingerprint(v).available,false)
  }
  assert.equal(calls,0)
})
test('worker packets retain results and errors, pair out-of-order responses and reveal omissions',()=>{
  class Worker {
    env={TOKEN:'test-secret-that-must-not-be-published'};execArgv=[]
    send(m){if(m.throw)throw m.throw;return m}
    deserialize(m){return m}
  }
  class Thread extends Worker {}
  const original=Worker.prototype.send
  const probe=observeWorkerInputs({ForksPoolWorker:Worker,ThreadsPoolWorker:Thread})
  const w=new Worker()
  const start={__vitest_worker_request__:true,type:'start',context:{config:{name:'unit'}}}
  assert.equal(w.send(start),start)
  w.send({__vitest_worker_request__:true,type:'run',context:{files:[{filepath:'/project/test.ts'}],workerId:1}})
  const req={t:'q',m:'fetch',a:['/project/test.ts'],i:'one'}
  assert.equal(w.deserialize(req),req)
  w.deserialize({t:'q',m:'getCountOfFailedTests',a:[],i:'two'})
  w.send({t:'s',i:'two',r:0});w.send({t:'s',i:'one',r:{tmp:'/tmp/module'}})
  w.send({t:'s',i:'missing',r:0})
  const error=Error('original loader failure')
  assert.throws(()=>w.send({throw:error}),e=>e===error)
  const report=probe.report(),s=report.workers[0]
  assert.equal(s.responses,2);assert.equal(s.pending,0)
  assert.equal(s.requests[0].method,'fetch');assert.ok(s.requests[0].response.value.available)
  assert.deepEqual(s.errors,['unmatched response']);assert.equal(s.unknown,1)
  assert.equal(report.replayEligible,false)
  assert.equal(JSON.stringify(report).includes('test-secret-that-must-not-be-published'),false)
  probe.close();assert.equal(Worker.prototype.send,original)
})
