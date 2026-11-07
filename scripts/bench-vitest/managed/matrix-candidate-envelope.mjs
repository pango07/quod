// Research transport only. A candidate routes a lookup; fresh compile-input
// validation and the complete current execution ledger still decide admission.
import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import {gzipSync, gunzipSync} from 'node:zlib'
import {canonicalReceiptJSON} from './receipt.mjs'

export const MATRIX_CANDIDATE_ENVELOPE_SCHEMA = 'quod-managed-matrix-input-candidate-v1-research'
const MAX_BYTES = 2 * 1024 * 1024
const json = canonicalReceiptJSON
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const digest = value => typeof value === 'string' && /^[0-9a-f]{64}$/.test(value)
const exact = (value, names) => assert(value && Object.getPrototypeOf(value) === Object.prototype &&
  Object.keys(value).length === names.length && names.every(name => Object.hasOwn(value, name)), 'candidate envelope fields differ')
const bytes = value => {
  assert(Buffer.isBuffer(value) && value.length > 0 && value.length <= MAX_BYTES, 'bounded exact candidate bytes required')
  assert(Buffer.from(value.toString('utf8')).equals(value), 'candidate UTF-8 differs')
  return value
}

export function makeMatrixCandidateKey({prepared, options, runtimePolicy, supervisorPolicy, host, nativePolicy}) {
  // No prior source list, bundle hash or cached execution is used for routing.
  // Changing any current noncompile execution input routes to another candidate.
  const current = {preparation: prepared.identity, options, runtimePolicy, supervisorPolicy, host, nativePolicy}
  return sha(MATRIX_CANDIDATE_ENVELOPE_SCHEMA + '\n' + json(current))
}

function receiptCompile(receiptBytes) {
  const receipt = JSON.parse(bytes(receiptBytes).toString('utf8'))
  assert(json(receipt) === receiptBytes.toString('utf8'), 'candidate receipt must retain exact canonical bytes')
  assert(receipt?.execution?.identity?.compile && typeof receipt.execution.identity.compile === 'object', 'candidate compile identity absent')
  return receipt.execution.identity.compile
}

export function encodeMatrixCandidateEnvelope({candidateKey, certificateBytes, receiptBytes}) {
  assert(digest(candidateKey), 'current candidate key required')
  const certificate = JSON.parse(bytes(certificateBytes).toString('utf8'))
  assert(Buffer.from(json(certificate) + '\n').equals(certificateBytes), 'candidate certificate must retain exact canonical bytes')
  assert(Object.hasOwn(certificate, 'compile'), 'candidate certificate lacks compile identity')
  const compile = receiptCompile(receiptBytes)
  assert.equal(json(certificate.compile), json(compile), 'certificate and receipt compile identities differ')
  const {compile: omitted, ...fields} = certificate
  const value = {schema: MATRIX_CANDIDATE_ENVELOPE_SCHEMA, candidateKey,
    certificate: {fields, sha256: sha(certificateBytes)},
    receipt: {encoding: 'gzip-base64', base64: gzipSync(receiptBytes, {level: 6}).toString('base64'),
      bytes: receiptBytes.length, sha256: sha(receiptBytes)}}
  const envelopeBytes = Buffer.from(json(value) + '\n')
  bytes(envelopeBytes)
  return {envelopeBytes, envelopeSha256: sha(envelopeBytes), receiptSha256: value.receipt.sha256,
    certificateSha256: value.certificate.sha256}
}

export function decodeMatrixCandidateEnvelope({candidateKey, envelopeBytes, trustedEnvelopeSha256}) {
  // This SHA must come from the authenticated collection reader. Embedded
  // receipt/certificate hashes alone never authorize any candidate bytes.
  assert(digest(candidateKey) && digest(trustedEnvelopeSha256), 'external authenticated candidate SHA required')
  assert.equal(sha(bytes(envelopeBytes)), trustedEnvelopeSha256, 'authenticated candidate bytes differ')
  const value = JSON.parse(envelopeBytes.toString('utf8'))
  assert(Buffer.from(json(value) + '\n').equals(envelopeBytes), 'noncanonical candidate envelope')
  exact(value, ['schema', 'candidateKey', 'certificate', 'receipt'])
  assert.equal(value.schema, MATRIX_CANDIDATE_ENVELOPE_SCHEMA)
  assert.equal(value.candidateKey, candidateKey, 'current candidate routing key differs')
  exact(value.certificate, ['fields', 'sha256']); exact(value.receipt, ['encoding', 'base64', 'bytes', 'sha256'])
  assert(digest(value.certificate.sha256) && digest(value.receipt.sha256), 'candidate component hashes required')
  assert.equal(value.receipt.encoding, 'gzip-base64', 'candidate receipt compression differs')
  assert(Number.isSafeInteger(value.receipt.bytes) && value.receipt.bytes > 0 && value.receipt.bytes <= MAX_BYTES, 'candidate raw receipt size bound')
  assert(typeof value.receipt.base64 === 'string' && value.receipt.base64.length <= Math.ceil(MAX_BYTES / 3) * 4, 'candidate receipt encoding bound')
  const compressed = Buffer.from(value.receipt.base64, 'base64')
  assert.equal(compressed.toString('base64'), value.receipt.base64, 'candidate receipt base64 differs')
  assert(compressed.length > 0 && compressed.length <= MAX_BYTES, 'candidate compressed receipt size bound')
  // Authenticate the complete compressed envelope before parsing or expanding;
  // neither a declared size nor a gzip header can enlarge this output bound.
  // Node 24.18 documents maxOutputLength for convenience methods:
  // https://nodejs.org/download/release/v24.18.0/docs/api/zlib.html#class-options
  const receiptBytes = gunzipSync(compressed, {maxOutputLength: MAX_BYTES})
  assert.equal(receiptBytes.length, value.receipt.bytes, 'candidate raw receipt size differs')
  assert.equal(sha(bytes(receiptBytes)), value.receipt.sha256, 'candidate receipt hash differs')
  const fields = value.certificate.fields
  assert(fields && Object.getPrototypeOf(fields) === Object.prototype && !Object.hasOwn(fields, 'compile'), 'candidate certificate header differs')
  const certificateBytes = Buffer.from(json({...fields, compile: receiptCompile(receiptBytes)}) + '\n')
  assert.equal(sha(bytes(certificateBytes)), value.certificate.sha256, 'candidate certificate reconstruction differs')
  return {certificateBytes, trustedCertificateSha256: value.certificate.sha256,
    receiptBytes, trustedReceiptSha256: value.receipt.sha256}
}
