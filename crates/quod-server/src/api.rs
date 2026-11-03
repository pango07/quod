//! The runner endpoints of quod.v1. Each is one blocking unit of work:
//! authenticate, resolve the team and scopes, then run quod-trust's calls
//! on the scope's ledger inside one transaction (the same `LocalLedger`
//! a laptop uses), then note the step for the dashboard.

use axum::Extension;
use axum::extract::State;
use quod_cas::{BlobStore, Cas, Hash, Tree};
use quod_ledger::{Ledger, Lookup, RunRecord, TrustState, now_rfc3339};
use quod_manifest::{Manifest, StepResult, manifest_id};
use quod_trust::{
    Claimed, Decision, LocalLedger, Policy, Reason, Replayed, StepLedger, Trust, Verdict,
};
use quod_wire::{
    ClaimRequest, ClaimResponse, CommitRequest, CommitResponse, Context, DivergenceRequest, HitRef,
    IncompleteRequest, IncompleteResponse, ReplayStatement, RunRequest, StepKeyRequest, WireHit,
    effective_policy, result_blobs,
};
use serde_json::json;

use crate::auth::{Principal, Runner};
use crate::store::StepRow;
use crate::{ApiError, App, AppState, blocking, dash};

use axum::Json;

/// The policy this call runs under.
fn policy(app: &App, runner: &Runner, ctx: &Context) -> Result<Policy, ApiError> {
    let team = app
        .auth
        .team(&runner.team)
        .ok_or_else(|| ApiError::forbidden("unknown team"))?;
    let p = effective_policy(&ctx.policy, &team.policy.policy());
    p.validate()?;
    Ok(p)
}

/// The largest stdout or stderr a recorded result may have.
pub const MAX_OUTPUT_BYTES: u64 = 64 << 20;
/// The longest client-supplied text kept (names, reasons), in bytes.
pub const MAX_TEXT: usize = 4096;

