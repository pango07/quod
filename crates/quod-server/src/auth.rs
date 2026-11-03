//! Who is calling, and what they may touch.
//!
//! Runners authenticate with a GitHub Actions OIDC token (audience
//! `quodci` by default), verified against the issuer's JWKS, or, on a
//! self-hosted deployment, a static token. Dashboard viewers use their
//! WorkOS access token (verified against WorkOS's JWKS for the configured
//! client id; the path's team is the team's WorkOS organization, and the
//! viewer must be a member of it) or a static viewer token.
//!
//! Every principal belongs to exactly one team, and every handler reads
//! only that team's directory, so no request can name another team's
//! keys, blobs or results (the team in a dashboard path must be the
//! principal's own, else 404).
//!
//! Scopes. A team's ledger is split by repository and trust scope, as a
//! local state dir is (`scopes/<repository>/<trust>`). For an OIDC caller
//! the scope comes from GitHub's signed claims, and the rule is stricter
//! than the runner hooks' local one, because here the client runs inside
//! the job: a job's code can send anything the client could. So only a
//! writer job (a push, dispatch or schedule on one of the team's writer
//! refs, optionally protected and from an allowed workflow) commits into
//! `trusted`. Every other job of the repository reads `trusted` (it may
//! replay what writers proved) and commits into a scope of its own ref,
//! which only that ref's jobs read. Events that run with the base
//! repository's token on input someone else controls (pull_request_target,
//! workflow_run, issue_comment, ...) get `unverified`, which reads and
//! writes only itself. Fork pull requests get no OIDC token from GitHub
//! at all. A delegated runner token (quod's own runner hooks, whose jobs
//! cannot reach it) states the scope it decided, as the hooks do locally.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use quod_wire::{CiContext, ScopeHint};
use serde::Deserialize;

use crate::ApiError;
use crate::config::{Document, OidcConfig, TeamConfig, TokenConfig, TokenKind, WorkosConfig};

/// The trusted scope's name.
pub const TRUSTED: &str = "trusted";
/// The scope of jobs whose trust cannot be established.
pub const UNVERIFIED: &str = "unverified";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Admin,
    Member,
}

/// A runner, resolved to a team, a repository and its scopes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runner {
    pub team: String,
    pub repository: String,
    /// Scopes a claim may replay from, in order.
    pub read: Vec<String>,
    /// The scope its commits go to.
    pub write: String,
    pub writer: bool,
    /// `oidc` or `token`.
    pub via: &'static str,
    /// What the OIDC claims say about the job (authoritative over the
    /// client's own context).
    pub claims: Option<CiContext>,
    /// GitHub's `ref_protected` claim: whether the job's ref is protected.
    /// `None` for a static token (unknown).
    pub ref_protected: Option<bool>,
    /// GitHub's `repository_id` (OIDC): the repository's ledgers are kept
    /// under it, so a renamed repository keeps them and a new repository
    /// that takes an old name does not inherit them.
    pub repository_id: Option<u64>,
}

