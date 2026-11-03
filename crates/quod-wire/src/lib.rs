//! quod.v1: the protocol one ledger speaks to its runners, whoever hosts
//! it (a self-hosted quod-server, or Quod Cloud). docs/cloud-ledger.md is
//! the prose version.
//!
//! Encoding: JSON bodies over HTTPS, paths under `/v1`, every request and
//! response carrying `quod-protocol: quod.v1`. JSON because every value
//! here already has one canonical JSON form (manifests, results and keys
//! are hashed as canonical JSON, `quod_cas::canonical_json`), so the wire
//! adds no second encoding to keep in step, a person can read a captured
//! exchange, and the dashboard (TypeScript) reads the same bodies. Blobs
//! are never JSON: they go as raw bytes under their hash.
//!
//! Versioning: the path and header name the major version. A server
//! answers a request whose `quod-protocol` it does not speak with 400 and
//! `{"error":"protocol"}`. Within v1, fields are only ever added, each
//! with a default, and readers ignore fields they do not know; a change
//! that would make an old client replay something it should not is v2.
//!
//! The trust decisions are not made here: the server runs quod-trust's
//! transitions on its own stores, inside its own transactions. A client
//! sends what it observed (the tree, the manifest it read, the result it
//! produced) and is told what to do.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use quod_cas::{Entry, Hash};
use quod_ledger::RunRecord;
use quod_manifest::{Manifest, Mismatch, StepResult};
use quod_trust::{Claimed, Policy, Reason, Step, Verdict};
use serde::{Deserialize, Serialize};

#[cfg(feature = "client")]
pub mod client;
pub mod testreplay;
#[cfg(feature = "client")]
pub mod testreplay_client;

/// The protocol name, sent as the `quod-protocol` header.
pub const PROTOCOL: &str = "quod.v1";
/// The header that carries it.
pub const PROTOCOL_HEADER: &str = "quod-protocol";

/// A step as the trust layer sees it, owned (`quod_trust::Step`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireStep {
    pub key: Hash,
    pub cmd: String,
    /// Output prefixes left out of comparisons (the step's scratch dirs).
    #[serde(default)]
    pub ignore: Vec<String>,
}

impl WireStep {
    pub fn from_step(s: &Step) -> WireStep {
        WireStep {
            key: s.key,
            cmd: s.cmd.to_string(),
            ignore: s.ignore.to_vec(),
        }
    }

    pub fn step(&self) -> Step<'_> {
        Step {
            key: self.key,
            cmd: &self.cmd,
            ignore: &self.ignore,
        }
    }
}

/// Where a self-hosted runner token says a job belongs: the scope its
/// own infrastructure decided (the runner hooks decide it from the event
/// payload, as they do locally). Honored only for a token the operator
/// marked as a runner token; ignored for OIDC callers, whose scope comes
/// from GitHub's signed claims.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeHint {
    /// `owner/name`.
    pub repository: String,
    /// `trusted`, `fork-<owner_repo>`, `unverified`, ...
    pub trust: String,
    /// Whether the runner judged the job a writer (a default-branch push or
    /// dispatch, or a schedule). Informational: as with a local ledger, a
    /// delegated runner's jobs write the scope it names (`trust`); the
    /// writer flag governs the runner's own warm caches, not the ledger.
    #[serde(default)]
    pub writer: bool,
}

/// What a runner knows about the CI job a step belongs to, for the
/// dashboard. For an OIDC caller the server takes repository, run, ref,
/// sha, actor, event and workflow from the token's claims and only the job
/// and step names from here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiContext {
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub run_number: Option<u64>,
    #[serde(default)]
    pub run_attempt: Option<u64>,
    /// The workflow's job id (`GITHUB_JOB`, e.g. `test`).
    #[serde(default)]
    pub job: Option<String>,
    /// GitHub's id of this job run (`${{ job.check_run_id }}`, passed as
    /// `QUOD_JOB_ID`): matrix legs of one workflow job differ.
    #[serde(default)]
    pub job_id: Option<String>,
    /// GitHub's display name of the job (`QUOD_JOB_NAME`, e.g.
    /// `check / ubuntu-24.04`).
    #[serde(default)]
    pub job_name: Option<String>,
    /// GitHub's 1-based step number within the job (`QUOD_STEP_INDEX`).
    #[serde(default)]
    pub step_index: Option<u32>,
    #[serde(default)]
    pub step_name: Option<String>,
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub sha: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub workflow: Option<String>,
    #[serde(default)]
    pub runner: Option<String>,
}

