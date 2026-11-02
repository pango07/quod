//! The runner's side of quod.v1: `RemoteLedger`, a `StepLedger` whose
//! ledger and result store are a quod-server's.
//!
//! The rule is "never a wrong replay, and on any doubt, run". A replay is
//! taken only when every check passes on the runner: the server's receipt
//! verifies under the key the runner trusts and names exactly the result
//! served; that result is for this step key and the manifest returned; the
//! manifest matches this runner's own tree (`quod_manifest::matches`, the
//! same check the local ledger makes); and every blob the replay serves
//! was fetched and hashes to its name. Anything else, including the server
//! being unreachable, slow or answering an error, makes the step execute.
//! Once a call fails the ledger goes offline for the rest of the process:
//! the step runs and nothing is recorded anywhere (the step's own result
//! is what the user gets, as without Quod).

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::time::Duration;

use ed25519_dalek::VerifyingKey;
use quod_cas::{BlobStore, Cas, Hash, Tree};
use quod_ledger::{Lookup, RunRecord, TrustRow};
use quod_manifest::{Manifest, StepResult, manifest_id, matches};
use quod_trust::{Claimed, Policy, Reason, Step, StepLedger, TrustError, Verdict};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::*;

/// Blobs larger than this go up in resumable chunks of this size.
pub const CHUNK: u64 = 32 << 20;

/// How a runner authenticates.
#[derive(Clone, Debug)]
pub enum Auth {
    /// A static token (self-hosted deployments): `QUOD_SERVER_TOKEN`.
    Token(String),
    /// A GitHub Actions OIDC token for `audience`, requested from the job's
    /// token endpoint (the job needs `permissions: id-token: write`).
    GithubOidc {
        request_url: String,
        request_token: String,
        audience: String,
    },
}

/// Where the server is and how to talk to it.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The base URL, e.g. `https://api.quodci.com`.
    pub url: String,
    pub auth: Auth,
    /// The server's Ed25519 key. When unset the runner takes it from
    /// `GET /v1/info` over the same TLS connection that authenticates the
    /// server; pinning it (`QUOD_SERVER_KEY`) means a compromised endpoint
    /// or certificate still cannot make a runner replay.
    pub server_key: Option<VerifyingKey>,
    pub timeout: Duration,
    pub scope: Option<ScopeHint>,
    pub ci: Option<CiContext>,
}

impl ServerConfig {
    /// From the environment: `None` when `QUOD_SERVER` is unset (local
    /// ledger), else the config or why it is unusable. `QUOD_SERVER_TOKEN`,
    /// or GitHub's `ACTIONS_ID_TOKEN_REQUEST_URL`/`_TOKEN` with
    /// `QUOD_SERVER_AUDIENCE` (default `quodci`); `QUOD_SERVER_KEY`;
    /// `QUOD_SERVER_TIMEOUT` (seconds, default 30); `QUOD_SCOPE_REPOSITORY`,
    /// `QUOD_SCOPE_TRUST`, `QUOD_SCOPE_WRITER` (a runner token's hint).
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Option<Result<ServerConfig, String>> {
        let url = get("QUOD_SERVER").filter(|v| !v.is_empty())?;
        Some(Self::build(url, &get))
    }

    fn build(url: String, get: &impl Fn(&str) -> Option<String>) -> Result<ServerConfig, String> {
        let url = url.trim_end_matches('/').to_string();
        if !url.starts_with("https://") && !is_loopback_http(&url) {
            return Err(format!(
                "QUOD_SERVER {url:?} is not an https URL (plain http is accepted only for a server on this machine: 127.0.0.1, localhost, [::1])"
            ));
        }
        let auth = match (
            get("QUOD_SERVER_TOKEN").filter(|v| !v.is_empty()),
            get("ACTIONS_ID_TOKEN_REQUEST_URL"),
            get("ACTIONS_ID_TOKEN_REQUEST_TOKEN"),
        ) {
            (Some(t), ..) => Auth::Token(t),
            (None, Some(u), Some(t)) => Auth::GithubOidc {
                request_url: u,
                request_token: t,
                audience: get("QUOD_SERVER_AUDIENCE").unwrap_or_else(|| "quodci".into()),
            },
            _ => {
                return Err(
                    "QUOD_SERVER is set but there is no QUOD_SERVER_TOKEN, and no GitHub OIDC token endpoint (the job needs `permissions: id-token: write`)".into(),
                );
            }
        };
        let server_key = get("QUOD_SERVER_KEY")
            .map(|k| verifying_key_from_hex(&k).map_err(|e| format!("QUOD_SERVER_KEY: {e}")))
            .transpose()?;
        let timeout = Duration::from_secs(
            get("QUOD_SERVER_TIMEOUT")
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        );
        let scope = get("QUOD_SCOPE_TRUST").map(|trust| ScopeHint {
            repository: get("QUOD_SCOPE_REPOSITORY")
                .or_else(|| get("GITHUB_REPOSITORY"))
                .unwrap_or_default(),
            trust,
            writer: get("QUOD_SCOPE_WRITER").is_some_and(|v| v == "1" || v == "true"),
        });
        Ok(ServerConfig {
            url,
            auth,
            server_key,
            timeout,
            scope,
            ci: get("GITHUB_ACTIONS")
                .is_some()
                .then(|| CiContext::from_env(get)),
        })
    }
}