impl Runner {
    /// Where this runner's repository is stored.
    pub fn repo(&self) -> crate::store::RepoKey<'_> {
        crate::store::RepoKey {
            name: &self.repository,
            github_id: self.repository_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewer {
    pub team: String,
    pub role: Role,
}

#[derive(Clone, Debug)]
pub enum Principal {
    /// A runner token that lets each request say its scope.
    Delegated {
        team: String,
    },
    Runner(Box<Runner>),
    Viewer(Viewer),
}

/// GitHub's OIDC claims Quod reads. GitHub sends ids as strings.
#[derive(Clone, Debug, Deserialize)]
pub struct GithubClaims {
    pub repository: String,
    pub repository_id: String,
    pub repository_owner_id: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    #[serde(default)]
    pub ref_protected: Option<String>,
    pub event_name: String,
    #[serde(default)]
    pub workflow: Option<String>,
    #[serde(default)]
    pub job_workflow_ref: Option<String>,
    #[serde(default)]
    pub sha: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub run_number: Option<String>,
    #[serde(default)]
    pub run_attempt: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

/// An issuer's keys, cached; refetched when a token names a key id the
/// cache does not have, at most once a minute.
pub struct Jwks {
    url: String,
    file: Option<PathBuf>,
    cache: RwLock<Option<(JwkSet, Instant)>>,
    /// One fetch at a time: concurrent requests for an unknown key wait
    /// for it and then read the cache, instead of each fetching.
    fetching: Mutex<()>,
}

impl Jwks {
    pub fn new(url: String, file: Option<PathBuf>) -> Jwks {
        Jwks {
            url,
            file,
            cache: RwLock::new(None),
            fetching: Mutex::new(()),
        }
    }

    /// The cached set's key for `kid`; `Err` when the set is fresh and
    /// lacks it; `None` when a fetch is due.
    fn cached(&self, kid: &str) -> Option<Result<DecodingKey, String>> {
        let cache = self.cache.read().unwrap_or_else(|e| e.into_inner());
        let (set, at) = cache.as_ref()?;
        if let Some(k) = set.find(kid) {
            return Some(DecodingKey::from_jwk(k).map_err(|e| e.to_string()));
        }
        (at.elapsed() < Duration::from_secs(60))
            .then(|| Err(format!("no key {kid:?} in the issuer's JWKS")))
    }

    fn load(&self) -> Result<JwkSet, String> {
        if let Some(f) = &self.file {
            let j = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
            return serde_json::from_str(&j).map_err(|e| format!("JWKS: {e}"));
        }
        let url = &self.url;
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build()
            .get(url)
            .call()
            .map_err(|e| format!("JWKS {url}: {e}"))?
            .into_json()
            .map_err(|e| format!("JWKS {url}: {e}"))
    }

    /// The key for `kid`, fetching the set if needed (blocking).
    fn key(&self, kid: &str) -> Result<DecodingKey, String> {
        if let Some(k) = self.cached(kid) {
            return k;
        }
        let _one = self.fetching.lock().unwrap_or_else(|e| e.into_inner());
        // Another request may have fetched while this one waited.
        if let Some(k) = self.cached(kid) {
            return k;
        }
        let find = |set: &JwkSet| set.find(kid).map(DecodingKey::from_jwk);
        let set = self.load();
        let set = match set {
            Ok(s) => s,
            Err(e) => {
                // A failed fetch also waits a minute before the next one.
                let mut c = self.cache.write().unwrap_or_else(|e| e.into_inner());
                let old = c.take().map(|(s, _)| s).unwrap_or(JwkSet { keys: vec![] });
                *c = Some((old, Instant::now()));
                return Err(e);
            }
        };
        let k = find(&set);
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = Some((set, Instant::now()));
        match k {
            Some(k) => k.map_err(|e| e.to_string()),
            None => Err(format!("no key {kid:?} in the issuer's JWKS")),
        }
    }

    /// Verifies an RS256 token's signature, issuer, lifetime (60 s leeway)
    /// and, when `audiences` is given, its audience.
    pub fn verify<T: serde::de::DeserializeOwned>(
        &self,
        token: &str,
        issuer: &str,
        audiences: Option<&[String]>,
        required: &[&str],
    ) -> Result<T, String> {
        let header = jsonwebtoken::decode_header(token).map_err(|e| e.to_string())?;
        if header.alg != Algorithm::RS256 {
            return Err(format!(
                "algorithm {:?}; only RS256 is accepted",
                header.alg
            ));
        }
        let kid = header.kid.ok_or("the token names no key id")?;
        let key = self.key(&kid)?;
        let mut v = Validation::new(Algorithm::RS256);
        v.set_issuer(&[issuer]);
        match audiences {
            Some(a) => v.set_audience(a),
            None => v.validate_aud = false,
        }
        v.set_required_spec_claims(required);
        v.leeway = 60;
        v.validate_nbf = true;
        let claims = jsonwebtoken::decode::<serde_json::Value>(token, &key, &v)
            .map(|d| d.claims)
            .map_err(|e| e.to_string())?;
        // A token issued in the future is not accepted beyond clock skew.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if claims["iat"].as_u64().is_some_and(|iat| iat > now + 600) {
            return Err("the token is issued in the future".into());
        }
        serde_json::from_value(claims).map_err(|e| e.to_string())
    }
}

/// GitHub Actions' OIDC tokens, for runners.
pub struct GithubOidc {
    cfg: OidcConfig,
    jwks: Jwks,
}

impl GithubOidc {
    pub fn new(cfg: OidcConfig) -> GithubOidc {
        let url = cfg
            .jwks_url
            .clone()
            .unwrap_or_else(|| format!("{}/.well-known/jwks", cfg.issuer));
        GithubOidc {
            jwks: Jwks::new(url, cfg.jwks_file.clone()),
            cfg,
        }
    }

    /// Verifies a token's signature, issuer, audience and lifetime.
    pub fn verify(&self, token: &str) -> Result<GithubClaims, String> {
        self.jwks.verify(
            token,
            &self.cfg.issuer,
            Some(&self.cfg.audiences),
            &["exp", "iat", "iss", "aud"],
        )
    }
}

/// The claims of a WorkOS AuthKit access token Quod reads.
#[derive(Clone, Debug, Deserialize)]
pub struct WorkosClaims {
    /// The WorkOS user id.
    pub sub: String,
    /// The organization the session is for, if any.
    #[serde(default)]
    pub org_id: Option<String>,
    /// The viewer's role slug in `org_id`.
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}

/// WorkOS access tokens, for dashboard viewers; with `WORKOS_API_KEY`, the
/// membership API for a viewer whose token names another organization.
pub struct Workos {
    cfg: WorkosConfig,
    jwks: Jwks,
    api_key: Option<String>,
    /// (user, org) -> the active membership's role, or none; 60 s.
    memberships: Mutex<HashMap<(String, String), Membership>>,
}

/// A membership answer from WorkOS (the role, or none) and when it came.
type Membership = (Option<Role>, Instant);

/// How long a membership answer from WorkOS is reused.
const MEMBERSHIP_TTL: Duration = Duration::from_secs(60);

fn role_of(role: Option<&str>, roles: &[String]) -> Role {
    if role == Some("admin") || roles.iter().any(|r| r == "admin") {
        Role::Admin
    } else {
        Role::Member
    }
}

impl Workos {
    pub fn new(cfg: WorkosConfig, api_key: Option<String>) -> Workos {
        Workos {
            jwks: Jwks::new(cfg.jwks_url(), cfg.jwks_file.clone()),
            cfg,
            api_key,
            memberships: Mutex::new(HashMap::new()),
        }
    }

    /// Signature (RS256, the client's JWKS), issuer, `exp` and `iat` with
    /// 60 s leeway, `sub`. No audience: WorkOS sets none for Quod, and the
    /// JWKS is the client id's, so only this application's tokens verify.
    pub fn verify(&self, token: &str) -> Result<WorkosClaims, String> {
        let c: WorkosClaims = self.jwks.verify(
            token,
            &self.cfg.issuer(),
            None,
            &["exp", "iat", "iss", "sub"],
        )?;
        if c.sub.is_empty() {
            return Err("the token names no user".into());
        }
        Ok(c)
    }

    /// The viewer's role in `org`, or `None` when not an active member.
    /// The token's own organization answers for itself; any other asks
    /// WorkOS (needs `WORKOS_API_KEY`; without it, not a member).
    pub fn role_in(&self, c: &WorkosClaims, org: &str) -> Result<Option<Role>, ApiError> {
        if c.org_id.as_deref() == Some(org) {
            return Ok(Some(role_of(c.role.as_deref(), &c.roles)));
        }
        let Some(key) = &self.api_key else {
            return Ok(None);
        };
        let k = (c.sub.clone(), org.to_string());
        if let Some((r, at)) = self
            .memberships
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&k)
            && at.elapsed() < MEMBERSHIP_TTL
        {
            return Ok(r.clone());
        }
        let url = format!(
            "{}/user_management/organization_memberships",
            self.cfg.api_url()
        );
        let v: serde_json::Value = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build()
            .get(&url)
            .query("user_id", &c.sub)
            .query("organization_id", org)
            .query("statuses", "active")
            .set("authorization", &format!("Bearer {key}"))
            .call()
            .map_err(|e| {
                tracing::warn!(target: "quod", error = %e, "WorkOS membership lookup failed");
                ApiError::new(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "unavailable",
                    "the membership check with WorkOS failed; try again",
                )
            })?
            .into_json()
            .map_err(ApiError::internal)?;
        let found = v["data"].as_array().and_then(|ms| {
            ms.iter().find(|m| {
                m["organization_id"].as_str() == Some(org)
                    && m["user_id"].as_str() == Some(c.sub.as_str())
                    && m["status"].as_str().is_none_or(|s| s == "active")
            })
        });
        let role = found.map(|m| {
            let roles: Vec<String> = m["roles"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|r| r["slug"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            role_of(m["role"]["slug"].as_str(), &roles)
        });
        self.memberships
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(k, (role.clone(), Instant::now()));
        Ok(role)
    }
}

/// Authentication: the configured tokens, GitHub OIDC and WorkOS.
pub struct Auth {
    teams: Vec<TeamConfig>,
    tokens: HashMap<[u8; 32], TokenConfig>,
    github: Option<GithubOidc>,
    workos: Option<Workos>,
}

/// The bearer token of a request.
fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError::unauthorized("no bearer token"))
}

impl Auth {
    pub fn new(doc: &Document, workos_api_key: Option<String>) -> Auth {
        Auth {
            teams: doc.teams.clone(),
            tokens: doc
                .tokens
                .iter()
                .filter_map(|t| {
                    let b: [u8; 32] = hex::decode(&t.blake3).ok()?.try_into().ok()?;
                    Some((b, t.clone()))
                })
                .collect(),
            github: doc.oidc.clone().map(GithubOidc::new),
            workos: doc
                .viewers
                .as_ref()
                .and_then(|v| v.workos.clone())
                .map(|w| Workos::new(w, workos_api_key)),
        }
    }

