// Complete per-specification plan for the separate matrix investigation.
// This module neither admits receipts nor substitutes native execution.
import fs from 'node:fs'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {fileURLToPath} from 'node:url'
import {canonicalManagedJSON} from './compiler.mjs'

const sha = value => crypto.createHash('sha256').update(value).digest('hex')
export const matrixInstanceKey = row => canonicalManagedJSON([row.project, row.file, row.pool || 'forks'])
export const matrixStateKey = row => canonicalManagedJSON([row.name, row.state])

export function createManagedMatrixPlan(suite) {
  assert(suite && Array.isArray(suite.selected) && suite.selected.length > 0 && suite.selected.length <= 128,
    'bounded nonempty specification selection required')
  const prepared = new Map(suite.prepared.map(row => [row.instanceId, row]))
  const declined = new Map(suite.declined.map(row => [row.instanceId, row]))
  assert.equal(prepared.size, suite.prepared.length)
  assert.equal(declined.size, suite.declined.length)
  const selectedIds = new Set(suite.selected.map(row => row.instanceId))
  assert.equal(selectedIds.size, suite.selected.length)
  assert([...prepared.keys()].every(id => selectedIds.has(id) && !declined.has(id)),
    'prepared IDs must be selected and disjoint from declined IDs')
  assert([...declined.keys()].every(id => selectedIds.has(id)), 'declined IDs must be selected')
  assert.equal(prepared.size + declined.size, selectedIds.size,
    'prepared and declined IDs must exactly cover selected IDs')
  const instances = suite.selected.map(selected => {
    const admitted = prepared.get(selected.instanceId), rejected = declined.get(selected.instanceId)
    assert(Boolean(admitted) !== Boolean(rejected), 'every specification needs exactly one preparation decision')
    const specification = {...selected, absolute: '/project/' + selected.file}
    if (admitted) return {instanceId: selected.instanceId, specification,
      decision: 'attempt-managed', preparation: admitted.identity}
    assert(typeof rejected.reason === 'string' && rejected.reason.length, 'decline reason required')
    return {instanceId: selected.instanceId, specification, decision: 'ordinary', reason: rejected.reason}
  }).sort((left, right) => left.instanceId.localeCompare(right.instanceId))
  const identity = {schema: 'quod-managed-matrix-plan-v1-research',
    plannerSha256: sha(fs.readFileSync(fileURLToPath(import.meta.url))),
    profile: suite.evidence.profile, instances,
    isolation: 'fresh workerd process and realm per actual project/file specification',
    proof: 'closed compilation plus private completion and exact per-instance native name/state comparison',
    failure: 'unsupported, incomplete or compile failure remains ordinary; supported actual test failure is sticky',
    receiptAdmission: false}
  return {schema: identity.schema, key: sha('quod-managed-matrix-plan-v1-research\n' + canonicalManagedJSON(identity)),
    identity, selected: suite.selected.length, prepared: prepared.size, declined: declined.size}
}

/** Compare multisets, including duplicate display names and skipped tests. */
export function compareManagedMatrixStates(managed, native) {
  const keys = rows => rows.map(matrixStateKey).sort()
  const managedKeys = keys(managed), nativeKeys = keys(native)
  const counts = rows => {
    const result = new Map()
    for (const row of rows) result.set(row, (result.get(row) || 0) + 1)
    return result
  }
  const left = counts(managedKeys), right = counts(nativeKeys)
  const differences = [...new Set([...left.keys(), ...right.keys()])].sort()
    .filter(key => left.get(key) !== right.get(key))
    .map(key => ({row: JSON.parse(key), managed: left.get(key) || 0, native: right.get(key) || 0}))
  return {equal: differences.length === 0, managedCount: managed.length, nativeCount: native.length,
    managedSha256: sha(canonicalManagedJSON(managedKeys)), nativeSha256: sha(canonicalManagedJSON(nativeKeys)), differences}
}