/// `http://` to this machine only: anything else would send the token and
/// receive results in the clear.
fn is_loopback_http(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// A value unique to one claim: the time, the process, a counter and the
/// key. Unpredictability is not needed, only that no earlier claim had it.
fn claim_nonce(key: &Hash) -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let host = std::fs::read_to_string("/etc/machine-id").unwrap_or_default();
    Hash::of(
        format!(
            "quod claim nonce\0{t}\0{}\0{seq}\0{key}\0{host}",
            std::process::id()
        )
        .as_bytes(),
    )
    .hex()
}

/// A `StepLedger` over quod-server. Blobs a replay serves are fetched into
/// `cas` (the runner's local store, which the replay reads); blobs a commit
/// needs are uploaded from it.
pub struct RemoteLedger<'a> {
    cfg: ServerConfig,
    agent: ureq::Agent,
    cas: &'a Cas,
    policy: Policy,
    bearer: RefCell<Option<String>>,
    server_key: RefCell<Option<VerifyingKey>>,
    /// Why the server is not used any more in this process, once it is not.
    offline: RefCell<Option<String>>,
    /// The scope each hit a claim returned came from, by (manifest, run).
    hits: RefCell<HashMap<(Hash, String), String>>,
}

#[derive(Debug)]
/// Why a call to the server failed.
pub struct CallError(pub String);

impl From<ureq::Error> for CallError {
    fn from(e: ureq::Error) -> CallError {
        match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                let msg = serde_json::from_str::<ErrorBody>(&body)
                    .map(|b| format!("{}: {}", b.error, b.message))
                    .unwrap_or(body);
                CallError(format!(
                    "HTTP {code}: {}",
                    msg.chars().take(300).collect::<String>()
                ))
            }
            ureq::Error::Transport(t) => CallError(format!("unreachable: {t}")),
        }
    }
}

impl From<std::io::Error> for CallError {
    fn from(e: std::io::Error) -> CallError {
        CallError(e.to_string())
    }
}

