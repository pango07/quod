//! Test-level replay over quod.v1: the `quod vitest` ledger directory stored
//! in Quod Cloud instead of actions/cache, and the per-run reports the
//! dashboard shows. docs/test-replay-cloud.md is the prose version.
//!
//! A ledger is a set of files (`ledger.json`, `results/...`, `blobs/...`,
//! `tests/...`, `render/...`), each stored in the team's content-addressed
//! blob store under its BLAKE3, so a save uploads only what the server
//! lacks. A save is one row naming every file; it exists only once every
//! blob is on the server, so a reader never sees half a ledger.
//!
//! A lookup answer is signed like a replay receipt: the statement names the
//! ledger, the hash of its file list and the lookup's nonce, and the client
//! checks it under the server key it trusts before it uses anything, then
//! checks every file against its hash. A ledger decides which test results
//! are reused, so a compromised endpoint must not be able to hand one over.
//! The signed message is prefixed (`LEDGER_DOMAIN`), so it can never be
//! mistaken for a step replay receipt or the other way round.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use quod_cas::Hash;
use serde::{Deserialize, Serialize};

use crate::{CiContext, PROTOCOL, ScopeHint, WireError};

/// Prefixed to a ledger statement's canonical JSON before signing.
pub const LEDGER_DOMAIN: &str = "quod.v1 test-ledger\n";
/// The most files one ledger may have.
pub const MAX_FILES: usize = 100_000;
/// The most bytes one ledger may have (all files).
pub const MAX_LEDGER_BYTES: u64 = 4 << 30;
/// The most ancestor commits a lookup may name.
pub const MAX_ANCESTORS: usize = 500;
/// The most hashes one pack request may name.
pub const MAX_PACK: usize = MAX_FILES;
/// The most files a report may list (ran, audited).
pub const MAX_REPORT_FILES: usize = 5_000;

/// The ledger directory's entries that travel: everything else in it
/// (`code/`, `dump/`, `plan.json`, `out.json`, temporary files, the opt-in
/// compile cache) is per-run.
pub const TOP_FILES: &[&str] = &["ledger.json"];
pub const TOP_DIRS: &[&str] = &["results", "blobs", "tests", "render"];

/// One file of a ledger: its path under the ledger directory, its BLAKE3
/// and its size.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LedgerFile {
    pub path: String,
    pub hash: Hash,
    pub size: u64,
}

/// A path a ledger may hold: relative, `/`-separated, no empty, `.` or `..`
/// component, no backslash, NUL or control character, under one of the
/// directories that travel (or `ledger.json`). Both sides check it: the
/// server before it stores a ledger, the client before it writes a file.
pub fn valid_path(p: &str) -> bool {
    if p.is_empty() || p.len() > 1024 || p.starts_with('/') {
        return false;
    }
    // No control characters, backslashes or colons (Windows separators
    // and alternate data streams).
    if p.chars().any(|c| c.is_control() || c == '\\' || c == ':') {
        return false;
    }
    let parts: Vec<&str> = p.split('/').collect();
    // No empty, `.` or `..` component, and none ending in a dot or a space
    // (which Windows drops, making two names one).
    if parts
        .iter()
        .any(|c| c.is_empty() || *c == "." || *c == ".." || c.ends_with('.') || c.ends_with(' '))
    {
        return false;
    }
    match parts.as_slice() {
        [f] => TOP_FILES.contains(f),
        [d, ..] => TOP_DIRS.contains(d),
        [] => false,
    }
}

/// A suite name: which vitest project of the repository (its directory
/// relative to the repository root, `.` for the root). 1-200 printable
/// characters, no `..` component.
pub fn valid_suite(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && !s.chars().any(|c| c.is_control() || c == '\\')
        && !s.split('/').any(|c| c == "..")
}

/// A commit id: 40 (SHA-1) or 64 (SHA-256) lowercase hex digits.
pub fn valid_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64)
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The hash a statement names a file list by: the canonical JSON of the
/// list sorted by path.
pub fn files_hash(files: &[LedgerFile]) -> Result<Hash, WireError> {
    let mut v = files.to_vec();
    v.sort();
    Ok(Hash::of(quod_cas::canonical_json(&v)?.as_bytes()))
}

