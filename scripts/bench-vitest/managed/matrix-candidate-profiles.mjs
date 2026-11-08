// Finite research selection only. A profile never authenticates cached data or
// changes the actual Vitest specification selection. Default108 stays intact.
import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import {MATRIX_CANDIDATE_POLICY, MATRIX_CANDIDATE_ROSTER, selectMatrixCandidates} from './matrix-candidates.mjs'

export const PROPERTY47_CANDIDATE_PROFILE = 'reviewed-original-property47-single-instance-v1-research'
export const PROPERTY47_CANDIDATE = Object.freeze({
  instanceId: 'ccda7135f8d1ede102763427b3efabcef8130aaed24822ce92a5ec7ef94a05b3',
  project: 'convex-helpers',
  file: 'packages/convex-helpers/server/stream.property.test.ts',
  pool: 'forks',
})
const propertyRoster = Object.freeze([PROPERTY47_CANDIDATE])
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const rosterJSON = rows => JSON.stringify(rows.map(row =>
  [row.instanceId, row.project, row.file, row.pool]).sort((left, right) => left[0].localeCompare(right[0])))

/** Only a primitive closed profile name is accepted, never a caller roster. */
export function resolveMatrixCandidateProfile(label = MATRIX_CANDIDATE_POLICY) {
  assert(typeof label === 'string' && [MATRIX_CANDIDATE_POLICY, PROPERTY47_CANDIDATE_PROFILE].includes(label),
    'unknown finite matrix candidate profile')
  const roster = label === MATRIX_CANDIDATE_POLICY ? MATRIX_CANDIDATE_ROSTER : propertyRoster
  return Object.freeze({schema: 'quod-managed-matrix-candidate-profile-v1-research', label, roster,
    rosterSha256: sha(rosterJSON(roster))})
}

export function selectMatrixProfileCandidates(selected, label = MATRIX_CANDIDATE_POLICY) {
  const profile = resolveMatrixCandidateProfile(label)
  if (profile.label === MATRIX_CANDIDATE_POLICY) return selectMatrixCandidates(selected)
  assert(Array.isArray(selected), 'actual specification list required')
  const matches = selected.filter(row => row.instanceId === PROPERTY47_CANDIDATE.instanceId)
  assert(matches.length <= 1, 'property profile contains a duplicate actual instance')
  for (const row of matches) for (const key of ['project', 'file', 'pool']) {
    assert.equal(row[key], PROPERTY47_CANDIDATE[key], 'property candidate identity differs: ' + key)
  }
  return matches
}