    /// The dashboard viewer of a request for the team the path names (its
    /// slug or its WorkOS organization id). A team that does not exist, or
    /// is not the viewer's, is 404: existence does not leak. A bad or
    /// expired credential is 401; a runner's is 403.
    pub fn viewer(&self, headers: &HeaderMap, path_team: &str) -> Result<Viewer, ApiError> {
        let token = bearer(headers)?;
        let h = *blake3::hash(token.as_bytes()).as_bytes();
        let team = || {
            crate::valid_id(path_team)
                .then(|| {
                    self.teams.iter().find(|t| {
                        t.slug == path_team || t.workos_org_id.as_deref() == Some(path_team)
                    })
                })
                .flatten()
        };
        let not_found = || ApiError::not_found("no such team");
        if let Some(t) = self.tokens.get(&h) {
            return match t.kind {
                TokenKind::Viewer => match team() {
                    Some(team) if team.slug == t.team => Ok(Viewer {
                        team: team.slug.clone(),
                        role: if t.role.as_deref() == Some("admin") {
                            Role::Admin
                        } else {
                            Role::Member
                        },
                    }),
                    _ => Err(not_found()),
                },
                TokenKind::Runner => Err(ApiError::forbidden(
                    "runner tokens cannot read the dashboard",
                )),
            };
        }
        if token.split('.').count() != 3 {
            return Err(ApiError::unauthorized("unknown token"));
        }
        let workos = self.workos.as_ref().ok_or_else(|| {
            ApiError::unauthorized("viewer auth with WorkOS is not configured on this server")
        })?;
        let claims = workos.verify(token).map_err(|e| {
            tracing::info!(target: "quod", error = %e, "WorkOS access token refused");
            ApiError::unauthorized("the access token does not verify")
        })?;
        let team = team().ok_or_else(not_found)?;
        let org = team.workos_org_id.as_deref().ok_or_else(not_found)?;
        match workos.role_in(&claims, org)? {
            Some(role) => Ok(Viewer {
                team: team.slug.clone(),
                role,
            }),
            None => Err(not_found()),
        }
    }