impl CiContext {
    /// From the variables GitHub Actions sets in every step.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> CiContext {
        CiContext {
            repository: get("GITHUB_REPOSITORY"),
            run_id: get("GITHUB_RUN_ID"),
            run_number: get("GITHUB_RUN_NUMBER").and_then(|v| v.parse().ok()),
            run_attempt: get("GITHUB_RUN_ATTEMPT").and_then(|v| v.parse().ok()),
            job: get("GITHUB_JOB"),
            job_id: get("QUOD_JOB_ID").filter(|v| !v.is_empty()),
            job_name: get("QUOD_JOB_NAME").filter(|v| !v.is_empty()),
            step_index: get("QUOD_STEP_INDEX").and_then(|v| v.parse().ok()),
            step_name: get("QUOD_STEP_NAME"),
            git_ref: get("GITHUB_REF"),
            sha: get("GITHUB_SHA"),
            actor: get("GITHUB_ACTOR"),
            event: get("GITHUB_EVENT_NAME"),
            workflow: get("GITHUB_WORKFLOW"),
            runner: get("RUNNER_NAME"),
        }
    }
}

/// What a replay or an execution is about, sent with every call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Context {
    pub step: WireStep,
    /// The client's policy. The server applies the stricter of it and the
    /// team's (`effective_policy`); the mode is the client's.
    pub policy: Policy,
    #[serde(default)]
    pub ci: Option<CiContext>,
    #[serde(default)]
    pub scope: Option<ScopeHint>,
}

/// POST /v1/claim: look up, decide and (on a replay) record, atomically on
/// the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimRequest {
    #[serde(flatten)]
    pub ctx: Context,
    /// The workspace tree the step would run on, every entry (directory
    /// listings included). The server matches candidates against it with
    /// the same `quod_manifest::matches` the local ledger uses.
    pub tree: Vec<Entry>,
    /// The run row to record if this replays (outcome `replayed`).
    pub run: RunRecord,
    /// A value unique to this claim, which a replay's receipt must sign:
    /// a recorded answer to an earlier claim (from a proxy, a cache, an
    /// attacker on the path) cannot be replayed into this one.
    #[serde(default)]
    pub nonce: Option<String>,
}

/// The matched manifest and the result a replay would serve, and the scope
/// it came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireHit {
    pub scope: String,
    pub manifest: Manifest,
    pub result: StepResult,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimResponse {
    /// How many manifests the key had in the scope that answered.
    pub candidates: usize,
    /// On a miss with candidates: the newest candidate's id and first
    /// mismatch.
    #[serde(default)]
    pub closest: Option<(Hash, Mismatch)>,
    #[serde(default)]
    pub hit: Option<WireHit>,
    pub claimed: Claimed,
    /// On a replay: the server's signature over what it served.
    #[serde(default)]
    pub receipt: Option<SignedReceipt>,
}

/// Names a hit a claim returned, so a later commit is compared with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HitRef {
    pub scope: String,
    pub manifest_id: Hash,
    pub run_id: String,
}

impl HitRef {
    pub fn of(h: &WireHit) -> HitRef {
        HitRef {
            scope: h.scope.clone(),
            manifest_id: h.result.manifest_id,
            run_id: h.result.run_id.clone(),
        }
    }
}

/// POST /v1/commit: the trust verdict, run row, manifest and result in one
/// transaction. The result's blobs must be on the server first (409 names
/// the missing ones).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitRequest {
    #[serde(flatten)]
    pub ctx: Context,
    pub run_id: String,
    pub manifest: Manifest,
    pub result: StepResult,
    #[serde(default)]
    pub hit: Option<HitRef>,
    pub reason: Reason,
    pub run: RunRecord,
}

/// POST /v1/divergence: a hit's execution behaved otherwise, with no result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DivergenceRequest {
    #[serde(flatten)]
    pub ctx: Context,
    pub run_id: String,
    pub hit: HitRef,
    pub reason: Reason,
    pub why: String,
    pub run: RunRecord,
}