/// `s` cut to at most `MAX_TEXT` bytes, on a character boundary.
pub fn cap(s: &str) -> String {
    let mut end = s.len().min(MAX_TEXT);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn cap_opt(s: &Option<String>) -> Option<String> {
    s.as_deref().map(cap)
}

fn check_step(ctx: &Context) -> Result<(), ApiError> {
    if ctx.step.cmd.len() > 64 << 10 || ctx.step.ignore.len() > 10_000 {
        return Err(ApiError::bad_request("step too large"));
    }
    Ok(())
}

/// A reason a runner may commit under. `DeferredAudit` names a pending
/// audit in a local ledger: the server holds none (it never defers), so a
/// commit claiming to settle one is refused rather than recorded as an
/// audit that no pending row backs.
fn check_reason(reason: &Reason) -> Result<(), ApiError> {
    if matches!(reason, Reason::DeferredAudit { .. }) {
        return Err(ApiError::bad_request(
            "deferred audits are local only: a remote runner audits inline",
        ));
    }
    Ok(())
}

/// The repository a call is about: the principal's (OIDC claims, a fixed
/// token, a delegated token's hint), never the body's.
fn repository(runner: &Runner) -> Result<&str, ApiError> {
    if runner.repository.is_empty() {
        return Err(ApiError::forbidden("this runner token names no repository"));
    }
    Ok(&runner.repository)
}

pub async fn claim(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<ClaimRequest>,
) -> Result<Json<ClaimResponse>, ApiError> {
    blocking(move || {
        let runner = App::runner_of(&principal, req.ctx.scope.as_ref())?;
        check_step(&req.ctx)?;
        let policy = policy(&app, &runner, &req.ctx)?;
        repository(&runner)?;
        let cas = app.store.cas(&runner.team)?;
        let tree = Tree::from_entries(req.tree);
        let step = req.ctx.step.step();
        let mut answer: Option<(String, Lookup, Claimed)> = None;
        let mut own: Option<(String, Lookup, Claimed)> = None;
        let mut compared: Option<(String, Lookup, Claimed)> = None;
        // A replay served from a scope the runner does not write (a pull
        // request replaying main's result): its run row goes to the
        // runner's own scope, never into the scope it read.
        let mut foreign_run: Option<RunRecord> = None;
        for scope in &runner.read {
            // A scope nobody wrote has nothing to replay; the runner's own
            // write scope is opened (and created) regardless.
            if *scope != runner.write && !app.store.has_ledger(&runner.team, runner.repo(), scope) {
                continue;
            }
            let ledger = app.store.ledger(&runner.team, runner.repo(), scope)?;
            let own_scope = *scope == runner.write;
            let trust = Trust::new(&ledger, &cas, policy)?;
            // Look up, decide and record in one transaction: two runners
            // claiming at once are serialized, so an audit slot or a
            // quarantine is seen by exactly one of them.
            let (lookup, claimed) = ledger.atomically(|l| -> Result<_, ApiError> {
                let lookup = l.find_match(&step.key, &tree)?;
                let claimed = match trust.decide(&step.key, lookup.hit.as_ref())? {
                    // The server's policy never defers (`effective_policy`);
                    // `Defer` is handled as `record_replay` decides it again.
                    Decision::Replay | Decision::Defer { .. } => {
                        let hit = lookup.hit.as_ref().expect("a replay needs a hit");
                        match trust.record_replay(&step, hit)? {
                            Replayed::Replayed(facts) => {
                                let r = &hit.1;
                                let mut run = req.run.clone();
                                run.outcome = Some("replayed".into());
                                run.manifest_id = Some(r.manifest_id);
                                run.exit_code = Some(r.exit_code);
                                run.step_key = Some(step.key);
                                // A saving only where the runner said how
                                // long the replay took (the hooks do not).
                                run.saved_ms = run.wall_ms.map(|w| r.duration_ms.saturating_sub(w));
                                if own_scope {
                                    l.record_run_facts(&run.facts())?;
                                } else {
                                    foreign_run = Some(run);
                                }
                                Claimed::Replay {
                                    facts,
                                    deferred: None,
                                }
                            }
                            // Unreachable under the server's policy; were it
                            // reached, the replay is rolled back rather than
                            // served with an audit nobody would run.
                            Replayed::Deferred(..) => {
                                return Err(ApiError::internal(
                                    "a deferred audit decided on the server",
                                ));
                            }
                            Replayed::Execute(reason) => Claimed::Execute { reason },
                        }
                    }
                    Decision::Execute(reason) => Claimed::Execute { reason },
                };
                Ok((lookup, claimed))
            })?;
            let replay = matches!(claimed, Claimed::Replay { .. });
            if replay {
                answer = Some((scope.clone(), lookup, claimed));
                break;
            }
            if *scope == runner.write {
                own = Some((scope.clone(), lookup, claimed));
            } else if lookup.hit.is_some() && compared.is_none() {
                compared = Some((scope.clone(), lookup, claimed));
            }
        }
        // Nothing replays. The execution is compared with the own scope's
        // hit if it has one, else with the first read scope's (a pull
        // request then audits what main proved, for its reason: proving,
        // an audit), and its commit still writes only the own scope.
        let (scope, lookup, claimed) = match (answer, own, compared) {
            (Some(a), ..) => a,
            (None, Some(o), _) if o.1.hit.is_some() => o,
            (None, _, Some(c)) => c,
            (None, Some(o), None) => o,
            (None, None, None) => unreachable!("the write scope is always among the read scopes"),
        };
        if let Some(run) = foreign_run {
            app.store
                .ledger(&runner.team, runner.repo(), &runner.write)?
                .record_run_facts(&run.facts())?;
        }
        let receipt = match (&claimed, &lookup.hit) {
            (Claimed::Replay { .. }, Some((_, r))) => Some(
                quod_wire::sign(
                    &app.settings.signing_key,
                    ReplayStatement {
                        protocol: quod_wire::PROTOCOL.into(),
                        team: runner.team.clone(),
                        scope: scope.clone(),
                        step_key: step.key,
                        manifest_id: r.manifest_id,
                        run_id: r.run_id.clone(),
                        result: quod_wire::result_hash(r).map_err(ApiError::internal)?,
                        issued_at: now_rfc3339(),
                        nonce: req.nonce.clone(),
                    },
                )
                .map_err(ApiError::internal)?,
            ),
            _ => None,
        };
        if let (Claimed::Replay { facts, .. }, Some((m, r))) = (&claimed, &lookup.hit) {
            note_step(
                &app,
                &runner,
                &req.ctx,
                &scope,
                &req.run.id,
                "replayed",
                Some(r),
                Some(m.entries.len() as u64),
                Some((facts.streak, facts.needed, facts.trust.state)),
                req.run.wall_ms,
            );
        }
        Ok(Json(ClaimResponse {
            candidates: lookup.candidates,
            closest: lookup.closest,
            hit: lookup.hit.map(|(manifest, result)| WireHit {
                scope,
                manifest,
                result,
            }),
            claimed,
            receipt,
        }))
    })
    .await
}

/// A hit a commit names, from a scope the runner may read.
fn resolve_hit(
    app: &App,
    runner: &Runner,
    key: &Hash,
    hit: Option<&HitRef>,
) -> Result<Option<(Manifest, StepResult)>, ApiError> {
    let Some(h) = hit else { return Ok(None) };
    repository(runner)?;
    if !runner.read.contains(&h.scope) {
        return Err(ApiError::forbidden(
            "the hit names a scope this runner cannot read",
        ));
    }
    if !app.store.has_ledger(&runner.team, runner.repo(), &h.scope) {
        return Ok(None);
    }
    let l = app.store.ledger(&runner.team, runner.repo(), &h.scope)?;
    let Some(m) = l.manifest(&h.manifest_id)? else {
        return Ok(None);
    };
    let r = l
        .results_for_run(&h.run_id)?
        .into_iter()
        .find(|r| r.step_key == *key && r.manifest_id == h.manifest_id);
    Ok(r.map(|r| (m, r)))
}

fn write_ledger(app: &App, runner: &Runner) -> Result<(Ledger, Cas), ApiError> {
    repository(runner)?;
    Ok((
        app.store
            .ledger(&runner.team, runner.repo(), &runner.write)?,
        app.store.cas(&runner.team)?,
    ))
}

pub async fn commit(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<CommitRequest>,
) -> Result<Json<CommitResponse>, ApiError> {
    blocking(move || {
        let runner = App::runner_of(&principal, req.ctx.scope.as_ref())?;
        check_step(&req.ctx)?;
        check_reason(&req.reason)?;
        let policy = policy(&app, &runner, &req.ctx)?;
        let step = req.ctx.step.step();
        let (m, r) = (&req.manifest, &req.result);
        let mid = manifest_id(m).map_err(ApiError::internal)?;
        if m.step_key != step.key
            || r.step_key != step.key
            || r.manifest_id != mid
            || r.run_id != req.run_id
            || req.run.id != req.run_id
        {
            return Err(ApiError::bad_request(
                "the manifest, result and run do not belong together",
            ));
        }
        if m.v > quod_manifest::VERSION || r.v > quod_manifest::VERSION {
            return Err(ApiError::bad_request(
                "a manifest or result from a newer schema than this server's",
            ));
        }
        let (ledger, cas) = write_ledger(&app, &runner)?;
        let mut missing = vec![];
        for h in result_blobs(r, step.ignore) {
            if !cas.has_blob(&h).map_err(ApiError::internal)? {
                missing.push(h);
            }
        }
        if !missing.is_empty() {
            return Err(ApiError::missing_blobs(missing));
        }
        // stdout and stderr are read whole to normalize and compare them:
        // refused past MAX_OUTPUT_BYTES, by size, before any is read.
        for (what, h) in [("stdout", &r.stdout), ("stderr", &r.stderr)] {
            let len = std::fs::metadata(cas.blob_path(h)).map_or(0, |m| m.len());
            if len > MAX_OUTPUT_BYTES {
                return Err(ApiError::new(
                    axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                    "too_large",
                    format!("the result's {what} is {len} bytes; at most {MAX_OUTPUT_BYTES} are recorded"),
                ));
            }
        }
        // The hit lives in a read scope's ledger, which may be another file
        // than this one; it is a recorded result and does not change.
        let hit = resolve_hit(
            &app,
            &runner,
            &step.key,
            req.hit.as_ref(),
        )?;
        let trust = Trust::new(&ledger, &cas, policy)?;
        let local = LocalLedger {
            ledger: &ledger,
            trust: &trust,
        };
        // A run recorded twice (a retried commit) is a 409, checked in the
        // same transaction that would record it.
        let v = ledger.atomically(|l| -> Result<Verdict, ApiError> {
            if !l.results_for_run(&req.run_id)?.is_empty() || l.run(&req.run_id)?.is_some() {
                return Err(ApiError::conflict("this run is already recorded"));
            }
            Ok(local.commit_execution(
                &step,
                &req.run_id,
                m,
                r,
                hit.as_ref(),
                &req.reason,
                req.run.clone(),
            )?)
        })?;
        note_step(
            &app,
            &runner,
            &req.ctx,
            &runner.write,
            &req.run_id,
            v.outcome(),
            Some(r),
            Some(m.entries.len() as u64),
            Some((v.streak, v.needed, v.trust.state)),
            Some(r.duration_ms),
        );
        note_trust(&app, &runner, &req.ctx, &v);
        let notice = note_trusted_write(&app, &runner, &req.ctx);
        Ok(Json(CommitResponse { verdict: v, notice }))
    })
    .await
}

pub async fn divergence(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<DivergenceRequest>,
) -> Result<Json<CommitResponse>, ApiError> {
    blocking(move || {
        let runner = App::runner_of(&principal, req.ctx.scope.as_ref())?;
        check_step(&req.ctx)?;
        check_reason(&req.reason)?;
        let policy = policy(&app, &runner, &req.ctx)?;
        let step = req.ctx.step.step();
        let Some(hit) = resolve_hit(&app, &runner, &step.key, Some(&req.hit))? else {
            return Err(ApiError::conflict("the hit is not in the ledger"));
        };
        let (ledger, cas) = write_ledger(&app, &runner)?;
        let trust = Trust::new(&ledger, &cas, policy)?;
        let local = LocalLedger {
            ledger: &ledger,
            trust: &trust,
        };
        let v = local.commit_divergence(
            &step,
            &req.run_id,
            &hit,
            &req.reason,
            &cap(&req.why),
            req.run.clone(),
        )?;
        note_step(
            &app,
            &runner,
            &req.ctx,
            &runner.write,
            &req.run_id,
            v.outcome(),
            None,
            None,
            Some((v.streak, v.needed, v.trust.state)),
            req.run.wall_ms,
        );
        note_trust(&app, &runner, &req.ctx, &v);
        let notice = note_trusted_write(&app, &runner, &req.ctx);
        Ok(Json(CommitResponse { verdict: v, notice }))
    })
    .await
}

pub async fn incomplete(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<IncompleteRequest>,
) -> Result<Json<IncompleteResponse>, ApiError> {
    blocking(move || {
        let runner = App::runner_of(&principal, req.ctx.scope.as_ref())?;
        let policy = policy(&app, &runner, &req.ctx)?;
        let step = req.ctx.step.step();
        let hit = resolve_hit(&app, &runner, &step.key, req.hit.as_ref())?;
        let (ledger, cas) = write_ledger(&app, &runner)?;
        let trust = Trust::new(&ledger, &cas, policy)?;
        let local = LocalLedger {
            ledger: &ledger,
            trust: &trust,
        };
        let line =
            local.commit_incomplete(&step, hit.as_ref(), &cap(&req.what), req.run.clone())?;
        Ok(Json(IncompleteResponse { line }))
    })
    .await
}

pub async fn runs(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<RunRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    blocking(move || {
        let runner = App::runner_of(&principal, req.ctx.scope.as_ref())?;
        let (ledger, _) = write_ledger(&app, &runner)?;
        ledger.record_run_facts(&req.run.facts())?;
        let outcome = req.run.outcome.clone().unwrap_or_else(|| "ran".into());
        note_step(
            &app,
            &runner,
            &req.ctx,
            &runner.write,
            &req.run.id,
            &outcome,
            None,
            None,
            None,
            req.run.wall_ms,
        );
        Ok(Json(json!({})))
    })
    .await
}

pub async fn step_keys(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<StepKeyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    blocking(move || {
        let runner = App::runner_of(&principal, req.ctx.scope.as_ref())?;
        if req.json.len() > 1 << 20 {
            return Err(ApiError::bad_request("step key JSON too large"));
        }
        // Only the JSON a key is computed from may be stored under it.
        if Hash::of(req.json.as_bytes()) != req.ctx.step.key {
            return Err(ApiError::bad_request(
                "the JSON does not hash to the step key",
            ));
        }
        let (ledger, _) = write_ledger(&app, &runner)?;
        ledger.put_step_key(&req.ctx.step.key, &req.json)?;
        Ok(Json(json!({})))
    })
    .await
}

/// The CI context of a call: the OIDC claims where there are any, the
/// client's job and step names, and the client's own context otherwise.
fn ci(runner: &Runner, ctx: &Context) -> quod_wire::CiContext {
    let own = ctx.ci.clone().unwrap_or_default();
    match &runner.claims {
        Some(c) => quod_wire::CiContext {
            job: own.job,
            job_id: own.job_id,
            job_name: own.job_name,
            step_index: own.step_index,
            step_name: own.step_name,
            runner: own.runner,
            ..c.clone()
        },
        None => own,
    }
}

/// Records a step for the dashboard and pushes its event. Best effort: the
/// ledger transaction has committed; a failure here loses a dashboard row,
/// never a result.
#[allow(clippy::too_many_arguments)]
fn note_step(
    app: &App,
    runner: &Runner,
    ctx: &Context,
    scope: &str,
    quod_run: &str,
    outcome: &str,
    result: Option<&StepResult>,
    inputs: Option<u64>,
    trust: Option<(u32, u32, TrustState)>,
    wall_ms: Option<u64>,
) {
    let ci = ci(runner, ctx);
    let row = StepRow {
        at: now_rfc3339(),
        repository: runner.repository.clone(),
        scope: scope.to_string(),
        quod_run: quod_run.to_string(),
        run_id: ci.run_id.clone(),
        run_number: ci.run_number,
        run_attempt: ci.run_attempt,
        job: cap_opt(&ci.job),
        job_id: cap_opt(&ci.job_id),
        job_name: cap_opt(&ci.job_name),
        step_index: ci.step_index,
        step_name: cap_opt(&ci.step_name),
        step_key: Some(ctx.step.key.to_string()),
        cmd: Some(ctx.step.cmd.chars().take(2000).collect()),
        outcome: outcome.to_string(),
        exit_code: result.map(|r| r.exit_code),
        wall_ms,
        saved_ms: (outcome == "replayed")
            .then(|| result.map(|r| r.duration_ms.saturating_sub(wall_ms.unwrap_or(0))))
            .flatten(),
        recorded_ms: result.map(|r| r.duration_ms),
        manifest_id: result.map(|r| r.manifest_id.to_string()),
        inputs,
        source_run: (outcome == "replayed")
            .then(|| result.map(|r| r.run_id.clone()))
            .flatten(),
        streak: trust.map(|t| t.0),
        needed: trust.map(|t| t.1),
        git_ref: cap_opt(&ci.git_ref),
        sha: cap_opt(&ci.sha),
        actor: cap_opt(&ci.actor),
        event: cap_opt(&ci.event),
        workflow: cap_opt(&ci.workflow),
        runner: cap_opt(&ci.runner),
    };
    let out = (|| -> anyhow::Result<()> {
        let a = app.store.activity(&runner.team)?;
        a.insert_step(&row)?;
        let ev = dash::step_finished_event(app, &runner.team, &row, trust.map(|t| t.2));
        a.push_event(
            "step.finished",
            Some(&row.repository),
            row.run_number,
            row.step_key.as_deref(),
            &ev,
        )?;
        Ok(())
    })();
    if let Err(e) = out {
        tracing::warn!(target: "quod", error = %format!("{e:#}"), "dashboard row not written");
    }
}

/// Records a write into the trusted scope (its ref, and GitHub's
/// `ref_protected` claim), and returns the job-log notice when that ref
/// is not known to be protected and the repository does not require it
/// (DECISIONS 2026-11-02: warn, do not require). Best effort, like the
/// dashboard rows.
fn note_trusted_write(app: &App, runner: &Runner, ctx: &Context) -> Option<String> {
    let git_ref = ci(runner, ctx).git_ref.unwrap_or_default();
    note_trusted_write_ref(app, runner, &git_ref)
}

/// `note_trusted_write` for a write whose ref is already known (a test
/// ledger save).
pub fn note_trusted_write_ref(app: &App, runner: &Runner, git_ref: &str) -> Option<String> {
    if runner.write != crate::auth::TRUSTED {
        return None;
    }
    let git_ref = git_ref.to_string();
    if let Err(e) = app
        .store
        .activity(&runner.team)
        .and_then(|a| a.record_trusted_write(&runner.repository, &git_ref, runner.ref_protected))
    {
        tracing::warn!(target: "quod", error = %format!("{e:#}"), "trusted write not recorded");
    }
    let required = app
        .auth
        .team(&runner.team)
        .is_some_and(|t| t.requires_protected_ref(&runner.repository));
    if required || runner.ref_protected == Some(true) {
        return None;
    }
    Some(writer_warning(
        &runner.repository,
        &git_ref,
        runner.ref_protected,
    ))
}

/// The writer-trust warning, for the job log and the dashboard.
pub fn writer_warning(repository: &str, git_ref: &str, protected: Option<bool>) -> String {
    let branch = git_ref.strip_prefix("refs/heads/").unwrap_or(git_ref);
    let branch = if branch.is_empty() {
        "this ref"
    } else {
        branch
    };
    format!(
        "trusted results for {repository} are written from {branch}, {}: anyone who can push to {branch} can record results that every CI job will reuse; protect {branch} or turn on require_protected_ref",
        match protected {
            Some(false) => "which is not a protected branch",
            _ => "whose protection is unknown",
        }
    )
}

/// Pushes `trust.changed` when a verdict changed what a key does.
fn note_trust(app: &App, runner: &Runner, ctx: &Context, v: &Verdict) {
    let changed = v.divergence.is_some() || (v.replayable() && v.streak == v.needed);
    if !changed {
        return;
    }
    let out = (|| -> anyhow::Result<()> {
        let ev = dash::trust_changed_event(app, &runner.team, &runner.repository, &ctx.step.key, v);
        app.store.activity(&runner.team)?.push_event(
            "trust.changed",
            Some(&runner.repository),
            None,
            Some(&ctx.step.key.to_string()),
            &ev,
        )?;
        Ok(())
    })();
    if let Err(e) = out {
        tracing::warn!(target: "quod", error = %format!("{e:#}"), "trust event not written");
    }
}