    pub fn team(&self, slug: &str) -> Option<&TeamConfig> {
        self.teams.iter().find(|t| t.slug == slug)
    }

    pub fn teams(&self) -> &[TeamConfig] {
        &self.teams
    }

    /// The caller of a runner endpoint, from `Authorization: Bearer`.
    /// Blocking (a JWKS fetch).
    pub fn authenticate(&self, headers: &HeaderMap) -> Result<Principal, ApiError> {
        let token = bearer(headers)?;
        // A configured static token wins, whatever it looks like; anything
        // else shaped like a JWT (two dots) is an OIDC token.
        let h = *blake3::hash(token.as_bytes()).as_bytes();
        if !self.tokens.contains_key(&h) && token.split('.').count() == 3 {
            let jwks = self
                .github
                .as_ref()
                .ok_or_else(|| ApiError::unauthorized("OIDC is not configured on this server"))?;
            let claims = jwks.verify(token).map_err(|e| {
                tracing::info!(target: "quod", error = %e, "OIDC token refused");
                ApiError::unauthorized("the OIDC token does not verify")
            })?;
            return self
                .oidc_runner(&claims)
                .map(|r| Principal::Runner(Box::new(r)));
        }
        let t = self
            .tokens
            .get(&h)
            .ok_or_else(|| ApiError::unauthorized("unknown token"))?;
        Ok(match t.kind {
            TokenKind::Viewer => Principal::Viewer(Viewer {
                team: t.team.clone(),
                role: if t.role.as_deref() == Some("admin") {
                    Role::Admin
                } else {
                    Role::Member
                },
            }),
            TokenKind::Runner if t.delegate => Principal::Delegated {
                team: t.team.clone(),
            },
            TokenKind::Runner => {
                let trust = t.trust.clone().unwrap_or_else(|| TRUSTED.into());
                Principal::Runner(Box::new(Runner {
                    team: t.team.clone(),
                    repository: t.repository.clone().unwrap_or_default(),
                    read: vec![trust.clone()],
                    writer: trust == TRUSTED,
                    write: trust,
                    via: "token",
                    claims: None,
                    ref_protected: None,
                    repository_id: None,
                }))
            }
        })
    }