/// POST /v1/incomplete: the step did not finish.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IncompleteRequest {
    #[serde(flatten)]
    pub ctx: Context,
    #[serde(default)]
    pub hit: Option<HitRef>,
    pub what: String,
    #[serde(default)]
    pub run: Option<RunRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncompleteResponse {
    #[serde(default)]
    pub line: Option<String>,
}

/// POST /v1/runs: a run row on its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunRequest {
    #[serde(flatten)]
    pub ctx: Context,
    pub run: RunRecord,
}

/// POST /v1/step-keys: the canonical JSON behind a key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepKeyRequest {
    #[serde(flatten)]
    pub ctx: Context,
    pub json: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitResponse {
    pub verdict: Verdict,
    /// A line for the job log, e.g. that trusted results are being
    /// written from an unprotected branch (DECISIONS 2026-11-02).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

/// POST /v1/blobs/missing: which of these the server does not hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobsRequest {
    pub hashes: Vec<Hash>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobsResponse {
    pub missing: Vec<Hash>,
}

/// GET /v1/info.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Info {
    pub protocol: String,
    pub version: String,
    /// The Ed25519 key receipts are signed with, hex.
    pub signing_key: String,
}

/// GET /v1/keys: the public keys receipts are signed with, for clients
/// to pin (`QUOD_SERVER_KEY`). Never a private key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keys {
    pub protocol: String,
    pub keys: Vec<PublicKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicKey {
    /// `ed25519`.
    pub alg: String,
    /// `receipts`: what the key signs.
    #[serde(rename = "use")]
    pub key_use: String,
    /// The 32-byte public key, hex: the value of `QUOD_SERVER_KEY`.
    pub public_key: String,
    /// `key_fingerprint`, as the server's log shows it.
    pub fingerprint: String,
}

impl PublicKey {
    pub fn of(k: &VerifyingKey) -> PublicKey {
        PublicKey {
            alg: "ed25519".into(),
            key_use: "receipts".into(),
            public_key: hex::encode(k.to_bytes()),
            fingerprint: key_fingerprint(k),
        }
    }
}

/// A public key's fingerprint: `ed25519:` and the first 16 hex digits of
/// BLAKE3 over its 32 bytes. Short enough to compare by eye between the
/// server's boot log, `GET /v1/keys` and a client's notice.
pub fn key_fingerprint(k: &VerifyingKey) -> String {
    format!("ed25519:{}", &Hash::of(k.as_bytes()).hex()[..16])
}

/// Every error body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// A stable code: `protocol`, `unauthorized`, `forbidden`, `not_found`,
    /// `bad_request`, `missing_blobs`, `conflict`, `not_implemented`,
    /// `internal`.
    pub error: String,
    pub message: String,
    /// For `missing_blobs`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<Hash>,
}

/// The blobs a result needs on the server before it may be committed, and
/// on a runner before it may be replayed: stdout, stderr, what it appended
/// to command files, and every file among the outputs a replay serves
/// (`comparable_outputs`) and every file it wrote outside the workspace.
pub fn result_blobs(r: &StepResult, ignore: &[String]) -> Vec<Hash> {
    let mut v = vec![r.stdout, r.stderr];
    v.extend(r.commands.iter().map(|c| c.h));
    v.extend(
        quod_trust::comparable_outputs(&r.outputs, ignore)
            .into_iter()
            .chain(r.root_outputs.iter())
            .filter(|e| e.k == quod_cas::Kind::File)
            .filter_map(|e| e.h),
    );
    v.sort();
    v.dedup();
    v
}

/// The policy a server applies: the client's mode, and for everything else
/// whichever of the two is stricter (more proofs, more audits, failures
/// replay only if both allow it). Audits are never deferred on a server,
/// whatever either says: every audit a remote runner is due runs inline in
/// that runner's job (docs/cloud-ledger.md, "Deferred audits and remote
/// runners"), so the server never holds a pending audit that only a runner
/// which may already be gone could settle.
pub fn effective_policy(client: &Policy, team: &Policy) -> Policy {
    Policy {
        mode: client.mode,
        proofs: client.proofs.max(team.proofs),
        shadow: client.shadow.max(team.shadow),
        audit: client.audit.max(team.audit),
        replay_failures: client.replay_failures && team.replay_failures,
        defer: false,
    }
}