impl<'a> RemoteLedger<'a> {
    pub fn new(cfg: ServerConfig, cas: &'a Cas, policy: Policy) -> RemoteLedger<'a> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5).min(cfg.timeout))
            .timeout(cfg.timeout)
            .build();
        RemoteLedger {
            server_key: RefCell::new(cfg.server_key),
            cfg,
            agent,
            cas,
            policy,
            bearer: RefCell::new(None),
            offline: RefCell::new(None),
            hits: RefCell::new(HashMap::new()),
        }
    }

    /// A ledger that is offline from the start (a `QUOD_SERVER` config that
    /// cannot be used): every step runs, nothing is recorded.
    pub fn unusable(why: String, cas: &'a Cas, policy: Policy) -> RemoteLedger<'a> {
        let cfg = ServerConfig {
            url: String::new(),
            auth: Auth::Token(String::new()),
            server_key: None,
            timeout: Duration::from_secs(1),
            scope: None,
            ci: None,
        };
        let r = RemoteLedger::new(cfg, cas, policy);
        r.go_offline(why);
        r
    }

    /// Whether the server is still in use.
    pub fn is_online(&self) -> bool {
        self.offline.borrow().is_none()
    }

    fn go_offline(&self, why: String) {
        if self.offline.borrow().is_none() {
            eprintln!(
                "quod: the Quod server is not used ({why}); the step runs, and nothing is replayed or recorded"
            );
            tracing::warn!(target: "quod", why = %why, "quod server offline");
            *self.offline.borrow_mut() = Some(why);
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.cfg.url)
    }

    fn bearer(&self) -> Result<String, CallError> {
        if let Some(b) = self.bearer.borrow().as_ref() {
            return Ok(b.clone());
        }
        let b = match &self.cfg.auth {
            Auth::Token(t) => t.clone(),
            Auth::GithubOidc {
                request_url,
                request_token,
                audience,
            } => {
                let sep = if request_url.contains('?') { '&' } else { '?' };
                let v: serde_json::Value = self
                    .agent
                    .get(&format!("{request_url}{sep}audience={audience}"))
                    .set("authorization", &format!("bearer {request_token}"))
                    .call()?
                    .into_json()?;
                v["value"]
                    .as_str()
                    .ok_or_else(|| CallError("the OIDC token endpoint returned no value".into()))?
                    .to_string()
            }
        };
        *self.bearer.borrow_mut() = Some(b.clone());
        Ok(b)
    }

    fn request(&self, method: &str, path: &str) -> Result<ureq::Request, CallError> {
        Ok(self
            .agent
            .request(method, &self.url(path))
            .set(PROTOCOL_HEADER, PROTOCOL)
            .set("authorization", &format!("Bearer {}", self.bearer()?)))
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T, CallError> {
        let body = serde_json::to_vec(body).map_err(|e| CallError(e.to_string()))?;
        let resp = self
            .request("POST", path)?
            .set("content-type", "application/json")
            .send_bytes(&body)?;
        if resp.header(PROTOCOL_HEADER) != Some(PROTOCOL) {
            return Err(CallError("the answer is not quod.v1".into()));
        }
        resp.into_json()
            .map_err(|e| CallError(format!("bad answer: {e}")))
    }

    fn server_key(&self) -> Result<VerifyingKey, CallError> {
        if let Some(k) = *self.server_key.borrow() {
            return Ok(k);
        }
        let info: Info = self
            .agent
            .get(&self.url("/v1/info"))
            .set(PROTOCOL_HEADER, PROTOCOL)
            .call()?
            .into_json()?;
        let k = verifying_key_from_hex(&info.signing_key).map_err(|e| CallError(e.to_string()))?;
        *self.server_key.borrow_mut() = Some(k);
        Ok(k)
    }

    fn ctx(&self, step: &Step) -> Context {
        Context {
            step: WireStep::from_step(step),
            policy: self.policy,
            ci: self.cfg.ci.clone(),
            scope: self.cfg.scope.clone(),
        }
    }

    fn hit_ref(&self, hit: &(Manifest, StepResult)) -> Option<HitRef> {
        let r = &hit.1;
        self.hits
            .borrow()
            .get(&(r.manifest_id, r.run_id.clone()))
            .map(|scope| HitRef {
                scope: scope.clone(),
                manifest_id: r.manifest_id,
                run_id: r.run_id.clone(),
            })
    }

    /// Fetches `hashes` the local store lacks, each checked against its name.
    pub fn fetch_blobs(&self, hashes: &[Hash]) -> Result<(), CallError> {
        for h in hashes {
            if self.cas.has(h) {
                continue;
            }
            let resp = self
                .request("GET", &format!("/v1/blobs/{}", h.hex()))?
                .call()?;
            let tmp = tempfile::NamedTempFile::new_in(self.cas.dir().join("tmp"))?;
            let mut f = tmp.as_file();
            std::io::copy(&mut resp.into_reader(), &mut f)?;
            f.flush()?;
            let (got, _) = self
                .cas
                .put_blob_file(tmp.path())
                .map_err(|e| CallError(e.to_string()))?;
            if got != *h {
                return Err(CallError(format!(
                    "blob {h} came back as {got}; not replayed"
                )));
            }
        }
        Ok(())
    }

    /// Uploads whichever of `hashes` the server lacks, from the local store.
    pub fn upload_blobs(&self, hashes: &[Hash]) -> Result<(), CallError> {
        let missing: BlobsResponse = self.post(
            "/v1/blobs/missing",
            &BlobsRequest {
                hashes: hashes.to_vec(),
            },
        )?;
        for h in &missing.missing {
            let path = self.cas.blob_path(h);
            let size = std::fs::metadata(&path)?.len();
            let blob = format!("/v1/blobs/{}", h.hex());
            if size <= CHUNK {
                let f = std::fs::File::open(&path)?;
                self.request("PUT", &blob)?
                    .set("content-type", "application/octet-stream")
                    .set("content-length", &size.to_string())
                    .send(f)?;
                continue;
            }
            // Resumable: ask how much the server has, send the rest in
            // chunks, the last one marked complete.
            let head = self.request("HEAD", &blob)?.call();
            let mut offset = match head {
                Ok(r) if r.header("upload-offset").is_none() => continue, // present
                Ok(r) => r
                    .header("upload-offset")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                Err(ureq::Error::Status(404, r)) => r
                    .header("upload-offset")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                Err(e) => return Err(e.into()),
            };
            let mut f = std::fs::File::open(&path)?;
            let mut conflicts = 0;
            while offset < size {
                let n = CHUNK.min(size - offset);
                let mut buf = vec![0u8; n as usize];
                std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(offset))?;
                f.read_exact(&mut buf)?;
                let last = offset + n == size;
                let sent = self
                    .request("PATCH", &blob)?
                    .set("content-type", "application/octet-stream")
                    .set("upload-offset", &offset.to_string())
                    .set("upload-complete", if last { "true" } else { "false" })
                    .send_bytes(&buf);
                match sent {
                    Ok(r) if r.status() == 200 => break, // present meanwhile
                    Ok(_) => offset += n,
                    // Another runner is uploading the same blob, or the
                    // server has another offset: carry on from the server's.
                    Err(ureq::Error::Status(409, r)) => {
                        conflicts += 1;
                        if conflicts > 300 {
                            return Err(CallError(format!("upload of {h} kept conflicting")));
                        }
                        offset = r
                            .header("upload-offset")
                            .and_then(|v| v.parse().ok())
                            .ok_or_else(|| CallError("409 without upload-offset".into()))?;
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(())
    }

    /// A verdict for an execution nothing recorded: the step ran, its own
    /// result stands, and the banner says it was not proven anything.
    fn unrecorded(&self, step: &Step, reason: &Reason) -> Verdict {
        Verdict {
            reason: reason.clone(),
            compared_with: None,
            divergence: None,
            byte_identical: false,
            streak: 0,
            needed: self.policy.proofs,
            trust: TrustRow::new(step.key, Some(step.cmd)),
            would_have_saved: None,
            unprovable: None,
            suspects: 0,
            shadow_left: 0,
        }
    }

    /// The claim, checked: Ok((lookup, claimed)) or the reason not to trust
    /// what came back.
    fn try_claim(
        &self,
        step: &Step,
        tree: &Tree,
        run: &mut dyn FnMut(&StepResult) -> RunRecord,
    ) -> Result<(Lookup, Claimed), CallError> {
        // The run row a replay records: built here, as the local ledger
        // builds it in its transaction, from the result the server would
        // serve; the server fills in what it serves.
        let placeholder = run(&StepResult {
            v: quod_manifest::VERSION,
            step_key: step.key,
            manifest_id: Hash::of(b""),
            run_id: String::new(),
            recorded_at: String::new(),
            exit_code: 0,
            stdout: Hash::of(b""),
            stderr: Hash::of(b""),
            outputs: vec![],
            root_outputs: vec![],
            duration_ms: 0,
            commands: vec![],
        });
        let nonce = claim_nonce(&step.key);
        let mut resp: ClaimResponse = self.post(
            "/v1/claim",
            &ClaimRequest {
                ctx: self.ctx(step),
                tree: tree.iter().cloned().collect(),
                run: placeholder,
                nonce: Some(nonce.clone()),
            },
        )?;
        let mut lookup = Lookup {
            hit: None,
            candidates: resp.candidates,
            closest: resp.closest.clone(),
        };
        // A hit is kept (for the banner and for what an execution is
        // compared with) only if it is what it says, from a schema this
        // binary reads (as `Ledger::find_match` filters), and matches this
        // tree.
        let hit = match resp.hit.take() {
            Some(h) => {
                let ok = h.result.step_key == step.key
                    && h.manifest.step_key == step.key
                    && h.manifest.v <= quod_manifest::VERSION
                    && h.result.v <= quod_manifest::VERSION
                    && manifest_id(&h.manifest).ok() == Some(h.result.manifest_id)
                    && matches(&h.manifest, tree).is_ok();
                if !ok {
                    if matches!(resp.claimed, Claimed::Replay { .. }) {
                        return Err(CallError(
                            "the server's hit does not match this workspace".into(),
                        ));
                    }
                    // Without the hit the server's reason (a streak, an
                    // audit) names a manifest this runner refused: a miss.
                    resp.claimed = Claimed::Execute {
                        reason: Reason::Miss,
                    };
                    None
                } else {
                    Some(h)
                }
            }
            None => None,
        };
        if let Some(h) = &hit {
            self.hits.borrow_mut().insert(
                (h.result.manifest_id, h.result.run_id.clone()),
                h.scope.clone(),
            );
        }
        if let Claimed::Replay { .. } = &resp.claimed {
            let h = hit
                .as_ref()
                .ok_or_else(|| CallError("a replay without a hit".into()))?;
            let receipt = resp
                .receipt
                .as_ref()
                .ok_or_else(|| CallError("a replay without a receipt".into()))?;
            verify(&self.server_key()?, receipt, &step.key, &h.result, &nonce)
                .map_err(|e| CallError(e.to_string()))?;
            self.fetch_blobs(&result_blobs(&h.result, step.ignore))?;
        }
        lookup.hit = hit.map(|h| (h.manifest, h.result));
        Ok((lookup, resp.claimed))
    }
}

fn empty_lookup() -> Lookup {
    Lookup {
        hit: None,
        candidates: 0,
        closest: None,
    }
}

impl StepLedger for RemoteLedger<'_> {
    fn put_step_key(&self, key: &Hash, json: &str) -> Result<(), TrustError> {
        if !self.is_online() {
            return Ok(());
        }
        let step = Step {
            key: *key,
            cmd: "",
            ignore: &[],
        };
        if let Err(e) = self.post::<serde_json::Value>(
            "/v1/step-keys",
            &StepKeyRequest {
                ctx: self.ctx(&step),
                json: json.to_string(),
            },
        ) {
            self.go_offline(e.0);
        }
        Ok(())
    }

    fn claim(
        &self,
        step: &Step,
        tree: &Tree,
        run: &mut dyn FnMut(&StepResult) -> RunRecord,
    ) -> Result<(Lookup, Claimed), TrustError> {
        if !self.is_online() {
            return Ok((
                empty_lookup(),
                Claimed::Execute {
                    reason: Reason::Miss,
                },
            ));
        }
        match self.try_claim(step, tree, run) {
            Ok(v) => Ok(v),
            Err(e) => {
                self.go_offline(e.0);
                Ok((
                    empty_lookup(),
                    Claimed::Execute {
                        reason: Reason::Miss,
                    },
                ))
            }
        }
    }

    fn commit_execution(
        &self,
        step: &Step,
        run_id: &str,
        manifest: &Manifest,
        result: &StepResult,
        hit: Option<&(Manifest, StepResult)>,
        reason: &Reason,
        run: RunRecord,
    ) -> Result<Verdict, TrustError> {
        if !self.is_online() {
            return Ok(self.unrecorded(step, reason));
        }
        let go = || -> Result<Verdict, CallError> {
            self.upload_blobs(&result_blobs(result, step.ignore))?;
            let resp: CommitResponse = self.post(
                "/v1/commit",
                &CommitRequest {
                    ctx: self.ctx(step),
                    run_id: run_id.to_string(),
                    manifest: manifest.clone(),
                    result: result.clone(),
                    hit: hit.and_then(|h| self.hit_ref(h)),
                    reason: reason.clone(),
                    run,
                },
            )?;
            Ok(resp.verdict)
        };
        Ok(go().unwrap_or_else(|e| {
            self.go_offline(e.0);
            self.unrecorded(step, reason)
        }))
    }

    fn commit_divergence(
        &self,
        step: &Step,
        run_id: &str,
        hit: &(Manifest, StepResult),
        reason: &Reason,
        why: &str,
        run: RunRecord,
    ) -> Result<Verdict, TrustError> {
        let Some(hit_ref) = self.hit_ref(hit).filter(|_| self.is_online()) else {
            return Ok(self.unrecorded(step, reason));
        };
        let r: Result<CommitResponse, CallError> = self.post(
            "/v1/divergence",
            &DivergenceRequest {
                ctx: self.ctx(step),
                run_id: run_id.to_string(),
                hit: hit_ref,
                reason: reason.clone(),
                why: why.to_string(),
                run,
            },
        );
        Ok(match r {
            Ok(r) => r.verdict,
            Err(e) => {
                self.go_offline(e.0);
                self.unrecorded(step, reason)
            }
        })
    }

    fn commit_incomplete(
        &self,
        step: &Step,
        hit: Option<&(Manifest, StepResult)>,
        what: &str,
        run: Option<RunRecord>,
    ) -> Result<Option<String>, TrustError> {
        if !self.is_online() {
            return Ok(None);
        }
        let r: Result<IncompleteResponse, CallError> = self.post(
            "/v1/incomplete",
            &IncompleteRequest {
                ctx: self.ctx(step),
                hit: hit.and_then(|h| self.hit_ref(h)),
                what: what.to_string(),
                run,
            },
        );
        Ok(match r {
            Ok(r) => r.line,
            Err(e) => {
                self.go_offline(e.0);
                None
            }
        })
    }

    fn record_run(&self, run: RunRecord) -> Result<(), TrustError> {
        if !self.is_online() {
            return Ok(());
        }
        let step = Step {
            key: run.step_key.unwrap_or_else(|| Hash::of(b"")),
            cmd: "",
            ignore: &[],
        };
        if let Err(e) = self.post::<serde_json::Value>(
            "/v1/runs",
            &RunRequest {
                ctx: self.ctx(&step),
                run,
            },
        ) {
            self.go_offline(e.0);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn plain_http_only_to_this_machine() {
        for ok in [
            "http://127.0.0.1:8080",
            "http://localhost",
            "http://[::1]:9/x",
        ] {
            assert!(is_loopback_http(ok), "{ok}");
        }
        for bad in [
            "http://quod.internal",
            "http://127.0.0.1.evil.com",
            "http://localhost.evil.com:80",
            "https://127.0.0.1",
        ] {
            assert!(!is_loopback_http(bad), "{bad}");
        }
        let t = [
            ("QUOD_SERVER", "http://quod.internal"),
            ("QUOD_SERVER_TOKEN", "t"),
        ];
        assert!(ServerConfig::from_env(env(&t)).unwrap().is_err());
    }

    #[test]
    fn config_from_env() {
        assert!(ServerConfig::from_env(env(&[])).is_none());
        let c = ServerConfig::from_env(env(&[
            ("QUOD_SERVER", "https://api.quodci.com/"),
            ("QUOD_SERVER_TOKEN", "t"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(c.url, "https://api.quodci.com");
        assert!(matches!(c.auth, Auth::Token(ref t) if t == "t"));
        let c = ServerConfig::from_env(env(&[
            ("QUOD_SERVER", "https://api.quodci.com"),
            ("ACTIONS_ID_TOKEN_REQUEST_URL", "https://x/token?a=1"),
            ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "rt"),
        ]))
        .unwrap()
        .unwrap();
        assert!(matches!(c.auth, Auth::GithubOidc { ref audience, .. } if audience == "quodci"));
        // No credentials, a bad URL, a bad key: unusable, said why.
        assert!(
            ServerConfig::from_env(env(&[("QUOD_SERVER", "https://x")]))
                .unwrap()
                .is_err()
        );
        assert!(
            ServerConfig::from_env(env(&[
                ("QUOD_SERVER", "ftp://x"),
                ("QUOD_SERVER_TOKEN", "t")
            ]))
            .unwrap()
            .is_err()
        );
        assert!(
            ServerConfig::from_env(env(&[
                ("QUOD_SERVER", "https://x"),
                ("QUOD_SERVER_TOKEN", "t"),
                ("QUOD_SERVER_KEY", "zz")
            ]))
            .unwrap()
            .is_err()
        );
    }

    #[test]
    fn an_unreachable_server_means_the_step_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        // Nothing listens on a port bound and dropped just now.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let cfg = ServerConfig::from_env(env(&[
            ("QUOD_SERVER", &format!("http://127.0.0.1:{port}")),
            ("QUOD_SERVER_TOKEN", "t"),
        ]))
        .unwrap()
        .unwrap();
        let l = RemoteLedger::new(cfg, &cas, Policy::default());
        let step = Step {
            key: Hash::of(b"k"),
            cmd: "true",
            ignore: &[],
        };
        let (lookup, claimed) = l
            .claim(&step, &Tree::default(), &mut |_| RunRecord::default())
            .unwrap();
        assert!(lookup.hit.is_none());
        assert_eq!(
            claimed,
            Claimed::Execute {
                reason: Reason::Miss
            }
        );
        assert!(!l.is_online());
        // And nothing after it is recorded, without an error.
        assert!(l.record_run(RunRecord::default()).is_ok());
    }
}