/// Checks a file list: valid, unique paths, `ledger.json` present, within
/// the limits.
pub fn check_files(files: &[LedgerFile]) -> Result<(), String> {
    if files.len() > MAX_FILES {
        return Err(format!("more than {MAX_FILES} files"));
    }
    let mut seen = std::collections::BTreeSet::new();
    // Paths that differ only in case land on one file on a case-folding
    // file system (macOS, Windows): refused too.
    let mut folded = std::collections::BTreeSet::new();
    let mut total: u64 = 0;
    for f in files {
        if !valid_path(&f.path) {
            return Err(format!("invalid path {:?}", f.path));
        }
        if !seen.insert(f.path.as_str()) || !folded.insert(f.path.to_lowercase()) {
            return Err(format!("duplicate path {:?}", f.path));
        }
        total = total.saturating_add(f.size);
    }
    if total > MAX_LEDGER_BYTES {
        return Err("the ledger is too large".into());
    }
    if !seen.contains("ledger.json") {
        return Err("no ledger.json".into());
    }
    Ok(())
}

// ----- lookup -----

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookupRequest {
    pub suite: String,
    /// Commits to prefer, nearest first: the job's commit and the
    /// ancestors its checkout has. The server returns the ledger of the
    /// first one it has, else the newest of the branch, else the default
    /// branch's newest.
    #[serde(default)]
    pub ancestors: Vec<String>,
    /// This lookup's nonce: the answer's statement must name it.
    pub nonce: String,
    #[serde(default)]
    pub ci: Option<CiContext>,
    #[serde(default)]
    pub scope: Option<ScopeHint>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FoundLedger {
    pub id: i64,
    pub scope: String,
    #[serde(default)]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub sha: Option<String>,
    pub at: String,
    /// How it was chosen: `ancestor` (a commit the lookup named),
    /// `branch` (the newest of the job's own branch) or `default` (the
    /// newest the default branch's writers saved).
    pub matched: String,
    pub files: Vec<LedgerFile>,
    pub receipt: SignedLedger,
    /// The whole ledger as one object (pack format, any order), when the
    /// server made one: one download instead of one read per file. Not
    /// signed: every file in it is checked against the signed list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<BundleRef>,
}

/// A blob by hash and size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleRef {
    pub hash: Hash,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookupResponse {
    #[serde(default)]
    pub ledger: Option<FoundLedger>,
    /// The scope a save from this job would write (none: the job may not
    /// save, e.g. events on someone else's input).
    #[serde(default)]
    pub write_scope: Option<String>,
}

/// What the server signs when it hands out a ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerStatement {
    pub protocol: String,
    /// Always `test-ledger`.
    pub kind: String,
    pub team: String,
    pub repository: String,
    pub scope: String,
    pub suite: String,
    /// GitHub's id of the repository the lookup was for (OIDC callers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<u64>,
    /// The scope the job that asked writes (`trusted` for a writer, its
    /// ref's scope otherwise): who the answer is for.
    pub requester_scope: String,
    pub ledger_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    /// `files_hash` of the file list served.
    pub files: Hash,
    pub issued_at: String,
    pub nonce: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedLedger {
    pub statement: LedgerStatement,
    /// Ed25519 over `LEDGER_DOMAIN` + the statement's canonical JSON, hex.
    pub signature: String,
    /// The signing key, hex.
    pub key: String,
}

fn ledger_message(s: &LedgerStatement) -> Result<String, WireError> {
    Ok(format!("{LEDGER_DOMAIN}{}", quod_cas::canonical_json(s)?))
}

pub fn sign_ledger(
    key: &SigningKey,
    statement: LedgerStatement,
) -> Result<SignedLedger, WireError> {
    let msg = ledger_message(&statement)?;
    Ok(SignedLedger {
        signature: hex::encode(key.sign(msg.as_bytes()).to_bytes()),
        key: hex::encode(key.verifying_key().to_bytes()),
        statement,
    })
}

/// Who a lookup is for, as the job itself knows it (GitHub's variables):
/// a signed answer must be for this repository and for a scope this job
/// may read.
#[derive(Clone, Debug, Default)]
pub struct Requester {
    /// `owner/name` (`GITHUB_REPOSITORY`).
    pub repository: Option<String>,
    /// `GITHUB_REPOSITORY_ID`.
    pub repository_id: Option<u64>,
    /// `GITHUB_REF`.
    pub git_ref: Option<String>,
}

/// The scope of a ref: `ref-<readable part>-<hash>`, a valid id that two
/// refs never share (the server's `auth::ref_scope`).
pub fn ref_scope(r: &str) -> String {
    let readable: String = r
        .trim_start_matches("refs/")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let h = Hash::of(r.as_bytes()).hex();
    format!("ref-{readable}-{}", &h[..12])
}

/// Checks a lookup answer: signed by `trusted`, for this protocol, this
/// suite, exactly this ledger id and file list, the lookup that carried
/// `nonce`, and this job: its repository, and a ledger of the default
/// branch (`trusted`) or of this job's own ref. A proxy that relays the
/// nonce to the server under another job's token (another repository, or a
/// branch with a ledger of its own) gets an answer this check refuses.
pub fn verify_ledger(
    trusted: &VerifyingKey,
    found: &FoundLedger,
    suite: &str,
    nonce: &str,
    me: &Requester,
) -> Result<(), WireError> {
    let bad = |m: &str| Err(WireError::Receipt(m.to_string()));
    let r = &found.receipt;
    if r.key != hex::encode(trusted.to_bytes()) {
        return bad("signed by another key than the server key this runner trusts");
    }
    let sig = hex::decode(&r.signature)
        .ok()
        .and_then(|b| <[u8; 64]>::try_from(b).ok())
        .map(|b| Signature::from_bytes(&b));
    let Some(sig) = sig else {
        return bad("the signature is not 64 bytes of hex");
    };
    if trusted
        .verify(ledger_message(&r.statement)?.as_bytes(), &sig)
        .is_err()
    {
        return bad("the signature does not verify");
    }
    let s = &r.statement;
    if s.protocol != PROTOCOL || s.kind != "test-ledger" {
        return bad("another protocol or kind");
    }
    if s.suite != suite {
        return bad("signed for another suite");
    }
    if s.ledger_id != found.id || s.scope != found.scope || s.sha != found.sha {
        return bad("signed for another ledger");
    }
    if s.files != files_hash(&found.files)? {
        return bad("signed for another file list");
    }
    if s.nonce != nonce {
        return bad("signed for another lookup (a stale or replayed answer)");
    }
    if let Some(r) = &me.repository
        && s.repository != *r
    {
        return bad("signed for another repository");
    }
    if let (Some(a), Some(b)) = (me.repository_id, s.repository_id)
        && a != b
    {
        return bad("signed for another repository id");
    }
    // The ledger's scope: the default branch's, or this job's own ref's.
    let own = me.git_ref.as_deref().map(ref_scope);
    if s.scope != "trusted" && Some(&s.scope) != own.as_ref() {
        return bad("a ledger of another branch");
    }
    // Who it was signed for: this job (a writer's `trusted`, else its
    // ref's scope; jobs on someone else's input read as `unverified`).
    if s.requester_scope != "trusted"
        && s.requester_scope != "unverified"
        && Some(&s.requester_scope) != own.as_ref()
    {
        return bad("signed for another job's lookup");
    }
    Ok(())
}

// ----- save -----

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveRequest {
    pub suite: String,
    pub files: Vec<LedgerFile>,
    #[serde(default)]
    pub ci: Option<CiContext>,
    #[serde(default)]
    pub scope: Option<ScopeHint>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveResponse {
    pub id: i64,
    pub scope: String,
    /// A line for the job log (the writer-trust warning).
    #[serde(default)]
    pub notice: Option<String>,
}

/// Blobs to send back in one answer: each as `<64 hex> <size>\n` and the
/// bytes, or `<64 hex> -\n` when the store lacks it, in the order asked.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackRequest {
    pub hashes: Vec<Hash>,
}

// ----- run reports -----

/// A test file that ran, and why.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RanFile {
    pub file: String,
    #[serde(default)]
    pub project: String,
    /// Plain text: `first run`, `changed: src/a.ts`, `new test file`, ...
    pub why: String,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    /// vitest's state: pass, fail, skip, ...
    #[serde(default)]
    pub state: String,
    /// A record run: why this file could not be recorded (it reads a
    /// per-run variable, uses the network, a child process ...); it then
    /// always runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unrecordable: Option<String>,
}

