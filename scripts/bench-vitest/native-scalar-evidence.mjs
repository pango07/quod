// Validate the declared scope of direct scalar observation. This is not a
// certificate of complete worker inputs or replay safety.
import assert from 'node:assert/strict'

export function validateNativeScalarEvidence(scalar,events=scalar.events) {
  assert.equal(scalar.observation,'native-data')
  assert(scalar.installed&&scalar.intact&&scalar.ownerGuardRequired&&scalar.ownerGuardPassed)
  const n=scalar.native
  assert(n?.installed&&n.intact&&n.available&&!n.failed&&!n.closed&&!n.replayEligible,
    'native scalar callback must remain intact and available')
  assert(Number.isSafeInteger(n.calls)&&n.calls>=0)
  assert.equal(scalar.recordedReads,n.calls,'every native callback must have a recorded read')
  assert(Array.isArray(events))
  let calls=0
  for(const e of events) {
    assert.equal(e.operation,'get')
    assert(Number.isSafeInteger(e.count)&&e.count>0)
    calls+=e.count
    assert(Array.isArray(e.sites)&&e.sites.length>0)
    assert(Array.isArray(e.sources)&&e.sources.length===e.sites.length,
      'every retained caller frame must have source evidence')
    assert.deepEqual(e.source,e.sources[0])
    for(let i=0;i<e.sites.length;i++) {
      const source=e.sources[i]
      assert(source?.available&&!source.replayEligible)
      assert.equal(source.scriptId,e.sites[i].scriptId,'source must name the actual caller script')
      assert.equal(source.code?.encoding,'utf16le')
      assert(Number.isSafeInteger(source.code.bytes)&&source.code.bytes>=0)
      assert(/^[0-9a-f]{64}$/.test(source.code.sha256))
    }
  }
  assert.equal(calls,n.calls,'native scalar events must account for every read exactly once')
}