// ----- signed results -----

/// What the server signs when it serves a replay: which team and scope,
/// which key and manifest, and exactly which result (the hash of its
/// canonical JSON). A client replays only a result whose statement verifies
/// under the server key it trusts and names what it asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayStatement {
    pub protocol: String,
    pub team: String,
    pub scope: String,
    pub step_key: Hash,
    pub manifest_id: Hash,
    pub run_id: String,
    pub result: Hash,
    pub issued_at: String,
    /// The claim's nonce: this statement answers that claim and no other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedReceipt {
    pub statement: ReplayStatement,
    /// Ed25519 over the statement's canonical JSON, hex.
    pub signature: String,
    /// The signing key, hex (which key the server used; the client checks
    /// it against the one it trusts).
    pub key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("bad key: {0}")]
    BadKey(String),
    #[error("receipt: {0}")]
    Receipt(String),
    #[error(transparent)]
    Canon(#[from] quod_cas::CanonError),
}

/// The hash a statement names a result by.
pub fn result_hash(r: &StepResult) -> Result<Hash, WireError> {
    Ok(Hash::of(quod_cas::canonical_json(r)?.as_bytes()))
}

/// A signing key from its 32-byte seed, hex.
pub fn signing_key_from_hex(s: &str) -> Result<SigningKey, WireError> {
    let b = hex::decode(s.trim()).map_err(|e| WireError::BadKey(e.to_string()))?;
    let seed: [u8; 32] = b
        .try_into()
        .map_err(|_| WireError::BadKey("an Ed25519 seed is 32 bytes (64 hex digits)".into()))?;
    Ok(SigningKey::from_bytes(&seed))
}

/// A verifying (public) key, hex.
pub fn verifying_key_from_hex(s: &str) -> Result<VerifyingKey, WireError> {
    let b = hex::decode(s.trim()).map_err(|e| WireError::BadKey(e.to_string()))?;
    let b: [u8; 32] = b
        .try_into()
        .map_err(|_| WireError::BadKey("an Ed25519 public key is 32 bytes".into()))?;
    VerifyingKey::from_bytes(&b).map_err(|e| WireError::BadKey(e.to_string()))
}

pub fn sign(key: &SigningKey, statement: ReplayStatement) -> Result<SignedReceipt, WireError> {
    let msg = quod_cas::canonical_json(&statement)?;
    Ok(SignedReceipt {
        signature: hex::encode(key.sign(msg.as_bytes()).to_bytes()),
        key: hex::encode(key.verifying_key().to_bytes()),
        statement,
    })
}

