//! quod-server's configuration. Plain settings come from environment
//! variables; teams, tokens, OIDC and viewer auth come as one JSON document,
//! from `QUOD_TEAMS_JSON` (inline), `QUOD_SERVER_CONFIG` (a path), or
//! `$QUOD_DATA_DIR/teams.json` on the volume. What needs the disk (the data
//! dir checks, the instance lock, the signing key) is `boot`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ed25519_dalek::SigningKey;
use serde::Deserialize;

/// GitHub Actions' OIDC issuer.
pub const GITHUB_ISSUER: &str = "https://token.actions.githubusercontent.com";
/// The audience runners request by default.
pub const DEFAULT_AUDIENCE: &str = "quodci";
/// WorkOS's API, for the JWKS and the membership fallback.
pub const WORKOS_API: &str = "https://api.workos.com";
/// The teams document's file name inside the data dir.
pub const TEAMS_FILE: &str = "teams.json";
/// The signing key's file name inside the data dir.
pub const KEY_FILE: &str = "signing.ed25519";
/// The largest blob accepted unless `QUOD_MAX_BLOB_BYTES` says otherwise.
pub const DEFAULT_MAX_BLOB: u64 = 1 << 30;
/// The largest JSON body unless `QUOD_MAX_JSON_BYTES` says otherwise (a
/// claim carries the whole workspace tree).
pub const DEFAULT_MAX_JSON: u64 = 32 << 20;

/// A secret from the environment: its `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    #[serde(default)]
    pub teams: Vec<TeamConfig>,
    #[serde(default)]
    pub tokens: Vec<TokenConfig>,
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
    /// How dashboard viewers authenticate besides static viewer tokens.
    #[serde(default)]
    pub viewers: Option<ViewersConfig>,
}

/// One team: its slug (`/v1/teams/<slug>`), which GitHub accounts' jobs
/// belong to it, which WorkOS organization its viewers come from, which
/// refs write its trusted scope, and its policy floor.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamConfig {
    pub slug: String,
    #[serde(default)]
    pub name: Option<String>,
    /// The WorkOS organization whose members view this team (the
    /// dashboard sends it as the path's team id, `org_...`).
    #[serde(default)]
    pub workos_org_id: Option<String>,
    /// The GitHub App installation linked to the team, for the record (an
    /// OIDC token names the repository and its owner, not the
    /// installation; `github_owner_ids` and `github_repository_ids` are
    /// what runner auth checks).
    #[serde(default)]
    pub github_installation_id: Option<u64>,
    /// GitHub `repository_owner_id`s (numbers, not logins: a login can be
    /// renamed and re-registered by someone else; an id cannot) whose
    /// OIDC tokens map to this team.
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
    /// Off by default (DECISIONS 2026-11-02): an unprotected writer ref is
    /// recorded and warned about instead.
    #[serde(default)]
    pub require_protected_ref: bool,
    /// If non-empty, writers must also run one of these workflows
    /// (`job_workflow_ref`, e.g. `acme/app/.github/workflows/ci.yml@refs/heads/main`).
    #[serde(default)]
    pub writer_workflows: Vec<String>,
    /// Per-repository settings.
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
    #[serde(default)]
    pub policy: TeamPolicy,
}

/// A repository's settings within its team.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    /// `owner/name`.
    pub full_name: String,
    /// Its default branch (the dashboard's `defaultBranch`); otherwise the
    /// branch of the team's first writer ref.
    #[serde(default)]
    pub default_branch: Option<String>,
    /// Overrides the team's `require_protected_ref` for this repository.
    #[serde(default)]
    pub require_protected_ref: Option<bool>,
}

impl TeamConfig {
    pub fn repo(&self, full_name: &str) -> Option<&RepoConfig> {
        self.repos.iter().find(|r| r.full_name == full_name)
    }

    /// Whether a writer of `repository` must run on a protected ref.
    pub fn requires_protected_ref(&self, repository: &str) -> bool {
        self.repo(repository)
            .and_then(|r| r.require_protected_ref)
            .unwrap_or(self.require_protected_ref)
    }

    /// The default branch of `repository`, as far as the server knows.
    pub fn default_branch(&self, repository: &str) -> String {
        self.repo(repository)
            .and_then(|r| r.default_branch.clone())
            .or_else(|| {
                self.writer_refs
                    .iter()
                    .find_map(|r| r.strip_prefix("refs/heads/").map(String::from))
            })
            .unwrap_or_else(|| "main".into())
    }
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
    #[serde(default = "default_shadow")]
    pub shadow: u32,
    #[serde(default = "default_audit")]
    pub audit: f64,
    #[serde(default)]
    pub replay_failures: bool,
}