/// A file a run reused instead of running, with its recorded time.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplayedFile {
    pub file: String,
    #[serde(default)]
    pub duration_ms: Option<f64>,
}

/// A replayable file the shadow ran to check it, and whether its fresh
/// result was the recorded one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFile {
    pub file: String,
    pub ok: bool,
}

/// Which ledger a run started from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerUsed {
    pub found: bool,
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub sha: Option<String>,
    #[serde(default)]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub matched: Option<String>,
}

/// One `quod vitest` run, as the critical path or the shadow reports it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TestReport {
    /// `vitest`.
    pub tool: String,
    /// `critical` (the developer's check) or `shadow` (the recording job).
    pub kind: String,
    /// Test files in the run (ran + replayed).
    pub total: u64,
    pub ran: u64,
    pub replayed: u64,
    /// Of the replayed, those whose source changed but not the functions
    /// they execute.
    #[serde(default)]
    pub narrowed: u64,
    #[serde(default)]
    pub wall_ms: Option<f64>,
    #[serde(default)]
    pub select_ms: Option<f64>,
    /// The recorded durations of every file, summed: test-file time, not
    /// wall time.
    #[serde(default)]
    pub est_plain_ms: Option<f64>,
    /// The recorded durations of the replayed files, summed.
    #[serde(default)]
    pub saved_ms: Option<f64>,
    #[serde(default)]
    pub exit: Option<i32>,
    #[serde(default)]
    pub audited: u64,
    #[serde(default)]
    pub audit_files: Vec<AuditFile>,
    #[serde(default)]
    pub ran_files: Vec<RanFile>,
    #[serde(default)]
    pub ledger: Option<LedgerUsed>,
    /// The files this run reused (the check's skips, counted per file).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replayed_files: Vec<ReplayedFile>,
    /// A record run: files whose recorded run behaved differently from the
    /// check's (quarantined at record time, not recorded).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub record_differs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportRequest {
    pub suite: String,
    pub report: TestReport,
    #[serde(default)]
    pub ci: Option<CiContext>,
    #[serde(default)]
    pub scope: Option<ScopeHint>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportResponse {
    pub id: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(key: &SigningKey, nonce: &str) -> FoundLedger {
        let files = vec![
            LedgerFile {
                path: "ledger.json".into(),
                hash: Hash::of(b"l"),
                size: 1,
            },
            LedgerFile {
                path: "results/a.json".into(),
                hash: Hash::of(b"a"),
                size: 1,
            },
        ];
        let st = LedgerStatement {
            protocol: PROTOCOL.into(),
            kind: "test-ledger".into(),
            team: "t".into(),
            repository: "o/r".into(),
            scope: "trusted".into(),
            suite: ".".into(),
            repository_id: Some(5),
            requester_scope: "trusted".into(),
            ledger_id: 7,
            sha: Some("a".repeat(40)),
            files: files_hash(&files).unwrap(),
            issued_at: "now".into(),
            nonce: nonce.into(),
        };
        FoundLedger {
            id: 7,
            scope: "trusted".into(),
            git_ref: Some("refs/heads/main".into()),
            sha: Some("a".repeat(40)),
            at: "now".into(),
            matched: "ancestor".into(),
            files,
            receipt: sign_ledger(key, st).unwrap(),
            bundle: None,
        }
    }

    fn me() -> Requester {
        Requester {
            repository: Some("o/r".into()),
            repository_id: Some(5),
            git_ref: Some("refs/heads/main".into()),
        }
    }

    #[test]
    fn an_answer_for_another_job_is_refused() {
        let k = SigningKey::from_bytes(&[7; 32]);
        let f = found(&k, "n1");
        let vk = k.verifying_key();
        // Another repository, by name or id.
        let mut r = me();
        r.repository = Some("o/other".into());
        assert!(verify_ledger(&vk, &f, ".", "n1", &r).is_err());
        let mut r = me();
        r.repository_id = Some(6);
        assert!(verify_ledger(&vk, &f, ".", "n1", &r).is_err());
        // A ledger of another branch, signed for that branch's job.
        let mut st = f.receipt.statement.clone();
        st.scope = ref_scope("refs/heads/evil");
        st.requester_scope = ref_scope("refs/heads/evil");
        let mut g = f.clone();
        g.scope = st.scope.clone();
        g.receipt = sign_ledger(&k, st).unwrap();
        assert!(verify_ledger(&vk, &g, ".", "n1", &me()).is_err());
        // The default branch's ledger signed for another branch's job.
        let mut st = f.receipt.statement.clone();
        st.requester_scope = ref_scope("refs/heads/evil");
        let mut g = f.clone();
        g.receipt = sign_ledger(&k, st).unwrap();
        assert!(verify_ledger(&vk, &g, ".", "n1", &me()).is_err());
        // This job's own branch ledger is fine.
        let mut st = f.receipt.statement.clone();
        st.scope = ref_scope("refs/heads/feat");
        st.requester_scope = ref_scope("refs/heads/feat");
        let mut g = f.clone();
        g.scope = st.scope.clone();
        g.receipt = sign_ledger(&k, st).unwrap();
        let mut r = me();
        r.git_ref = Some("refs/heads/feat".into());
        verify_ledger(&vk, &g, ".", "n1", &r).unwrap();
    }

    #[test]
    fn a_ledger_answer_verifies_only_for_what_it_signed() {
        let k = SigningKey::from_bytes(&[7; 32]);
        let other = SigningKey::from_bytes(&[8; 32]);
        let f = found(&k, "n1");
        verify_ledger(&k.verifying_key(), &f, ".", "n1", &me()).unwrap();
        // Another key, nonce, suite.
        assert!(verify_ledger(&other.verifying_key(), &f, ".", "n1", &me()).is_err());
        assert!(verify_ledger(&k.verifying_key(), &f, ".", "n2", &me()).is_err());
        assert!(verify_ledger(&k.verifying_key(), &f, "pkg", "n1", &me()).is_err());
        // A file swapped, added or dropped.
        let mut g = f.clone();
        g.files[1].hash = Hash::of(b"evil");
        assert!(verify_ledger(&k.verifying_key(), &g, ".", "n1", &me()).is_err());
        let mut g = f.clone();
        g.files.push(LedgerFile {
            path: "results/b.json".into(),
            hash: Hash::of(b"b"),
            size: 1,
        });
        assert!(verify_ledger(&k.verifying_key(), &g, ".", "n1", &me()).is_err());
        let mut g = f.clone();
        g.files.pop();
        assert!(verify_ledger(&k.verifying_key(), &g, ".", "n1", &me()).is_err());
        // Another ledger id or scope.
        let mut g = f.clone();
        g.id = 8;
        assert!(verify_ledger(&k.verifying_key(), &g, ".", "n1", &me()).is_err());
        let mut g = f.clone();
        g.scope = "ref-x".into();
        assert!(verify_ledger(&k.verifying_key(), &g, ".", "n1", &me()).is_err());
        // The order of the list does not matter.
        let mut g = f.clone();
        g.files.reverse();
        verify_ledger(&k.verifying_key(), &g, ".", "n1", &me()).unwrap();
        // A forged key field with the real signature.
        let mut g = f.clone();
        g.receipt.key = hex::encode(other.verifying_key().to_bytes());
        assert!(verify_ledger(&other.verifying_key(), &g, ".", "n1", &me()).is_err());
    }

    #[test]
    fn a_ledger_signature_is_not_a_replay_receipt() {
        // The same key signs both; the domain prefix keeps them apart.
        let k = SigningKey::from_bytes(&[7; 32]);
        let f = found(&k, "n1");
        let msg = quod_cas::canonical_json(&f.receipt.statement).unwrap();
        let sig = hex::decode(&f.receipt.signature).unwrap();
        let sig = Signature::from_bytes(&<[u8; 64]>::try_from(sig).unwrap());
        assert!(k.verifying_key().verify(msg.as_bytes(), &sig).is_err());
    }

    #[test]
    fn paths_stay_inside_the_ledger() {
        for ok in [
            "ledger.json",
            "results/x.json",
            "blobs/abc",
            "tests/a b.json",
            "render/stdout",
            "results/-1234_x.json",
        ] {
            assert!(valid_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "results/../../x",
            "results/./x",
            "results//x",
            "code/driver.mjs",
            "plan.json",
            "results",
            "results\\..\\x",
            "results/a\nb",
            "ledger.json/x",
            "results/a:b",
            "results/a.",
            "results/a ",
        ] {
            assert!(!valid_path(bad), "{bad:?}");
        }
        assert!(valid_suite(".") && valid_suite("packages/backend"));
        assert!(!valid_suite("") && !valid_suite("../x") && !valid_suite("a/../b"));
        assert!(valid_sha(&"a".repeat(40)) && !valid_sha("HEAD") && !valid_sha(&"A".repeat(40)));
    }

    #[test]
    fn file_lists_are_checked() {
        let f = |p: &str| LedgerFile {
            path: p.into(),
            hash: Hash::of(p.as_bytes()),
            size: 1,
        };
        assert!(check_files(&[f("ledger.json"), f("results/a.json")]).is_ok());
        assert!(check_files(&[f("results/a.json")]).is_err());
        assert!(check_files(&[f("ledger.json"), f("ledger.json")]).is_err());
        assert!(check_files(&[f("ledger.json"), f("../x")]).is_err());
        assert!(check_files(&[f("ledger.json"), f("results/A"), f("results/a")]).is_err());
        let mut big = f("ledger.json");
        big.size = MAX_LEDGER_BYTES + 1;
        assert!(check_files(&[big]).is_err());
    }
}
