//! quod-server's configuration: everything from environment variables, so
//! a container (Railway, docs/cloud-ledger.md) needs no files. Teams,
//! tokens and OIDC come as one JSON document, inline
//! (`QUOD_SERVER_CONFIG_JSON`) or from a file (`QUOD_SERVER_CONFIG`).

use std::path::PathBuf;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use serde::Deserialize;

/// GitHub Actions' OIDC issuer.
pub const GITHUB_ISSUER: &str = "https://token.actions.githubusercontent.com";
/// The audience runners request by default.
pub const DEFAULT_AUDIENCE: &str = "quodci";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    #[serde(default)]
    pub teams: Vec<TeamConfig>,
    #[serde(default)]
    pub tokens: Vec<TokenConfig>,
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
}

/// One team: its slug (`/v1/teams/<slug>`), which GitHub accounts' jobs
/// belong to it, which refs write its trusted scope, and its policy floor.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamConfig {
    pub slug: String,
    #[serde(default)]
    pub name: Option<String>,
    /// GitHub `repository_owner_id`s (numbers, not logins: a login can be
    /// renamed and re-registered by someone else; an id cannot) whose
    /// OIDC tokens map to this team. Until the GitHub App's installation
    /// links are read from the database (P5), this is the link.
    #[serde(default)]
    pub github_owner_ids: Vec<u64>,
    /// If non-empty, only these `repository_id`s.
    #[serde(default)]
    pub github_repository_ids: Vec<u64>,
    /// Refs whose push, dispatch or schedule jobs are writers (default
    /// `refs/heads/main`). OIDC tokens carry no default-branch claim.
    #[serde(default = "default_writer_refs")]
    pub writer_refs: Vec<String>,
    /// Writers must also run on a protected ref (`ref_protected` claim).
    #[serde(default)]
    pub require_protected_ref: bool,
    /// If non-empty, writers must also run one of these workflows
    /// (`job_workflow_ref`, e.g. `acme/app/.github/workflows/ci.yml@refs/heads/main`).
    #[serde(default)]
    pub writer_workflows: Vec<String>,
    #[serde(default)]
    pub policy: TeamPolicy,
}

fn default_writer_refs() -> Vec<String> {
    vec!["refs/heads/main".into()]
}

/// The strictest policy a team accepts from its runners (`effective_policy`).
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamPolicy {
    #[serde(default = "default_proofs")]
    pub proofs: u32,
    #[serde(default = "default_audit")]
    pub audit: f64,
    #[serde(default)]
    pub replay_failures: bool,
}

fn default_proofs() -> u32 {
    quod_trust::DEFAULT_PROOFS
}

fn default_audit() -> f64 {
    quod_trust::DEFAULT_AUDIT
}

impl Default for TeamPolicy {
    fn default() -> TeamPolicy {
        TeamPolicy {
            proofs: default_proofs(),
            audit: default_audit(),
            replay_failures: false,
        }
    }
}

impl TeamPolicy {
    pub fn policy(&self) -> quod_trust::Policy {
        quod_trust::Policy {
            mode: quod_trust::Mode::Observe,
            proofs: self.proofs,
            audit: self.audit,
            replay_failures: self.replay_failures,
        }
    }
}

/// A static token. Stored as the BLAKE3 hash of the token (`quod-server
/// hash-token`), never the token.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenConfig {
    /// BLAKE3 of the token, hex.
    pub blake3: String,
    pub team: String,
    pub kind: TokenKind,
    /// Runner tokens: the repository (`owner/name`) its jobs belong to,
    /// unless `delegate` lets the runner say per request.
    #[serde(default)]
    pub repository: Option<String>,
    /// Runner tokens: the scope (default `trusted`).
    #[serde(default)]
    pub trust: Option<String>,
    /// Runner tokens: the runner infrastructure decides each job's
    /// repository and scope (quod's own runner hooks do, from the event
    /// payload, as they do with a local ledger). Only for runners whose
    /// jobs cannot reach the token.
    #[serde(default)]
    pub delegate: bool,
    /// Viewer tokens: `admin` or `member`.
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    Runner,
    Viewer,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcConfig {
    #[serde(default = "default_issuer")]
    pub issuer: String,
    #[serde(default = "default_audiences")]
    pub audiences: Vec<String>,
    /// Where the issuer's JWKS is (default: the issuer's
    /// `/.well-known/jwks`). `jwks_file` instead reads it from disk (tests,
    /// air-gapped GHES).
    #[serde(default)]
    pub jwks_url: Option<String>,
    #[serde(default)]
    pub jwks_file: Option<PathBuf>,
}

