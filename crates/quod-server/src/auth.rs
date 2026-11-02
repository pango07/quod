//! Who is calling, and what they may touch.
//!
//! Runners authenticate with a GitHub Actions OIDC token (audience
//! `quodci` by default), verified against the issuer's JWKS, or, on a
//! self-hosted deployment, a static token. Dashboard viewers use static
//! viewer tokens here; WorkOS access tokens are P5.
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
use std::sync::RwLock;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use quod_wire::{CiContext, ScopeHint};
use serde::Deserialize;

use crate::ApiError;
use crate::config::{Document, OidcConfig, TeamConfig, TokenConfig, TokenKind};

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

/// The issuer's keys, cached; refetched when a token names a key id the
/// cache does not have, at most once a minute.
pub struct Jwks {
    cfg: OidcConfig,
    cache: RwLock<Option<(JwkSet, Instant)>>,
}

impl Jwks {
    pub fn new(cfg: OidcConfig) -> Jwks {
        Jwks {
            cfg,
            cache: RwLock::new(None),
        }
    }

    fn load(&self) -> Result<JwkSet, String> {
        if let Some(f) = &self.cfg.jwks_file {
            let j = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
            return serde_json::from_str(&j).map_err(|e| format!("JWKS: {e}"));
        }
        let url = self
            .cfg
            .jwks_url
            .clone()
            .unwrap_or_else(|| format!("{}/.well-known/jwks", self.cfg.issuer));
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build()
            .get(&url)
            .call()
            .map_err(|e| format!("JWKS {url}: {e}"))?
            .into_json()
            .map_err(|e| format!("JWKS {url}: {e}"))
    }

    /// The key for `kid`, fetching the set if needed (blocking).
    fn key(&self, kid: &str) -> Result<DecodingKey, String> {
        let find = |set: &JwkSet| set.find(kid).map(DecodingKey::from_jwk);
        if let Some((set, at)) = self.cache.read().unwrap().as_ref() {
            if let Some(k) = find(set) {
                return k.map_err(|e| e.to_string());
            }
            if at.elapsed() < Duration::from_secs(60) {
                return Err(format!("no key {kid:?} in the issuer's JWKS"));
            }
        }
        let set = self.load()?;
        let k = find(&set);
        *self.cache.write().unwrap() = Some((set, Instant::now()));
        match k {
            Some(k) => k.map_err(|e| e.to_string()),
            None => Err(format!("no key {kid:?} in the issuer's JWKS")),
        }
    }

    /// Verifies a token's signature, issuer, audience and lifetime.
    pub fn verify(&self, token: &str) -> Result<GithubClaims, String> {
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
        v.set_issuer(&[&self.cfg.issuer]);
        v.set_audience(&self.cfg.audiences);
        v.set_required_spec_claims(&["exp", "iat", "iss", "aud"]);
        v.leeway = 60;
        jsonwebtoken::decode::<GithubClaims>(token, &key, &v)
            .map(|d| d.claims)
            .map_err(|e| e.to_string())
    }
}

/// Authentication: the configured tokens and OIDC.
pub struct Auth {
    teams: Vec<TeamConfig>,
    tokens: HashMap<[u8; 32], TokenConfig>,
    jwks: Option<Jwks>,
}

impl Auth {
    pub fn new(doc: &Document) -> Auth {
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
            jwks: doc.oidc.clone().map(Jwks::new),
        }
    }

    pub fn team(&self, slug: &str) -> Option<&TeamConfig> {
        self.teams.iter().find(|t| t.slug == slug)
    }

    pub fn teams(&self) -> &[TeamConfig] {
        &self.teams
    }

    /// The caller, from `Authorization: Bearer`. Blocking (a JWKS fetch).
    pub fn authenticate(&self, headers: &HeaderMap) -> Result<Principal, ApiError> {
        let token = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.strip_prefix("Bearer ")
                    .or_else(|| v.strip_prefix("bearer "))
            })
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| ApiError::unauthorized("no bearer token"))?;
        // A JWT has two dots; a static token never needs to.
        if token.split('.').count() == 3 {
            let jwks = self
                .jwks
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
        let h = *blake3::hash(token.as_bytes()).as_bytes();
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
        && (!team.require_protected_ref || c.ref_protected.as_deref() == Some("true"))
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
    format!(
        "ref-{readable}-{}",
        &blake3::hash(r.as_bytes()).to_hex()[..12]
    )
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