fn default_proofs() -> u32 {
    quod_trust::DEFAULT_PROOFS
}

fn default_shadow() -> u32 {
    quod_trust::DEFAULT_SHADOW
}

fn default_audit() -> f64 {
    quod_trust::DEFAULT_AUDIT
}

impl Default for TeamPolicy {
    fn default() -> TeamPolicy {
        TeamPolicy {
            proofs: default_proofs(),
            shadow: default_shadow(),
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
            shadow: self.shadow,
            audit: self.audit,
            replay_failures: self.replay_failures,
            // A server never defers an audit (`quod_wire::effective_policy`).
            defer: false,
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

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewersConfig {
    #[serde(default)]
    pub workos: Option<WorkosConfig>,
}

/// WorkOS access tokens as dashboard viewer credentials
/// (cloud/docs/api.md, "Server changes needed" 1). The client id is
/// public; the API key (`WORKOS_API_KEY`, environment only) is needed only
/// for a viewer whose token names another organization than the path's.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkosConfig {
    pub client_id: String,
    /// The access tokens' `iss` (default
    /// `https://api.workos.com/user_management/<client_id>`; a custom auth
    /// domain's issuer otherwise).
    #[serde(default)]
    pub issuer: Option<String>,
    /// Default `https://api.workos.com/sso/jwks/<client_id>`; `jwks_file`
    /// reads it from disk (tests).
    #[serde(default)]
    pub jwks_url: Option<String>,
    #[serde(default)]
    pub jwks_file: Option<PathBuf>,
    /// The API base for the membership fallback (default WorkOS's; tests).
    #[serde(default)]
    pub api_url: Option<String>,
}

impl WorkosConfig {
    pub fn issuer(&self) -> String {
        self.issuer
            .clone()
            .unwrap_or_else(|| format!("{WORKOS_API}/user_management/{}", self.client_id))
    }

    pub fn jwks_url(&self) -> String {
        self.jwks_url
            .clone()
            .unwrap_or_else(|| format!("{WORKOS_API}/sso/jwks/{}", self.client_id))
    }

    pub fn api_url(&self) -> String {
        self.api_url
            .clone()
            .unwrap_or_else(|| WORKOS_API.into())
            .trim_end_matches('/')
            .to_string()
    }
}

/// Everything the server runs with.
#[derive(Clone)]
pub struct Settings {
    pub bind: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub signing_key: SigningKey,
    pub doc: Document,
    /// `WORKOS_API_KEY`, for the membership fallback. Never logged.
    pub workos_api_key: Option<String>,
    /// How long one event stream is held before `event: bye`.
    pub sse_hold: Duration,
    /// The largest blob accepted.
    pub max_blob: u64,
    /// The largest JSON body accepted (a claim carries the whole tree).
    pub max_json: usize,
}

impl Settings {
    /// Settings with the defaults, for tests and embedding.
    pub fn new(data_dir: impl Into<PathBuf>, signing_key: SigningKey, doc: Document) -> Settings {
        Settings {
            bind: "127.0.0.1".into(),
            port: 0,
            data_dir: data_dir.into(),
            signing_key,
            doc,
            workos_api_key: None,
            sse_hold: Duration::from_secs(240),
            max_blob: DEFAULT_MAX_BLOB,
            max_json: DEFAULT_MAX_JSON as usize,
        }
    }
}

/// The environment, read once: everything but what needs the disk.
#[derive(Clone, Debug)]
pub struct Env {
    pub dev: bool,
    pub bind: String,
    pub port: u16,
    pub data_dir: PathBuf,
    /// `QUOD_DATA_DIR_PERSISTENT=1`: the operator attests the data dir
    /// survives restarts and redeploys.
    pub persistent_attested: bool,
    /// Railway's mount path of the service's volume, if one is attached.
    pub railway_volume: Option<PathBuf>,
    /// Whether this runs on Railway at all.
    pub on_railway: bool,
    /// `QUOD_SIGNING_KEY`: a seed in the environment (self-hosting).
    pub signing_key_hex: Option<Secret>,
    pub key_file: PathBuf,
    pub teams: TeamsSource,
    pub workos_client_id: Option<String>,
    pub workos_issuer: Option<String>,
    pub workos_api_key: Option<Secret>,
    pub lock_wait: Duration,
    pub sse_hold: Duration,
    pub max_blob: u64,
    pub max_json: usize,
}

/// Where the teams document comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeamsSource {
    Inline(String),
    File(PathBuf),
    /// `$QUOD_DATA_DIR/teams.json` if it exists, else no teams.
    DataDir,
}

impl Env {
    /// From the environment. Refuses what this build cannot honor rather
    /// than run with less: `DATABASE_URL` or object-store settings (the
    /// Postgres and S3 backends are P4), and no `QUOD_DATA_DIR` outside
    /// `QUOD_DEV=1`.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Env, String> {
        let set = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let dev = get("QUOD_DEV").is_some_and(|v| v == "1");
        for v in [
            "DATABASE_URL",
            "S3_ENDPOINT",
            "S3_BUCKET",
            "S3_ACCESS_KEY_ID",
            "S3_SECRET_ACCESS_KEY",
        ] {
            if set(v).is_some() {
                return Err(format!(
                    "{v} is set, but this build stores only in SQLite and a local directory (QUOD_DATA_DIR on a persistent volume); the Postgres and object-store backends are not built yet (docs/cloud-ledger.md, P4). Unset {v}."
                ));
            }
        }
        let data_dir = match set("QUOD_DATA_DIR") {
            Some(d) => PathBuf::from(d),
            None if dev => PathBuf::from("quod-data"),
            None => {
                return Err("QUOD_DATA_DIR is not set (the persistent directory for the SQLite ledgers, blobs and the signing key, e.g. a Railway volume at /data)".into());
            }
        };
        if !data_dir.is_absolute() && !dev {
            return Err(format!(
                "QUOD_DATA_DIR {} is not an absolute path",
                data_dir.display()
            ));
        }
        let persistent_attested = match get("QUOD_DATA_DIR_PERSISTENT").as_deref() {
            None | Some("") | Some("0") => false,
            Some("1") => true,
            Some(v) => {
                return Err(format!(
                    "QUOD_DATA_DIR_PERSISTENT is {v:?}; set it to 1 only when QUOD_DATA_DIR is on a persistent volume"
                ));
            }
        };
        let key_file = set("QUOD_SIGNING_KEY_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join(KEY_FILE));
        let teams = match (
            set("QUOD_TEAMS_JSON").or_else(|| set("QUOD_SERVER_CONFIG_JSON")),
            set("QUOD_SERVER_CONFIG"),
        ) {
            (Some(_), Some(_)) => {
                return Err("both QUOD_TEAMS_JSON and QUOD_SERVER_CONFIG are set: use one".into());
            }
            (Some(j), None) => TeamsSource::Inline(j),
            (None, Some(p)) => TeamsSource::File(PathBuf::from(p)),
            (None, None) => TeamsSource::DataDir,
        };
        let port = match set("PORT") {
            Some(p) => p.parse().map_err(|_| format!("PORT {p:?} is not a port"))?,
            None => 8080,
        };
        let num = |k: &str, d: u64| -> Result<u64, String> {
            match set(k) {
                Some(v) => v.parse().map_err(|_| format!("{k} {v:?} is not a number")),
                None => Ok(d),
            }
        };
        Ok(Env {
            dev,
            bind: set("QUOD_BIND").unwrap_or_else(|| "0.0.0.0".into()),
            port,
            persistent_attested,
            railway_volume: set("RAILWAY_VOLUME_MOUNT_PATH").map(PathBuf::from),
            on_railway: set("RAILWAY_PROJECT_ID").is_some()
                || set("RAILWAY_ENVIRONMENT_ID").is_some(),
            signing_key_hex: set("QUOD_SIGNING_KEY").map(Secret),
            key_file,
            teams,
            workos_client_id: set("WORKOS_CLIENT_ID"),
            workos_issuer: set("WORKOS_ISSUER"),
            workos_api_key: set("WORKOS_API_KEY").map(Secret),
            lock_wait: Duration::from_secs(num("QUOD_LOCK_WAIT_SECS", 30)?),
            sse_hold: Duration::from_secs(num("QUOD_SSE_HOLD_SECS", 240)?),
            max_blob: num("QUOD_MAX_BLOB_BYTES", DEFAULT_MAX_BLOB)?,
            max_json: num("QUOD_MAX_JSON_BYTES", DEFAULT_MAX_JSON)? as usize,
            data_dir,
        })
    }