fn default_issuer() -> String {
    GITHUB_ISSUER.into()
}

fn default_audiences() -> Vec<String> {
    vec![DEFAULT_AUDIENCE.into()]
}

/// Everything the server runs with.
#[derive(Clone)]
pub struct Settings {
    pub bind: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub signing_key: SigningKey,
    pub doc: Document,
    /// How long one event stream is held before `event: bye`.
    pub sse_hold: Duration,
    /// The largest blob accepted.
    pub max_blob: u64,
    /// The largest JSON body accepted (a claim carries the whole tree).
    pub max_json: usize,
}

impl Settings {
    /// From the environment. Refuses what this build cannot honor rather
    /// than run with less: `DATABASE_URL` or object-store settings (the
    /// Postgres and S3 backends are P4; serving from local disk on a host
    /// without a persistent disk would lose every result on redeploy), and
    /// a missing signing key outside `QUOD_DEV=1`.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Settings, String> {
        let dev = get("QUOD_DEV").is_some_and(|v| v == "1");
        for v in ["DATABASE_URL", "S3_ENDPOINT", "S3_BUCKET"] {
            if get(v).is_some_and(|x| !x.is_empty()) {
                return Err(format!(
                    "{v} is set, but this build stores only in SQLite and a local directory (QUOD_DATA_DIR); the Postgres and object-store backends are not built yet (docs/cloud-ledger.md, P4). Unset {v} to run self-hosted."
                ));
            }
        }
        let data_dir = match get("QUOD_DATA_DIR") {
            Some(d) if !d.is_empty() => PathBuf::from(d),
            _ if dev => PathBuf::from("quod-data"),
            _ => return Err("QUOD_DATA_DIR is not set (a persistent directory for the SQLite ledgers and blobs)".into()),
        };
        let signing_key = match (get("QUOD_SIGNING_KEY"), get("QUOD_SIGNING_KEY_FILE")) {
            (Some(k), _) if !k.is_empty() => {
                quod_wire::signing_key_from_hex(&k).map_err(|e| format!("QUOD_SIGNING_KEY: {e}"))?
            }
            (_, Some(f)) if !f.is_empty() => {
                let k = std::fs::read_to_string(&f)
                    .map_err(|e| format!("QUOD_SIGNING_KEY_FILE {f}: {e}"))?;
                quod_wire::signing_key_from_hex(&k)
                    .map_err(|e| format!("QUOD_SIGNING_KEY_FILE {f}: {e}"))?
            }
            _ if dev => {
                tracing::warn!(target: "quod", "QUOD_DEV=1 and no QUOD_SIGNING_KEY: signing with a key generated for this process only");
                generate_key()?
            }
            _ => {
                return Err(
                    "QUOD_SIGNING_KEY (an Ed25519 seed, 64 hex digits; `quod-server keygen`) or QUOD_SIGNING_KEY_FILE is required: a key generated per instance would make every runner that pinned the last one refuse its replays".into(),
                );
            }
        };
        let doc = match (get("QUOD_SERVER_CONFIG_JSON"), get("QUOD_SERVER_CONFIG")) {
            (Some(j), _) if !j.is_empty() => parse_document(&j)?,
            (_, Some(p)) if !p.is_empty() => parse_document(
                &std::fs::read_to_string(&p).map_err(|e| format!("QUOD_SERVER_CONFIG {p}: {e}"))?,
            )?,
            _ => Document {
                teams: vec![],
                tokens: vec![],
                oidc: None,
            },
        };
        let port = match get("PORT") {
            Some(p) => p.parse().map_err(|_| format!("PORT {p:?} is not a port"))?,
            None => 8080,
        };
        Ok(Settings {
            bind: get("QUOD_BIND").unwrap_or_else(|| "0.0.0.0".into()),
            port,
            data_dir,
            signing_key,
            doc,
            sse_hold: Duration::from_secs(
                get("QUOD_SSE_HOLD_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(240),
            ),
            max_blob: get("QUOD_MAX_BLOB_BYTES")
                .and_then(|v| v.parse().ok())
                .unwrap_or(8 << 30),
            max_json: get("QUOD_MAX_JSON_BYTES")
                .and_then(|v| v.parse().ok())
                .unwrap_or(256 << 20),
        })
    }
}

