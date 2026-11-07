// Called only by the remote dispatcher fixtures while the original assertions
// and their appended controls are present. Synthetic hashes are test authority.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import assert from 'node:assert/strict'
import {gzipSync} from 'node:zlib'
import {canonicalReceiptJSON} from './receipt.mjs'
import {decodeMatrixCandidateEnvelope} from './matrix-candidate-envelope.mjs'
import {validateResolvedCompileCertificates, readValidatedCompileIdentity} from './resolved-compile-certificate.mjs'
import {makeMatrixExecutionIdentityFromProof} from './matrix-receipt.mjs'
import {prepareManagedMatrix} from './preparation.mjs'

const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
const clone = value => JSON.parse(canonicalReceiptJSON(value))

export async function verifyMatrixInputProofFixtures({project, seedOut, selected, out}) {
  const result = {schema: 'quod-managed-matrix-input-proof-fixtures-v1', passed: false, checks: [],
    actualSdkAuthenticationClaimed: false}
  const check = (name, pass) => {result.checks.push({name, pass: Boolean(pass)}); assert(pass, name)}
  const rejects = async (name, operation) => {
    let rejected = false
    try {await operation()} catch {rejected = true}
    check(name, rejected)
  }
  try {
    const seeded = read(path.join(seedOut, 'result.json'))
    const row = seeded.instances.find(row => row.candidate)
    const directory = path.join(seedOut, row.evidenceDirectory)
    const envelopeBytes = fs.readFileSync(path.join(directory, 'recorded-candidate.json'))
    const request = {candidateKey: row.candidateKey, envelopeBytes, trustedEnvelopeSha256: sha(envelopeBytes)}
    const decoded = decodeMatrixCandidateEnvelope(request)
    check('compressed envelope reconstructs exact original receipt bytes', decoded.receiptBytes.equals(
      fs.readFileSync(path.join(seedOut, 'receipts', row.instanceId + '.json'))))
    check('deduplicated envelope reconstructs exact original certificate bytes', decoded.certificateBytes.equals(
      fs.readFileSync(path.join(directory, 'compile-certificate.json'))))
    const mutate = transform => {
      const value = JSON.parse(envelopeBytes.toString('utf8')); transform(value)
      const body = Buffer.from(canonicalReceiptJSON(value) + '\n')
      return {...request, envelopeBytes: body, trustedEnvelopeSha256: sha(body)}
    }
    await rejects('an embedded digest cannot replace external envelope authority', () => decodeMatrixCandidateEnvelope({...request,
      trustedEnvelopeSha256: undefined}))
    await rejects('wrong external envelope authority rejects intact compressed bytes', () => decodeMatrixCandidateEnvelope({...request,
      trustedEnvelopeSha256: '0'.repeat(64)}))
    await rejects('another current routing key rejects authenticated candidate bytes', () => decodeMatrixCandidateEnvelope({...request,
      candidateKey: '0'.repeat(64)}))
    await rejects('authenticated truncated gzip rejects before any receipt lookup', () => decodeMatrixCandidateEnvelope(mutate(value => {
      const compressed = Buffer.from(value.receipt.base64, 'base64')
      value.receipt.base64 = compressed.subarray(0, compressed.length - 8).toString('base64')
    })))
    await rejects('authenticated false expanded length rejects', () => decodeMatrixCandidateEnvelope(mutate(value => {value.receipt.bytes++})))
    await rejects('authenticated decompression cannot exceed the independent output cap', () => decodeMatrixCandidateEnvelope(mutate(value => {
      value.receipt.base64 = gzipSync(Buffer.alloc(2 * 1024 * 1024 + 1, 65)).toString('base64')
      value.receipt.bytes = 2 * 1024 * 1024
    })))
    await rejects('authenticated reconstructed certificate SHA mismatch rejects', () => decodeMatrixCandidateEnvelope(mutate(value => {
      value.certificate.sha256 = '0'.repeat(64)
    })))
    await rejects('authenticated ambiguous envelope JSON rejects', () => {
      const body = Buffer.from('{"schema":"duplicate",' + envelopeBytes.toString('utf8').slice(1))
      return decodeMatrixCandidateEnvelope({...request, envelopeBytes: body, trustedEnvelopeSha256: sha(body)})
    })
    const built = read(path.join(directory, 'test-build.json'))
    const actual = read(path.join(directory, 'execution-identity.json'))
    const suite = await prepareManagedMatrix({project, seed: 42, files: selected})
    const prepared = suite.prepared.find(value => value.instanceId === row.instanceId)
    const decisions = await validateResolvedCompileCertificates({project, requests: [{
      certificateBytes: decoded.certificateBytes, trustedCertificateSha256: decoded.trustedCertificateSha256,
      options: built.options, testFile: prepared.absolute}]})
    check('a real fresh source validation issues an opaque frozen proof', decisions.length === 1 &&
      decisions[0].hit && Object.isFrozen(decisions[0].proof))
    const proof = decisions[0].proof
    const argumentsFor = value => ({proof: value, options: built.options, prepared,
      runtimePolicy: actual.identity.runtimePolicy, supervisorPolicy: actual.identity.supervisorPolicy,
      host: actual.identity.host, nativePolicy: actual.identity.nativePolicy})
    const fromProof = await makeMatrixExecutionIdentityFromProof(argumentsFor(proof))
    check('validated proof and actual full bundle produce the exact same complete execution identity and key',
      canonicalReceiptJSON(fromProof) === canonicalReceiptJSON(actual))
    const returned = readValidatedCompileIdentity(proof)
    returned.identity.bundleSha256 = '0'.repeat(64)
    check('mutating a diagnostic identity copy cannot mutate the private proof',
      readValidatedCompileIdentity(proof).bundleSha256 === actual.identity.compile.bundleSha256 &&
      readValidatedCompileIdentity(proof).identity.bundleSha256 === actual.identity.compile.bundleSha256)
    for (const [name, value] of [['plain object', {}], ['shallow copy', {...proof}], ['serialized token', clone(proof)]]) {
      await rejects(name + ' cannot replace a real module-issued compile proof', () => makeMatrixExecutionIdentityFromProof(argumentsFor(value)))
    }
    await rejects('real compile proof cannot authorize changed current options', () => makeMatrixExecutionIdentityFromProof({
      ...argumentsFor(proof), options: {...built.options, seed: built.options.seed + 1}}))
    await rejects('real compile proof cannot authorize changed current preparation', () => makeMatrixExecutionIdentityFromProof({
      ...argumentsFor(proof), prepared: {...prepared, config: {...prepared.config, isolate: false}}}))
    result.passed = true
    return result
  } finally {
    fs.writeFileSync(out, JSON.stringify(result, null, 2) + '\n')
  }
}