    /// The teams document from its source (the data dir's file is read
    /// only once the data dir is checked).
    pub fn document(&self) -> Result<Document, String> {
        let mut doc = match &self.teams {
            TeamsSource::Inline(j) => parse_document(j)?,
            TeamsSource::File(p) => read_document(p)?,
            TeamsSource::DataDir => {
                let p = self.data_dir.join(TEAMS_FILE);
                if p.is_file() {
                    read_document(&p)?
                } else {
                    Document::default()
                }
            }
        };
        // WORKOS_CLIENT_ID (not secret) turns WorkOS viewer auth on when
        // the document does not configure it.
        if let Some(id) = &self.workos_client_id {
            let viewers = doc.viewers.get_or_insert_with(ViewersConfig::default);
            if viewers.workos.is_none() {
                viewers.workos = Some(WorkosConfig {
                    client_id: id.clone(),
                    issuer: self.workos_issuer.clone(),
                    jwks_url: None,
                    jwks_file: None,
                    api_url: None,
                });
            }
        }
        check_document(&doc)?;
        Ok(doc)
    }
}

fn read_document(p: &Path) -> Result<Document, String> {
    parse_document(
        &std::fs::read_to_string(p).map_err(|e| format!("teams config {}: {e}", p.display()))?,
    )
}

/// Parses and checks the teams/tokens/OIDC/viewers document.
pub fn parse_document(json: &str) -> Result<Document, String> {
    let doc: Document = serde_json::from_str(json).map_err(|e| format!("teams config: {e}"))?;
    check_document(&doc)?;
    Ok(doc)
}

fn check_document(doc: &Document) -> Result<(), String> {
    let mut slugs = std::collections::BTreeSet::new();
    for t in &doc.teams {
        if !crate::valid_id(&t.slug) {
            return Err(format!("team slug {:?} is not a valid id", t.slug));
        }
        if !slugs.insert(&t.slug) {
            return Err(format!("team {} is configured twice", t.slug));
        }
        let p = t.policy.policy();
        p.validate().map_err(|e| format!("team {}: {e}", t.slug))?;
        if let Some(o) = &t.workos_org_id
            && !crate::valid_id(o)
        {
            return Err(format!("team {}: workos_org_id {o:?} is not an id", t.slug));
        }
        for r in &t.repos {
            if !crate::auth::valid_repository(&r.full_name) {
                return Err(format!(
                    "team {}: repository {:?} is not owner/name",
                    t.slug, r.full_name
                ));
            }
        }
    }
    // One GitHub owner, one team; one WorkOS organization, one team; and
    // no org id that is another team's slug (the dashboard path takes
    // either).
    let mut owners = std::collections::BTreeMap::new();
    let mut orgs = std::collections::BTreeMap::new();
    for t in &doc.teams {
        for o in &t.github_owner_ids {
            if let Some(other) = owners.insert(*o, &t.slug) {
                return Err(format!(
                    "GitHub owner id {o} is linked to both {other} and {}",
                    t.slug
                ));
            }
        }
        if let Some(o) = &t.workos_org_id {
            if let Some(other) = orgs.insert(o.clone(), &t.slug) {
                return Err(format!(
                    "WorkOS organization {o} is linked to both {other} and {}",
                    t.slug
                ));
            }
            if slugs.iter().any(|s| *s == o && **s != t.slug) {
                return Err(format!("WorkOS organization {o} is also a team slug"));
            }
        }
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
    if let Some(w) = doc.viewers.as_ref().and_then(|v| v.workos.as_ref())
        && (w.client_id.is_empty()
            || !w
                .client_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'))
    {
        return Err(format!(
            "WorkOS client id {:?} is not a client id",
            w.client_id
        ));
    }
    Ok(())
}

/// A fresh Ed25519 key from the OS's randomness.
pub fn generate_key() -> Result<SigningKey, String> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| format!("randomness: {e}"))?;
    Ok(SigningKey::from_bytes(&seed))
}

/// The public key's fingerprint, as logs and `/v1/keys` show it
/// (`quod_wire::key_fingerprint`). Enough to check by eye that a pinned
/// key (`QUOD_SERVER_KEY`) is the one the server loaded.
pub fn fingerprint(key: &ed25519_dalek::VerifyingKey) -> String {
    quod_wire::key_fingerprint(key)
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
        let ok = Env::from_env(env(&[("QUOD_DATA_DIR", "/d")])).unwrap();
        assert_eq!(ok.key_file, PathBuf::from("/d/signing.ed25519"));
        assert_eq!(ok.teams, TeamsSource::DataDir);
        assert!(!ok.persistent_attested);
        // No data dir outside dev mode, or a relative one: refused.
        assert!(Env::from_env(env(&[])).is_err());
        assert!(Env::from_env(env(&[("QUOD_DATA_DIR", "data")])).is_err());
        // Postgres or an object store configured: refused (P4), never ignored.
        for v in [
            "DATABASE_URL",
            "S3_ENDPOINT",
            "S3_BUCKET",
            "S3_SECRET_ACCESS_KEY",
        ] {
            let e = Env::from_env(env(&[("QUOD_DATA_DIR", "/d"), (v, "x")]));
            assert!(e.err().unwrap().contains(v));
        }
        // The attestation is 1 or nothing.
        assert!(
            Env::from_env(env(&[
                ("QUOD_DATA_DIR", "/d"),
                ("QUOD_DATA_DIR_PERSISTENT", "1")
            ]))
            .unwrap()
            .persistent_attested
        );
        assert!(
            Env::from_env(env(&[
                ("QUOD_DATA_DIR", "/d"),
                ("QUOD_DATA_DIR_PERSISTENT", "yes")
            ]))
            .is_err()
        );
        // PORT and the key file are honored; two teams sources are refused.
        let s = Env::from_env(env(&[
            ("QUOD_DATA_DIR", "/d"),
            ("PORT", "4321"),
            ("QUOD_SIGNING_KEY_FILE", "/k/key"),
        ]))
        .unwrap();
        assert_eq!((s.port, s.key_file), (4321, PathBuf::from("/k/key")));
        assert!(
            Env::from_env(env(&[
                ("QUOD_DATA_DIR", "/d"),
                ("QUOD_TEAMS_JSON", "{}"),
                ("QUOD_SERVER_CONFIG", "/x.json"),
            ]))
            .is_err()
        );
    }