/// Checks a receipt: signed by `trusted`, over a statement for this
/// protocol, this key, this manifest, exactly this result, and the claim
/// that carried `nonce` (an earlier answer, recorded and played back, names
/// another nonce).
pub fn verify(
    trusted: &VerifyingKey,
    receipt: &SignedReceipt,
    key: &Hash,
    result: &StepResult,
    nonce: &str,
) -> Result<(), WireError> {
    let bad = |m: &str| Err(WireError::Receipt(m.to_string()));
    if receipt.key != hex::encode(trusted.to_bytes()) {
        return bad("signed by another key than the server key this runner trusts");
    }
    let sig = hex::decode(&receipt.signature)
        .ok()
        .and_then(|b| <[u8; 64]>::try_from(b).ok())
        .map(|b| Signature::from_bytes(&b));
    let Some(sig) = sig else {
        return bad("the signature is not 64 bytes of hex");
    };
    let msg = quod_cas::canonical_json(&receipt.statement)?;
    if trusted.verify(msg.as_bytes(), &sig).is_err() {
        return bad("the signature does not verify");
    }
    let s = &receipt.statement;
    if s.protocol != PROTOCOL {
        return bad("another protocol");
    }
    if s.step_key != *key || s.step_key != result.step_key {
        return bad("signed for another step key");
    }
    if s.manifest_id != result.manifest_id || s.run_id != result.run_id {
        return bad("signed for another manifest or run");
    }
    if s.result != result_hash(result)? {
        return bad("signed for other result bytes");
    }
    if s.nonce.as_deref() != Some(nonce) {
        return bad("signed for another claim (a stale or replayed answer)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> StepResult {
        StepResult {
            v: quod_manifest::VERSION,
            step_key: Hash::of(b"k"),
            manifest_id: Hash::of(b"m"),
            run_id: "01RUN".into(),
            recorded_at: "2026-11-01T00:00:00.000Z".into(),
            exit_code: 0,
            stdout: Hash::of(b"out"),
            stderr: Hash::of(b""),
            outputs: vec![],
            root_outputs: vec![],
            duration_ms: 1000,
            commands: vec![],
        }
    }

    fn statement(r: &StepResult) -> ReplayStatement {
        ReplayStatement {
            protocol: PROTOCOL.into(),
            team: "acme".into(),
            scope: "trusted".into(),
            step_key: r.step_key,
            manifest_id: r.manifest_id,
            run_id: r.run_id.clone(),
            result: result_hash(r).unwrap(),
            issued_at: "2026-11-01T00:00:00.000Z".into(),
            nonce: Some("n1".into()),
        }
    }

    #[test]
    fn a_receipt_verifies_only_for_what_it_signed() {
        let k = signing_key_from_hex(&"07".repeat(32)).unwrap();
        let r = result();
        let rec = sign(&k, statement(&r)).unwrap();
        let pk = k.verifying_key();
        verify(&pk, &rec, &r.step_key, &r, "n1").unwrap();
        // A genuine receipt for an earlier claim, played back into this one.
        assert!(verify(&pk, &rec, &r.step_key, &r, "n2").is_err());

        // Another result's bytes (one changed exit code).
        let mut other = r.clone();
        other.exit_code = 1;
        assert!(verify(&pk, &rec, &r.step_key, &other, "n1").is_err());
        // Another key asked for.
        assert!(verify(&pk, &rec, &Hash::of(b"other"), &r, "n1").is_err());
        // A tampered statement.
        let mut t = rec.clone();
        t.statement.scope = "fork-x".into();
        assert!(verify(&pk, &t, &r.step_key, &r, "n1").is_err());
        // Signed by a key this runner does not trust.
        let k2 = signing_key_from_hex(&"08".repeat(32)).unwrap();
        let rec2 = sign(&k2, statement(&r)).unwrap();
        assert!(verify(&pk, &rec2, &r.step_key, &r, "n1").is_err());
        // Claiming the trusted key but signed by another.
        let mut forged = rec2.clone();
        forged.key = rec.key.clone();
        assert!(verify(&pk, &forged, &r.step_key, &r, "n1").is_err());
    }

    #[test]
    fn the_stricter_policy_wins_but_the_mode_is_the_clients() {
        let client = Policy {
            mode: quod_trust::Mode::Replay,
            proofs: 1,
            shadow: 0,
            audit: 0.0,
            replay_failures: true,
            defer: true,
        };
        let team = Policy {
            mode: quod_trust::Mode::Observe,
            proofs: 3,
            shadow: 2,
            audit: 0.05,
            replay_failures: false,
            defer: true,
        };
        let p = effective_policy(&client, &team);
        assert_eq!(p.mode, quod_trust::Mode::Replay);
        assert_eq!(
            (p.proofs, p.shadow, p.audit, p.replay_failures),
            (3, 2, 0.05, false)
        );
        // Never deferred on a server, whatever both ask.
        assert!(!p.defer);
    }

    #[test]
    fn requests_round_trip_and_ignore_unknown_fields() {
        let r = result();
        let req = RunRequest {
            ctx: Context {
                step: WireStep {
                    key: r.step_key,
                    cmd: "go test ./...".into(),
                    ignore: vec![".quod/gocache".into()],
                },
                policy: Policy::default(),
                ci: None,
                scope: None,
            },
            run: RunRecord {
                id: "01RUN".into(),
                ..RunRecord::default()
            },
        };
        let mut v = serde_json::to_value(&req).unwrap();
        v["a_field_from_a_newer_client"] = serde_json::json!(1);
        let back: RunRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back, req);
    }
}