    /// The runner a delegated token's request describes.
    pub fn delegated(team: &str, hint: Option<&ScopeHint>) -> Result<Runner, ApiError> {
        let hint = hint.ok_or_else(|| {
            ApiError::bad_request("a delegated runner token needs a scope with every request")
        })?;
        if !crate::valid_id(&hint.trust) || !valid_repository(&hint.repository) {
            return Err(ApiError::bad_request(
                "the scope names an invalid repository or trust",
            ));
        }
        Ok(Runner {
            team: team.to_string(),
            repository: hint.repository.clone(),
            read: vec![hint.trust.clone()],
            write: hint.trust.clone(),
            writer: hint.writer && hint.trust == TRUSTED,
            via: "token",
            claims: None,
            ref_protected: None,
            repository_id: None,
        })
    }

    /// Maps verified claims to a team and scopes.
    pub fn oidc_runner(&self, c: &GithubClaims) -> Result<Runner, ApiError> {
        let owner: u64 = c
            .repository_owner_id
            .parse()
            .map_err(|_| ApiError::unauthorized("repository_owner_id is not a number"))?;
        let repo_id: u64 = c
            .repository_id
            .parse()
            .map_err(|_| ApiError::unauthorized("repository_id is not a number"))?;
        let team = self
            .teams
            .iter()
            .find(|t| {
                t.github_owner_ids.contains(&owner)
                    && (t.github_repository_ids.is_empty()
                        || t.github_repository_ids.contains(&repo_id))
            })
            .ok_or_else(|| ApiError::forbidden("this repository is not linked to a team"))?;
        if !valid_repository(&c.repository) {
            return Err(ApiError::unauthorized(
                "the repository claim is not owner/name",
            ));
        }
        let (read, write, writer) = oidc_scopes(team, c);
        Ok(Runner {
            team: team.slug.clone(),
            repository: c.repository.clone(),
            read,
            write,
            writer,
            via: "oidc",
            ref_protected: c.ref_protected.as_deref().map(|p| p == "true"),
            repository_id: Some(repo_id),
            claims: Some(CiContext {
                repository: Some(c.repository.clone()),
                run_id: c.run_id.clone(),
                run_number: c.run_number.as_deref().and_then(|v| v.parse().ok()),
                run_attempt: c.run_attempt.as_deref().and_then(|v| v.parse().ok()),
                git_ref: Some(c.git_ref.clone()),
                sha: c.sha.clone(),
                actor: c.actor.clone(),
                event: Some(c.event_name.clone()),
                workflow: c.workflow.clone(),
                ..CiContext::default()
            }),
        })
    }
}

/// (read scopes, write scope, writer) for a job, from its claims.
pub fn oidc_scopes(team: &TeamConfig, c: &GithubClaims) -> (Vec<String>, String, bool) {
    let writer_event = matches!(
        c.event_name.as_str(),
        "push" | "workflow_dispatch" | "schedule"
    );
    let writer = writer_event
        && team.writer_refs.contains(&c.git_ref)
        && (!team.requires_protected_ref(&c.repository)
            || c.ref_protected.as_deref() == Some("true"))
        && (team.writer_workflows.is_empty()
            || c.job_workflow_ref
                .as_ref()
                .is_some_and(|w| team.writer_workflows.contains(w)));
    if writer {
        return (vec![TRUSTED.into()], TRUSTED.into(), true);
    }
    // The repository's own code on another ref: may replay what writers
    // proved, never write it.
    let same_repo = matches!(
        c.event_name.as_str(),
        "push" | "workflow_dispatch" | "schedule" | "pull_request" | "merge_group"
    );
    if same_repo {
        let own = ref_scope(&c.git_ref);
        return (vec![TRUSTED.into(), own.clone()], own, false);
    }
    (vec![UNVERIFIED.into()], UNVERIFIED.into(), false)
}

/// The scope of a ref: `ref-<readable part>-<hash>`, a valid id that two
/// refs never share.
pub fn ref_scope(r: &str) -> String {
    // One definition, which the vitest client also uses to check that a
    // signed ledger is for its own ref.
    quod_wire::testreplay::ref_scope(r)
}

/// `owner/name`, each part a GitHub-shaped name.
pub fn valid_repository(r: &str) -> bool {
    let mut parts = r.split('/');
    let ok = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.len() <= 100
                && p != "."
                && p != ".."
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team() -> TeamConfig {
        crate::config::parse_document(r#"{"teams":[{"slug":"acme","github_owner_ids":[42]}]}"#)
            .unwrap()
            .teams
            .remove(0)
    }

    fn claims(event: &str, r: &str) -> GithubClaims {
        GithubClaims {
            repository: "acme/app".into(),
            repository_id: "7".into(),
            repository_owner_id: "42".into(),
            git_ref: r.into(),
            ref_protected: Some("false".into()),
            event_name: event.into(),
            workflow: Some("ci".into()),
            job_workflow_ref: Some("acme/app/.github/workflows/ci.yml@refs/heads/main".into()),
            sha: None,
            run_id: None,
            run_number: None,
            run_attempt: None,
            actor: None,
        }
    }

    #[test]
    fn only_writer_jobs_write_trusted() {
        let t = team();
        for e in ["push", "workflow_dispatch", "schedule"] {
            let (r, w, writer) = oidc_scopes(&t, &claims(e, "refs/heads/main"));
            assert_eq!(
                (r, w.as_str(), writer),
                (vec![TRUSTED.to_string()], TRUSTED, true),
                "{e}"
            );
        }
        // The same repository on another ref: reads trusted, writes its own.
        for (e, r) in [
            ("push", "refs/heads/feature"),
            ("pull_request", "refs/pull/1/merge"),
            ("push", "refs/tags/v1"),
            ("merge_group", "refs/heads/gh-readonly-queue/main/pr-1"),
        ] {
            let (read, w, writer) = oidc_scopes(&t, &claims(e, r));
            assert!(!writer, "{e} {r}");
            assert_ne!(w, TRUSTED);
            assert_eq!(read, vec![TRUSTED.to_string(), w.clone()]);
        }
        // Events on someone else's input: isolated.
        for e in [
            "pull_request_target",
            "workflow_run",
            "issue_comment",
            "release",
        ] {
            let (read, w, writer) = oidc_scopes(&t, &claims(e, "refs/heads/main"));
            assert_eq!(
                (read, w.as_str(), writer),
                (vec![UNVERIFIED.to_string()], UNVERIFIED, false),
                "{e}"
            );
        }
    }

    #[test]
    fn writer_rules_can_be_tightened() {
        let mut t = team();
        t.require_protected_ref = true;
        assert!(!oidc_scopes(&t, &claims("push", "refs/heads/main")).2);
        let mut c = claims("push", "refs/heads/main");
        c.ref_protected = Some("true".into());
        assert!(oidc_scopes(&t, &c).2);
        t.writer_workflows = vec!["acme/app/.github/workflows/release.yml@refs/heads/main".into()];
        assert!(!oidc_scopes(&t, &c).2);
    }

    #[test]
    fn a_protected_ref_is_off_by_default_and_per_repo() {
        let mut t = team();
        // Off by default (DECISIONS 2026-11-02): an unprotected main writes.
        assert!(oidc_scopes(&t, &claims("push", "refs/heads/main")).2);
        // On for one repository only.
        t.repos = vec![crate::config::RepoConfig {
            full_name: "acme/app".into(),
            default_branch: None,
            require_protected_ref: Some(true),
        }];
        assert!(!oidc_scopes(&t, &claims("push", "refs/heads/main")).2);
        // Off for one repository while the team requires it.
        t.require_protected_ref = true;
        t.repos[0].require_protected_ref = Some(false);
        assert!(oidc_scopes(&t, &claims("push", "refs/heads/main")).2);
    }

    #[test]
    fn ref_scopes_are_distinct_valid_ids() {
        let a = ref_scope("refs/heads/a/b");
        let b = ref_scope("refs/heads/a_b");
        assert_ne!(a, b);
        assert!(crate::valid_id(&a) && crate::valid_id(&b));
    }

    #[test]
    fn repositories_are_owner_slash_name() {
        assert!(valid_repository("acme/app"));
        for bad in ["acme", "acme/app/x", "../x", "acme/..", "a b/c", "/x", "x/"] {
            assert!(!valid_repository(bad), "{bad}");
        }
    }
}