    #[test]
    fn secrets_never_show_in_debug() {
        let e = Env::from_env(env(&[
            ("QUOD_DATA_DIR", "/d"),
            ("WORKOS_API_KEY", "sk_live_secret"),
            ("QUOD_SIGNING_KEY", "ab"),
        ]))
        .unwrap();
        let d = format!("{e:?}");
        assert!(
            !d.contains("sk_live_secret") && d.contains("redacted"),
            "{d}"
        );
        assert_eq!(e.max_json, 32 << 20);
        assert_eq!(e.max_blob, 1 << 30);
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
        // One owner linked to two teams: refused.
        let j = r#"{"teams":[{"slug":"a","github_owner_ids":[1]},{"slug":"b","github_owner_ids":[2,1]}]}"#;
        assert!(parse_document(j).unwrap_err().contains("owner id 1"));
        // One WorkOS organization linked to two teams: refused.
        let j = r#"{"teams":[{"slug":"a","workos_org_id":"org_1"},{"slug":"b","workos_org_id":"org_1"}]}"#;
        assert!(parse_document(j).unwrap_err().contains("org_1"));
    }

    #[test]
    fn the_go_live_team_parses() {
        let j = r#"{
          "teams": [{"slug": "quodci", "name": "quodci", "workos_org_id": "org_01QUODCI",
                     "github_installation_id": 177628965,
                     "github_owner_ids": [104618549], "github_repository_ids": [1440517147],
                     "repos": [{"full_name": "pango07/quod", "default_branch": "main"}]}],
          "oidc": {"audiences": ["quodci"]},
          "viewers": {"workos": {"client_id": "client_01ABC"}}
        }"#;
        let d = parse_document(j).unwrap();
        let t = &d.teams[0];
        assert!(!t.require_protected_ref && !t.requires_protected_ref("pango07/quod"));
        assert_eq!(t.default_branch("pango07/quod"), "main");
        assert_eq!(t.policy.proofs, quod_trust::DEFAULT_PROOFS);
        let w = d.viewers.unwrap().workos.unwrap();
        assert_eq!(
            w.issuer(),
            "https://api.workos.com/user_management/client_01ABC"
        );
        assert_eq!(w.jwks_url(), "https://api.workos.com/sso/jwks/client_01ABC");
    }

    #[test]
    fn workos_client_id_from_the_environment() {
        let e = Env::from_env(env(&[
            ("QUOD_DATA_DIR", "/d"),
            (
                "QUOD_TEAMS_JSON",
                r#"{"teams":[{"slug":"quodci","workos_org_id":"org_1"}]}"#,
            ),
            ("WORKOS_CLIENT_ID", "client_01ABC"),
        ]))
        .unwrap();
        let d = e.document().unwrap();
        assert_eq!(d.viewers.unwrap().workos.unwrap().client_id, "client_01ABC");
    }
}