/// Parses and checks the teams/tokens/OIDC document.
pub fn parse_document(json: &str) -> Result<Document, String> {
    let doc: Document = serde_json::from_str(json).map_err(|e| format!("server config: {e}"))?;
    for t in &doc.teams {
        if !crate::valid_id(&t.slug) {
            return Err(format!("team slug {:?} is not a valid id", t.slug));
        }
        let p = t.policy.policy();
        p.validate().map_err(|e| format!("team {}: {e}", t.slug))?;
    }
    for t in &doc.tokens {
        if !doc.teams.iter().any(|x| x.slug == t.team) {
            return Err(format!(
                "a token names team {:?}, which is not configured",
                t.team
            ));
        }
        if hex::decode(&t.blake3).map(|b| b.len()) != Ok(32) {
            return Err("a token's blake3 must be 64 hex digits (`quod-server hash-token`)".into());
        }
        if let Some(s) = &t.trust
            && !crate::valid_id(s)
        {
            return Err(format!("token trust {s:?} is not a valid scope name"));
        }
    }
    Ok(doc)
}

/// A fresh Ed25519 key from the OS's randomness.
pub fn generate_key() -> Result<SigningKey, String> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| format!("randomness: {e}"))?;
    Ok(SigningKey::from_bytes(&seed))
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
    fn refuses_what_it_cannot_honor() {
        let key = "11".repeat(32);
        let ok = Settings::from_env(env(&[("QUOD_DATA_DIR", "/d"), ("QUOD_SIGNING_KEY", &key)]));
        assert!(ok.is_ok());
        // No signing key outside dev mode: refused, never generated.
        assert!(Settings::from_env(env(&[("QUOD_DATA_DIR", "/d")])).is_err());
        // Postgres or an object store configured: refused (P4), never ignored.
        for v in ["DATABASE_URL", "S3_ENDPOINT", "S3_BUCKET"] {
            let e = Settings::from_env(env(&[
                ("QUOD_DATA_DIR", "/d"),
                ("QUOD_SIGNING_KEY", &key),
                (v, "x"),
            ]));
            assert!(e.err().unwrap().contains(v));
        }
        // PORT is honored.
        let s = Settings::from_env(env(&[
            ("QUOD_DATA_DIR", "/d"),
            ("QUOD_SIGNING_KEY", &key),
            ("PORT", "4321"),
        ]))
        .unwrap();
        assert_eq!(s.port, 4321);
    }

    #[test]
    fn a_token_for_an_unknown_team_is_refused() {
        let j = format!(
            r#"{{"teams":[{{"slug":"acme"}}],"tokens":[{{"blake3":"{}","team":"other","kind":"runner"}}]}}"#,
            "00".repeat(32)
        );
        assert!(parse_document(&j).is_err());
        let j = format!(
            r#"{{"teams":[{{"slug":"acme"}}],"tokens":[{{"blake3":"{}","team":"acme","kind":"runner"}}]}}"#,
            "00".repeat(32)
        );
        let d = parse_document(&j).unwrap();
        assert_eq!(d.teams[0].writer_refs, vec!["refs/heads/main".to_string()]);
    }
}
