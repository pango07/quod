//! The server path, acceptance style: a quod-server in-process and runners
//! with separate state dirs.
//!
//! S1  runner 2 replays the result runner 1 proved (blobs fetched, receipt verified)
//! S2  concurrency: racing commits prove once, racing claims replay once
//! S3  a fork's scope, or a non-writer job's, never poisons trusted
//! S4  team A never reads team B's keys, blobs or results
//! S5  an unreachable or untrusted server means the step runs
//! S6  zero false hits under mutation of every recorded input (A7, through the server)
//! S7  GitHub OIDC: verified against a local JWKS, mapped to a team and scope
//! S8  the dashboard's read API and event stream
//! S9  trust v2 (proofs 1, shadow audits, suspect marking) on the server equals local
//! S10 the dashboard contract's go-live fields (job and step identity, runs by id and attempt, writer trust)
//! S11 WorkOS access tokens for dashboard viewers
//! S12 runner endpoints authenticate before reading the body
//! S13 go-live limits: output size, step-key JSON, repository ids, token dates, upload sweep
//! S14 a v6 data dir (the deployed build's) migrates at start; the signing key is kept
//! S15 deferred audits stay local: a remote runner audits inline, the server holds none

mod common;

use common::*;
use quod_cas::{Entry, Hash, Kind, Tree};
use quod_ledger::LedgerStore;
use quod_trust::Reason;
use quod_wire::client::Auth;

const CMD: &str = "go test ./...";

fn files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("go.mod", "module acme/app"),
        ("main.go", "package main"),
        ("pkg/a/a.go", "package a"),
        ("pkg/a/a_test.go", "package a // tests"),
        ("pkg/b/b.go", "package b"),
    ]
}

#[test]
fn s1_a_fresh_runner_replays_what_another_proved() {
    let s = Server::start(2);
    let t = tree(&files());
    let r1 = Runner::token(&s, ACME_RUNNER);
    // Once never replays; twice proves.
    let a = step(&r1, CMD, &t, "ok  acme/app 0.4s\n");
    assert!(matches!(&a, Outcome::Executed(v) if v.streak == 1), "{a:?}");
    let b = step(&r1, CMD, &t, "ok  acme/app 0.4s\n");
    assert!(proven(&b), "{b:?}");
    // A runner with an empty state dir replays it, its blobs fetched into
    // its own store.
    let r2 = Runner::token(&s, ACME_RUNNER);
    match step(&r2, CMD, &t, "never executed") {
        Outcome::Replayed(out) => assert_eq!(out, b"ok  acme/app 0.4s\n"),
        o => panic!("runner 2 should replay: {o:?}"),
    }
    // Another tree: it executes.
    let t2 = tree(&[
        ("go.mod", "module acme/app"),
        ("main.go", "package main // changed"),
    ]);
    assert!(!claim(&r2, CMD, &t2).replayed());
}

#[test]
fn s2_racing_commits_prove_once_and_racing_claims_replay_once() {
    let s = Server::start(2);
    let t = tree(&files());
    // Eight runners commit the same result for the same inputs at once:
    // every one is recorded, the streaks chain (no lost update), the key
    // is proven, nothing diverged.
    let runners: Vec<Runner> = (0..8).map(|_| Runner::token(&s, ACME_RUNNER)).collect();
    let outs: Vec<Outcome> = std::thread::scope(|sc| {
        let hs: Vec<_> = runners
            .iter()
            .map(|r| sc.spawn(|| step(r, CMD, &t, "same\n")))
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut streaks: Vec<u32> = outs
        .iter()
        .filter_map(|o| match o {
            Outcome::Executed(v) => {
                assert!(v.divergence.is_none(), "{v:?}");
                Some(v.streak)
            }
            _ => None,
        })
        .collect();
    streaks.sort();
    // A runner that claimed after the key was proven replays instead; the
    // ones that executed saw each other's commits one at a time.
    let executed = streaks.len();
    assert_eq!(
        streaks,
        (1..=executed as u32).collect::<Vec<_>>(),
        "{outs:?}"
    );
    // Racing claims on a proven key under a 50% audit: the server decides
    // each claim in its own transaction, so exactly one of them gets the
    // replay slot and every other is the next audit, never a second skip.
    let s = Server::start(1);
    let mut r = Runner::token(&s, ACME_RUNNER);
    r.policy = policy(1, 0.5);
    assert!(proven(&step(&r, CMD, &t, "x\n")));
    let racers: Vec<Runner> = (0..8)
        .map(|_| {
            let mut x = Runner::token(&s, ACME_RUNNER);
            x.policy = policy(1, 0.5);
            x
        })
        .collect();
    let claims: Vec<Outcome> = std::thread::scope(|sc| {
        let hs: Vec<_> = racers
            .iter()
            .map(|x| sc.spawn(|| claim(x, CMD, &t)))
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let replays = claims.iter().filter(|c| c.replayed()).count();
    let audits = claims
        .iter()
        .filter(|c| matches!(c, Outcome::WouldExecute(Reason::Audit { .. })))
        .count();
    assert_eq!((replays, audits), (1, 7), "{claims:?}");
}

#[test]
fn s3_forks_and_non_writers_never_poison_trusted() {
    let s = Server::start(2);
    let t = tree(&files());
    // A fork's runner proves a fabricated result in its own scope...
    let fork = Runner::token(&s, ACME_FORK);
    step(&fork, CMD, &t, "PASS (fabricated)\n");
    assert!(proven(&step(&fork, CMD, &t, "PASS (fabricated)\n")));
    assert!(claim(&fork, CMD, &t).replayed());
    // ...and the trusted scope has never heard of it.
    let trusted = Runner::token(&s, ACME_RUNNER);
    assert!(matches!(
        claim(&trusted, CMD, &t),
        Outcome::WouldExecute(Reason::Miss)
    ));

    // A pull request's job (OIDC, not a writer) proves a fabricated result:
    // its own ref's scope only. A default-branch push never replays it.
    let pr = Runner::new(&s, Auth::Token(oidc("pull_request", "refs/pull/7/merge")));
    let t2 = tree(&[("go.mod", "module acme/app"), ("x.go", "package x")]);
    step(&pr, CMD, &t2, "PASS (from a PR)\n");
    assert!(proven(&step(&pr, CMD, &t2, "PASS (from a PR)\n")));
    assert!(claim(&pr, CMD, &t2).replayed());
    let push = Runner::new(&s, Auth::Token(oidc("push", "refs/heads/main")));
    assert!(matches!(
        claim(&push, CMD, &t2),
        Outcome::WouldExecute(Reason::Miss)
    ));
    // What the default branch proves, the pull request may replay.
    step(&push, CMD, &t2, "PASS (main)\n");
    step(&push, CMD, &t2, "PASS (main)\n");
    match claim(&pr, CMD, &t2) {
        Outcome::Replayed(out) => assert_eq!(out, b"PASS (main)\n"),
        o => panic!("{o:?}"),
    }
    // A delegated runner token (the hooks) states its scope; a fork scope
    // it states is a fork scope.
    let mut hooks_fork = Runner::token(&s, ACME_DELEGATE);
    hooks_fork.cfg.scope = Some(quod_wire::ScopeHint {
        repository: "acme/app".into(),
        trust: "fork-evil_app".into(),
        writer: false,
    });
    assert!(claim(&hooks_fork, CMD, &t).replayed());
    let mut hooks_trusted = Runner::token(&s, ACME_DELEGATE);
    hooks_trusted.cfg.scope = Some(quod_wire::ScopeHint {
        repository: "acme/app".into(),
        trust: "trusted".into(),
        writer: true,
    });
    assert!(matches!(
        claim(&hooks_trusted, CMD, &t),
        Outcome::WouldExecute(Reason::Miss)
    ));
    // Without a scope, a delegated token is refused, so the step runs.
    let hooks_none = Runner::token(&s, ACME_DELEGATE);
    assert!(matches!(
        claim(&hooks_none, CMD, &t),
        Outcome::WouldExecute(Reason::Miss)
    ));
}

/// A pull request's job that executes on inputs main proved (an audit, or
/// a key still proving there) is compared with main's result, and a
/// difference quarantines the key in the pull request's own scope only.
#[test]
fn s3b_a_pull_request_execution_is_compared_with_main() {
    let s = Server::start(2);
    let t = tree(&files());
    let push = Runner::new(&s, Auth::Token(oidc("push", "refs/heads/main")));
    step(&push, CMD, &t, "PASS\n");
    assert!(proven(&step(&push, CMD, &t, "PASS\n")));
    // Every hit audited: the PR executes instead of replaying, and gets
    // main's hit to compare with.
    let mut pr = Runner::new(&s, Auth::Token(oidc("pull_request", "refs/pull/9/merge")));
    pr.policy = policy(2, 1.0);
    match step(&pr, CMD, &t, "PASS, but different\n") {
        Outcome::Executed(v) => {
            assert!(v.divergence.is_some(), "{v:?}");
            assert_eq!(v.trust.state, quod_ledger::TrustState::Quarantined);
        }
        o => panic!("{o:?}"),
    }
    // Main is untouched: it still replays.
    match claim(&push, CMD, &t) {
        Outcome::Replayed(out) => assert_eq!(out, b"PASS\n"),
        o => panic!("{o:?}"),
    }
}

/// The server signs the claim's nonce into the receipt.
#[test]
fn s1b_a_receipt_answers_one_claim() {
    let s = Server::start(1);
    let t = tree(&files());
    let mut r = Runner::token(&s, ACME_RUNNER);
    r.policy = policy(1, 0.0);
    assert!(proven(&step(&r, CMD, &t, "x\n")));
    let req = quod_wire::ClaimRequest {
        ctx: quod_wire::Context {
            step: quod_wire::WireStep {
                key: key_of(CMD),
                cmd: CMD.into(),
                ignore: vec![],
            },
            policy: policy(1, 0.0),
            ci: None,
            scope: None,
        },
        tree: t.iter().cloned().collect(),
        run: quod_ledger::RunRecord {
            id: run_id(),
            ..Default::default()
        },
        nonce: Some("nonce-1".into()),
    };
    let resp: quod_wire::ClaimResponse = ureq::post(&format!("{}/v1/claim", s.url))
        .set("quod-protocol", "quod.v1")
        .set("authorization", &format!("Bearer {ACME_RUNNER}"))
        .send_json(serde_json::to_value(&req).unwrap())
        .unwrap()
        .into_json()
        .unwrap();
    let receipt = resp.receipt.expect("a replay is signed");
    assert_eq!(receipt.statement.nonce.as_deref(), Some("nonce-1"));
    let key = quod_wire::verifying_key_from_hex(&s.public_key()).unwrap();
    let result = resp.hit.unwrap().result;
    quod_wire::verify(&key, &receipt, &key_of(CMD), &result, "nonce-1").unwrap();
    // Played back into another claim, it does not verify.
    assert!(quod_wire::verify(&key, &receipt, &key_of(CMD), &result, "nonce-2").is_err());
}

#[test]
fn s4_one_team_never_reads_another() {
    let s = Server::start(2);
    let t = tree(&files());
    let acme = Runner::token(&s, ACME_RUNNER);
    step(&acme, CMD, &t, "acme secret output\n");
    assert!(proven(&step(&acme, CMD, &t, "acme secret output\n")));
    let stdout = Hash::of(b"acme secret output\n");

    // Beta's runner, same repository name, same key, same tree: a miss.
    let beta = Runner::token(&s, BETA_RUNNER);
    assert!(matches!(
        claim(&beta, CMD, &t),
        Outcome::WouldExecute(Reason::Miss)
    ));
    // Acme's blob is not beta's to read, nor to learn exists.
    let (c, _) = s.get(&format!("/v1/blobs/{}", stdout.hex()), BETA_RUNNER);
    assert_eq!(c, 404);
    let (c, _) = s.get(&format!("/v1/blobs/{}", stdout.hex()), ACME_RUNNER);
    assert_eq!(c, 200);
    let missing: serde_json::Value = ureq::post(&format!("{}/v1/blobs/missing", s.url))
        .set("quod-protocol", "quod.v1")
        .set("authorization", &format!("Bearer {BETA_RUNNER}"))
        .send_json(serde_json::json!({"hashes": [stdout]}))
        .unwrap()
        .into_json()
        .unwrap();
    assert_eq!(missing["missing"].as_array().unwrap().len(), 1);
    // Beta's own results stay beta's.
    step(&beta, CMD, &t, "beta output\n");
    assert!(proven(&step(&beta, CMD, &t, "beta output\n")));
    match claim(&acme, CMD, &t) {
        Outcome::Replayed(out) => assert_eq!(out, b"acme secret output\n"),
        o => panic!("{o:?}"),
    }
    // Dashboards: each viewer sees its own team, the other is a 404.
    assert_eq!(s.get("/v1/teams/acme/overview", ACME_VIEWER).0, 200);
    assert_eq!(s.get("/v1/teams/acme/overview", BETA_VIEWER).0, 404);
    assert_eq!(s.get("/v1/teams/beta/steps", ACME_VIEWER).0, 404);
    assert_eq!(s.get("/v1/teams/nosuch/health", ACME_VIEWER).0, 404);
    // A runner token cannot read a dashboard; a viewer cannot act as a runner.
    assert_eq!(s.get("/v1/teams/acme/overview", ACME_RUNNER).0, 403);
    let (c, _) = s.get(&format!("/v1/blobs/{}", stdout.hex()), ACME_VIEWER);
    assert_eq!(c, 403);
    // No token, a bad token: 401.
    assert_eq!(s.get("/v1/teams/acme/overview", "nope").0, 401);
}

#[test]
fn s5_an_unreachable_or_untrusted_server_means_the_step_runs() {
    let s = Server::start(2);
    let t = tree(&files());
    let r1 = Runner::token(&s, ACME_RUNNER);
    step(&r1, CMD, &t, "out\n");
    assert!(proven(&step(&r1, CMD, &t, "out\n")));

    // A runner that pins another server key never replays: the receipt
    // does not verify, so the step runs, and nothing is recorded.
    let other = hex::encode(
        quod_wire::signing_key_from_hex(&"77".repeat(32))
            .unwrap()
            .verifying_key()
            .to_bytes(),
    );
    let r2 = Runner::at(&s.url, Some(&other), Auth::Token(ACME_RUNNER.into()));
    let l = r2.ledger();
    let step_ = quod_trust::Step {
        key: key_of(CMD),
        cmd: CMD,
        ignore: &[],
    };
    let (lookup, claimed) = quod_trust::StepLedger::claim(
        &l,
        &step_,
        &t,
        &mut |_| quod_ledger::RunRecord::default(),
        None,
    )
    .unwrap();
    assert!(lookup.hit.is_none());
    assert_eq!(
        claimed,
        quod_trust::Claimed::Execute {
            reason: Reason::Miss
        }
    );
    assert!(!l.is_online());

    // A server that is gone: the step runs.
    let url = s.url.clone();
    drop(s);
    let r3 = Runner::at(&url, None, Auth::Token(ACME_RUNNER.into()));
    assert!(matches!(
        claim(&r3, CMD, &t),
        Outcome::WouldExecute(Reason::Miss)
    ));
    // An unknown token: refused, so the step runs.
    let s = Server::start(2);
    let r4 = Runner::token(&s, "not a token");
    let l = r4.ledger();
    let (_, claimed) = quod_trust::StepLedger::claim(
        &l,
        &step_,
        &t,
        &mut |_| quod_ledger::RunRecord::default(),
        None,
    )
    .unwrap();
    assert_eq!(
        claimed,
        quod_trust::Claimed::Execute {
            reason: Reason::Miss
        }
    );
    assert!(
        !l.is_online(),
        "a refused token takes the server out of use"
    );
}

/// Every way a tree can differ from what a proven run read, one at a time:
/// each file's content and mode, each directory's listing (a new entry),
/// each absent probe (the path appears), and a file removed. None may
/// replay; the unmutated tree must.
#[test]
fn s6_zero_false_hits_through_the_server() {
    let s = Server::start(2);
    let mut fs: Vec<(String, String)> = (0..40)
        .map(|i| {
            (
                format!("pkg/p{}/f{i}.go", i % 7),
                format!("package p // {i}"),
            )
        })
        .collect();
    fs.push(("go.mod".into(), "module acme/app".into()));
    let refs: Vec<(&str, &str)> = fs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let t = tree(&refs);
    let r1 = Runner::token(&s, ACME_RUNNER);
    step(&r1, CMD, &t, "ok\n");
    assert!(proven(&step(&r1, CMD, &t, "ok\n")));
    let r2 = Runner::token(&s, ACME_RUNNER);
    assert!(claim(&r2, CMD, &t).replayed(), "the unmutated tree replays");

    let m = manifest(key_of(CMD), &t);
    let leaves: Vec<Entry> = t.iter().filter(|e| e.k == Kind::File).cloned().collect();
    let mut mutants: Vec<(String, Tree)> = vec![];
    for e in &m.entries {
        match e.k {
            Kind::File => {
                mutants.push((
                    format!("content of {}", e.p),
                    t.with_leaf_changed(Entry::file(&e.p, Hash::of(b"mutated"), 0o644)),
                ));
                mutants.push((
                    format!("mode of {}", e.p),
                    t.with_leaf_changed(Entry::file(&e.p, e.h.unwrap(), 0o755)),
                ));
                mutants.push((
                    format!("{} removed", e.p),
                    Tree::from_leaves(leaves.iter().filter(|l| l.p != e.p).cloned()),
                ));
            }
            Kind::Dir => {
                let p = quod_cas::child_path(&e.p, "zz_new.go");
                mutants.push((
                    format!("listing of {}", e.p),
                    Tree::from_leaves(leaves.iter().cloned().chain([Entry::file(
                        p,
                        Hash::of(b"new"),
                        0o644,
                    )])),
                ));
            }
            Kind::Absent => {
                mutants.push((
                    format!("{} appeared", e.p),
                    Tree::from_leaves(leaves.iter().cloned().chain([Entry::file(
                        &e.p,
                        Hash::of(b"here now"),
                        0o644,
                    )])),
                ));
            }
            _ => {}
        }
    }
    assert!(mutants.len() > 120, "{} mutants", mutants.len());
    let mut false_hits = vec![];
    for (what, mt) in &mutants {
        if claim(&r2, CMD, mt).replayed() {
            false_hits.push(what.clone());
        }
    }
    eprintln!(
        "S6: 0 false hits expected over {} mutations; found {}",
        mutants.len(),
        false_hits.len()
    );
    assert!(false_hits.is_empty(), "false hits: {false_hits:?}");
}

/// An OIDC token for acme/app, as GitHub would mint it.
pub fn oidc(event: &str, git_ref: &str) -> String {
    oidc_with(event, git_ref, ACME_OWNER, "quodci", 300)
}

pub fn oidc_with(event: &str, git_ref: &str, owner: u64, aud: &str, ttl: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let claims = serde_json::json!({
        "iss": "https://token.actions.githubusercontent.com",
        "aud": aud,
        "sub": format!("repo:acme/app:ref:{git_ref}"),
        "iat": now - 10,
        "nbf": now - 10,
        "exp": now + ttl,
        "repository": "acme/app",
        "repository_id": "7",
        "repository_owner": "acme",
        "repository_owner_id": owner.to_string(),
        "ref": git_ref,
        "ref_protected": "true",
        "event_name": event,
        "workflow": "CI",
        "job_workflow_ref": "acme/app/.github/workflows/ci.yml@refs/heads/main",
        "sha": "0123456789abcdef0123456789abcdef01234567",
        "run_id": "1001",
        "run_number": "12",
        "run_attempt": "1",
        "actor": "octocat",
    });
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("quod-test-key".into());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(
        &std::fs::read(fixture("test-oidc-key.pem")).unwrap(),
    )
    .unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

#[test]
fn s7_github_oidc_is_verified_and_mapped() {
    let s = Server::start(1);
    let t = tree(&files());
    let mut push = Runner::new(&s, Auth::Token(oidc("push", "refs/heads/main")));
    push.policy = policy(1, 0.0);
    assert!(proven(&step(&push, CMD, &t, "ok\n")));
    // The dashboard rows carry the claims, not the client's word.
    let (c, body) = s.get("/v1/teams/acme/runs", ACME_VIEWER);
    assert_eq!(c, 200, "{body}");
    let runs: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(runs["runs"][0]["number"], 12);
    assert_eq!(runs["runs"][0]["actor"], "octocat");

    // On the dashboard a JWT is a WorkOS access token: GitHub's does not
    // verify as one (401); a runner's static token is 403.
    assert_eq!(
        s.get("/v1/teams/acme/health", &oidc("push", "refs/heads/main"))
            .0,
        401
    );
    assert_eq!(s.get("/v1/teams/acme/health", ACME_RUNNER).0, 403);
    // A runner endpoint: what GitHub OIDC tokens are refused for.
    let refused = |tok: String| match ureq::post(&format!("{}/v1/blobs/missing", s.url))
        .set("quod-protocol", "quod.v1")
        .set("authorization", &format!("Bearer {tok}"))
        .send_json(serde_json::json!({"hashes": []}))
    {
        Ok(r) => r.status(),
        Err(ureq::Error::Status(c, _)) => c,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(refused(oidc("push", "refs/heads/main")), 200);
    assert_eq!(
        refused(oidc_with(
            "push",
            "refs/heads/main",
            ACME_OWNER,
            "quodci",
            -300
        )),
        401,
        "expired"
    );
    assert_eq!(
        refused(oidc_with(
            "push",
            "refs/heads/main",
            ACME_OWNER,
            "other",
            300
        )),
        401,
        "audience"
    );
    assert_eq!(
        refused(oidc_with("push", "refs/heads/main", 999, "quodci", 300)),
        403,
        "unlinked owner"
    );
    // Tampered payload: the signature no longer verifies.
    let tok = oidc("push", "refs/heads/main");
    let mut parts: Vec<String> = tok.split('.').map(String::from).collect();
    parts[1] = parts[1].chars().rev().collect();
    assert_eq!(refused(parts.join(".")), 401, "tampered");
    // alg none.
    let none = format!(
        "{}.{}.",
        "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0",
        tok.split('.').nth(1).unwrap()
    );
    assert_eq!(refused(none), 401, "alg none");
    // Signed by a key the issuer never published (another kid).
    let mut h = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    h.kid = Some("quod-test-key".into());
    let forged = jsonwebtoken::encode(
        &h,
        &serde_json::json!({"aud": "quodci"}),
        &jsonwebtoken::EncodingKey::from_secret(b"guess"),
    )
    .unwrap();
    assert_eq!(refused(forged), 401, "HS256 with the RSA kid");
}

#[test]
fn s8_dashboard_reads_and_streams() {
    let s = Server::start(2);
    let t = tree(&files());
    let r = Runner::token(&s, ACME_RUNNER);
    step(&r, CMD, &t, "ok\n");
    step(&r, CMD, &t, "ok\n");
    let r2 = Runner::token(&s, ACME_RUNNER);
    assert!(claim(&r2, CMD, &t).replayed());

    let j = |path: &str| -> serde_json::Value {
        let (c, b) = s.get(path, ACME_VIEWER);
        assert_eq!(c, 200, "{path}: {b}");
        serde_json::from_str(&b).unwrap()
    };
    let o = j("/v1/teams/acme/overview?range=7d");
    assert_eq!(o["totals"]["stepsTotal"], 3);
    assert_eq!(o["totals"]["stepsReplayed"], 1);
    assert_eq!(o["trust"]["proven"], 1);
    assert_eq!(o["repos"][0]["slug"], "app");
    let h = j("/v1/teams/acme/health");
    assert_eq!(h["wrongReplays"], 0);
    let steps = j("/v1/teams/acme/steps");
    let key = steps["steps"][0]["key"].as_str().unwrap().to_string();
    assert_eq!(steps["steps"][0]["trust"]["kind"], "replaying");
    let st = j(&format!("/v1/teams/acme/repos/app/steps/{key}"));
    assert_eq!(st["inputs"]["filesRead"], 5);
    assert!(st["history"].as_array().unwrap().len() >= 2);
    let run = j("/v1/teams/acme/repos/app/runs/1");
    assert_eq!(run["stepsTotal"], 3);
    assert_eq!(run["jobs"][0]["steps"][2]["outcome"], "replayed");
    assert_eq!(run["jobs"][0]["steps"][2]["why"]["kind"], "replayed");
    let repo = j("/v1/teams/acme/repos/app");
    assert_eq!(repo["summary"]["fullName"], "acme/app");
    j("/v1/teams/acme/search-index");
    let act = j("/v1/teams/acme/activity");
    assert!(act["cursor"].as_str().unwrap().parse::<i64>().unwrap() >= 3);
    assert_eq!(s.get("/v1/teams/acme/repos/nope", ACME_VIEWER).0, 404);
    assert_eq!(
        s.get("/v1/teams/acme/overview?range=1y", ACME_VIEWER).0,
        400
    );
    // Not built yet: 501, said so.
    for p in ["/v1/teams/acme/runners", "/v1/teams/acme/connect"] {
        let (c, b) = s.get(p, ACME_VIEWER);
        assert_eq!(c, 501, "{p}");
        assert!(b.contains("not_implemented"));
    }
    // The stream: from the start, every event, then bye.
    let resp = ureq::get(&format!("{}/v1/teams/acme/events", s.url))
        .set("authorization", &format!("Bearer {ACME_VIEWER}"))
        .set("last-event-id", "0")
        .call()
        .unwrap();
    assert!(resp.content_type().starts_with("text/event-stream"));
    let body = resp.into_string().unwrap();
    assert!(body.contains("event: step.finished"), "{body}");
    assert!(body.contains("event: trust.changed"), "{body}");
    assert!(body.contains("id: 1\n"), "{body}");
    assert!(body.trim_end().ends_with("data: {}"), "{body}");
    // Resuming after the last id sends nothing new before bye.
    let last = act["cursor"].as_str().unwrap();
    let resp = ureq::get(&format!(
        "{}/v1/teams/acme/events?scope=repo&repo=app",
        s.url
    ))
    .set("authorization", &format!("Bearer {ACME_VIEWER}"))
    .set("last-event-id", last)
    .call()
    .unwrap()
    .into_string()
    .unwrap();
    assert!(!resp.contains("step.finished"), "{resp}");
    // Another team's stream is a 404.
    let e = ureq::get(&format!("{}/v1/teams/acme/events", s.url))
        .set("authorization", &format!("Bearer {BETA_VIEWER}"))
        .call();
    assert!(matches!(e, Err(ureq::Error::Status(404, _))));
}

#[test]
fn protocol_and_health() {
    let s = Server::start(2);
    let r = ureq::get(&format!("{}/healthz", s.url)).call().unwrap();
    let health: serde_json::Value = r.into_json().unwrap();
    assert_eq!(health["status"], "ok");
    assert_eq!(health["keys"][0]["public_key"], s.public_key());
    // The public key clients pin, never the private one.
    let keys: serde_json::Value = ureq::get(&format!("{}/v1/keys", s.url))
        .call()
        .unwrap()
        .into_json()
        .unwrap();
    assert_eq!(keys["keys"][0]["alg"], "ed25519");
    assert_eq!(keys["keys"][0]["public_key"], s.public_key());
    assert!(
        keys["keys"][0]["fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("ed25519:")
    );
    let all = keys.to_string() + &health.to_string();
    assert!(!all.contains(SIGNING_SEED), "the private key was served");
    let info: serde_json::Value = ureq::get(&format!("{}/v1/info", s.url))
        .call()
        .unwrap()
        .into_json()
        .unwrap();
    assert_eq!(info["protocol"], "quod.v1");
    assert_eq!(info["signing_key"], s.public_key());
    // A request that does not name quod.v1 is refused.
    let e = ureq::get(&format!("{}/v1/teams/acme/health", s.url))
        .set("authorization", &format!("Bearer {ACME_VIEWER}"))
        .set("quod-protocol", "quod.v2")
        .call();
    assert!(matches!(e, Err(ureq::Error::Status(400, _))));
    // A commit whose blobs were never uploaded is refused with their names.
    let t = tree(&files());
    let key = key_of(CMD);
    let m = manifest(key, &t);
    let res = quod_manifest::StepResult {
        v: quod_manifest::VERSION,
        step_key: key,
        manifest_id: quod_manifest::manifest_id(&m).unwrap(),
        run_id: "R1".into(),
        recorded_at: String::new(),
        exit_code: 0,
        stdout: Hash::of(b"never uploaded"),
        stderr: Hash::of(b""),
        outputs: vec![],
        root_outputs: vec![],
        duration_ms: 1,
        commands: vec![],
    };
    let body = quod_wire::CommitRequest {
        ctx: quod_wire::Context {
            step: quod_wire::WireStep {
                key,
                cmd: CMD.into(),
                ignore: vec![],
            },
            policy: policy(2, 0.0),
            ci: None,
            scope: None,
        },
        run_id: "R1".into(),
        manifest: m,
        result: res,
        hit: None,
        reason: Reason::Miss,
        run: quod_ledger::RunRecord {
            id: "R1".into(),
            ..Default::default()
        },
    };
    let e = ureq::post(&format!("{}/v1/commit", s.url))
        .set("quod-protocol", "quod.v1")
        .set("authorization", &format!("Bearer {ACME_RUNNER}"))
        .send_json(serde_json::to_value(&body).unwrap());
    match e {
        Err(ureq::Error::Status(409, r)) => {
            let b: serde_json::Value = r.into_json().unwrap();
            assert_eq!(b["error"], "missing_blobs");
            assert_eq!(b["missing"].as_array().unwrap().len(), 2);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn large_blobs_upload_in_resumable_chunks() {
    let s = Server::start(2);
    let r = Runner::token(&s, ACME_RUNNER);
    // Bigger than one chunk: goes up in PATCHes, the last marked complete.
    let big: Vec<u8> = (0..(quod_wire::client::CHUNK + 12345))
        .map(|i| (i % 251) as u8)
        .collect();
    let h = r.cas.put(&big).unwrap();
    let l = r.ledger();
    l.upload_blobs(&[h]).unwrap();
    let r2 = Runner::token(&s, ACME_RUNNER);
    r2.ledger().fetch_blobs(&[h]).unwrap();
    assert_eq!(std::fs::read(r2.cas.blob_path(&h)).unwrap(), big);
    // A blob whose bytes are not its name is refused.
    let e = ureq::put(&format!("{}/v1/blobs/{}", s.url, Hash::of(b"a").hex()))
        .set("quod-protocol", "quod.v1")
        .set("authorization", &format!("Bearer {ACME_RUNNER}"))
        .send_bytes(b"b");
    assert!(matches!(e, Err(ureq::Error::Status(400, _))));
}

/// Trust v2 through the server, step for step the same as a local ledger:
/// one record proves (proofs 1), the next would-be hits are shadow audits,
/// then the schedule decays and a hit replays; a later audit that diverges
/// quarantines the key, restarts its shadow and marks the replay it served
/// suspect, on the server's ledger as on the local one. ClaimReplay and
/// CommitExecution run quod-trust's transitions, so every verdict and the
/// trust row agree field for field.
#[test]
fn s9_trust_v2_on_the_server_is_trust_v2_locally() {
    use quod_trust::{LocalLedger, Trust};

    let s = Server::start_v2();
    let t = tree(&files());
    let mut remote = Runner::token(&s, ACME_RUNNER);
    // The client asks for main's default, deferred decay audits; the
    // server audits inline whatever the client asks (`effective_policy`),
    // so it matches a local ledger that does not defer.
    remote.policy = policy_v2();
    assert!(remote.policy.defer);
    let rl = remote.ledger();

    let dir = tempfile::tempdir().unwrap();
    let cas = quod_cas::Cas::open(dir.path().join("cas")).unwrap();
    let ledger = quod_ledger::Ledger::open(&dir.path().join("ledger.sqlite")).unwrap();
    let inline = quod_trust::Policy {
        defer: false,
        ..policy_v2()
    };
    let trust = Trust::new(&ledger, &cas, inline).unwrap();
    let local = LocalLedger {
        ledger: &ledger,
        trust: &trust,
    };

    // What a step's outcome says, minus run ids and times.
    fn summary(o: &Outcome) -> String {
        match o {
            Outcome::Replayed(out) => format!("replayed {:?}", String::from_utf8_lossy(out)),
            Outcome::WouldExecute(r) => format!("would execute {r:?}"),
            Outcome::Executed(v) => format!(
                "executed {:?} {} streak {}/{} shadow_left {} suspects {} {:?} eligible {} replays {} audits {} divergences {} shadow_from {} volatile {}",
                match &v.reason {
                    // When the quarantine began differs by milliseconds.
                    Reason::Quarantined {
                        why,
                        streak,
                        needed,
                        ..
                    } => Reason::Quarantined {
                        since: String::new(),
                        why: why.clone(),
                        streak: *streak,
                        needed: *needed,
                    },
                    r => r.clone(),
                },
                v.outcome(),
                v.streak,
                v.needed,
                v.shadow_left,
                v.suspects,
                v.trust.state,
                v.trust.eligible,
                v.trust.replays,
                v.trust.audits,
                v.trust.divergences,
                v.trust.shadow_from,
                v.trust.volatile,
            ),
        }
    }

    let plan = ["ok\n", "ok\n", "ok\n", "ok\n", "different\n", "ok\n"];
    let mut on_server = vec![];
    let mut locally = vec![];
    for out in plan {
        on_server.push(summary(&drive(&rl, &remote.cas, CMD, &t, out)));
        locally.push(summary(&drive(&local, &cas, CMD, &t, out)));
    }
    assert!(rl.is_online());
    assert_eq!(on_server, locally);

    let o = &on_server;
    assert!(
        o[0].starts_with("executed Miss miss streak 1/1 shadow_left 2"),
        "{o:?}"
    );
    assert!(
        o[1].starts_with("executed Audit { nth: 1, shadow: true }"),
        "{o:?}"
    );
    assert!(
        o[2].starts_with("executed Audit { nth: 2, shadow: true }")
            && o[2].contains("shadow_left 0"),
        "{o:?}"
    );
    assert_eq!(o[3], "replayed \"ok\\n\"", "{o:?}");
    // The decayed schedule audits the next hit; it diverges.
    assert!(
        o[4].starts_with("executed Audit { nth: 4, shadow: false } diverged")
            && o[4].contains("suspects 1")
            && o[4].contains("Quarantined"),
        "{o:?}"
    );
    // Quarantined: it executes to re-prove, compared with the newest
    // result (the divergent one), which differs again; nothing new is
    // suspect, and the shadow ahead is longer.
    assert!(
        o[5].starts_with("executed Quarantined")
            && o[5].contains("suspects 0")
            && o[5].contains("shadow_left 6"),
        "{o:?}"
    );

    // The replay it served is marked suspect in both ledgers.
    let server_ledger = s
        .store
        .ledger("acme", "acme/app", "trusted")
        .unwrap();
    assert_eq!(server_ledger.suspect_runs(None).unwrap().len(), 1);
    assert_eq!(ledger.suspect_runs(None).unwrap().len(), 1);
    assert_eq!(
        server_ledger.schema_version().unwrap(),
        quod_ledger::SCHEMA_VERSION
    );
}

/// The dashboard contract's cheap server changes (cloud/docs/api.md,
/// "Server changes needed" 4, 5, 8, 9, 11): GitHub's job id, job name and
/// step number; runs by run id and attempt; the configured default branch;
/// one day per day of the range; and where trusted results come from, with
/// the warning while a writer's ref is not known to be protected.
#[test]
fn s10_dashboard_contract_go_live_fields() {
    let s = Server::start(1);
    let t = tree(&files());
    let mut r = Runner::token(&s, ACME_RUNNER);
    r.policy = policy(1, 0.0);
    r.cfg.ci = Some(quod_wire::CiContext {
        repository: Some("acme/app".into()),
        run_id: Some("555".into()),
        run_number: Some(7),
        run_attempt: Some(2),
        job: Some("test".into()),
        job_id: Some("9001".into()),
        job_name: Some("test (ubuntu-24.04)".into()),
        step_index: Some(3),
        step_name: Some("go test".into()),
        git_ref: Some("refs/heads/main".into()),
        ..Default::default()
    });
    step(&r, CMD, &t, "ok\n");
    let j = |path: &str| -> serde_json::Value {
        let (c, b) = s.get(path, ACME_VIEWER);
        assert_eq!(c, 200, "{path}: {b}");
        serde_json::from_str(&b).unwrap()
    };
    let run = j("/v1/teams/acme/repos/app/runs/7");
    assert_eq!(run["runId"], 555);
    assert_eq!(run["runAttempt"], 2);
    assert!(
        run["githubUrl"]
            .as_str()
            .unwrap()
            .ends_with("/actions/runs/555/attempts/2")
    );
    assert_eq!(run["jobs"][0]["id"], "9001");
    assert_eq!(run["jobs"][0]["name"], "test (ubuntu-24.04)");
    assert_eq!(run["jobs"][0]["steps"][0]["index"], 3);
    let o = j("/v1/teams/acme/overview?range=30d");
    assert_eq!(o["days"].as_array().unwrap().len(), 30);
    assert!(o["days"][0]["day"].as_str() < o["days"][29]["day"].as_str());
    // A static runner token wrote trusted: protection unknown, so warned.
    let repo = j("/v1/teams/acme/repos/app");
    assert_eq!(repo["defaultBranch"], "trunk");
    assert_eq!(repo["requireProtectedRef"], false);
    let w = &repo["summary"]["trustedWrites"][0];
    assert_eq!(w["ref"], "refs/heads/main");
    assert!(w["protected"].is_null());
    assert!(
        repo["summary"]["writerTrustWarning"]
            .as_str()
            .unwrap()
            .contains("require_protected_ref")
    );
    assert_eq!(j("/v1/teams/acme/health")["requireProtectedRef"], false);
    // A GitHub OIDC writer on a protected main: the warning goes.
    let mut push = Runner::new(&s, Auth::Token(oidc("push", "refs/heads/main")));
    push.policy = policy(1, 0.0);
    step(
        &push,
        CMD,
        &tree(&[("go.mod", "module acme/app v2")]),
        "ok\n",
    );
    let repo = j("/v1/teams/acme/repos/app");
    assert_eq!(repo["summary"]["trustedWrites"][0]["protected"], true);
    assert!(repo["summary"]["writerTrustWarning"].is_null(), "{repo}");
    // step.finished names the job by GitHub's id and the key as hex.
    let body = ureq::get(&format!("{}/v1/teams/acme/events", s.url))
        .set("authorization", &format!("Bearer {ACME_VIEWER}"))
        .set("last-event-id", "0")
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(body.contains("\"jobId\":\"9001\""), "{body}");
    assert!(
        body.contains(&format!("\"stepKey\":\"{}\"", key_of(CMD).hex())),
        "{body}"
    );
}

/// A WorkOS access token, signed with the test key the JWKS fixture holds.
fn workos(sub: &str, org: Option<&str>, role: &str, iss: Option<&str>, ttl: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let iss = iss
        .map(String::from)
        .unwrap_or_else(|| format!("https://api.workos.com/user_management/{WORKOS_CLIENT}"));
    let claims = serde_json::json!({
        "iss": iss, "sub": sub, "org_id": org, "role": role,
        "iat": now - 10, "exp": now + ttl, "sid": "session_1",
    });
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("quod-test-key".into());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(
        &std::fs::read(fixture("test-oidc-key.pem")).unwrap(),
    )
    .unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

/// Dashboard viewers with WorkOS access tokens (cloud/docs/api.md, "Server
/// changes needed" 1): the path's team is the org id; the token's own
/// organization makes a member; another organization is asked of WorkOS
/// (with an API key) or is 404; admins only for the settings endpoints.
#[test]
fn s11_workos_viewers() {
    let s = Server::start(1);
    let get = |path: &str, tok: &str| s.get(path, tok).0;
    let member = workos("user_1", Some(ACME_ORG), "member", None, 300);
    assert_eq!(get("/v1/teams/org_acme/health", &member), 200);
    assert_eq!(get("/v1/teams/acme/health", &member), 200);
    // Another organization's team: 404, never 403.
    assert_eq!(get("/v1/teams/org_beta/health", &member), 404);
    assert_eq!(get("/v1/teams/org_nope/health", &member), 404);
    // Bad issuer, or expired: 401.
    let other_iss = workos(
        "user_1",
        Some(ACME_ORG),
        "member",
        Some("https://evil"),
        300,
    );
    assert_eq!(get("/v1/teams/org_acme/health", &other_iss), 401);
    let expired = workos("user_1", Some(ACME_ORG), "member", None, -300);
    assert_eq!(get("/v1/teams/org_acme/health", &expired), 401);
    // Static viewer tokens still work, by slug or org id.
    assert_eq!(get("/v1/teams/org_acme/health", ACME_VIEWER), 200);
    // Settings: members 403, admins reach the (not built) endpoint.
    let put = |tok: &str| match ureq::put(&format!("{}/v1/teams/org_acme/connect", s.url))
        .set("quod-protocol", "quod.v1")
        .set("authorization", &format!("Bearer {tok}"))
        .send_json(serde_json::json!({}))
    {
        Ok(r) => r.status(),
        Err(ureq::Error::Status(c, _)) => c,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(put(&member), 403);
    let admin = workos("user_2", Some(ACME_ORG), "admin", None, 300);
    assert_eq!(put(&admin), 501);

    // The membership fallback: a token for another organization, checked
    // with WorkOS's API (a local stand-in here), cached per (user, org).
    let rt = tokio::runtime::Runtime::new().unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c2 = calls.clone();
    let api = rt.block_on(async move {
        let app = axum::Router::new().route(
            "/user_management/organization_memberships",
            axum::routing::get(
                move |q: axum::extract::Query<std::collections::HashMap<String, String>>,
                      h: axum::http::HeaderMap| {
                    let c2 = c2.clone();
                    async move {
                        c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        assert_eq!(h["authorization"], "Bearer sk_test_key");
                        let member = q.get("user_id").map(String::as_str) == Some("user_3")
                            && q.get("organization_id").map(String::as_str) == Some(ACME_ORG);
                        axum::Json(if member {
                            serde_json::json!({"data": [{"user_id": "user_3", "organization_id": ACME_ORG,
                                                          "status": "active", "role": {"slug": "admin"}}]})
                        } else {
                            serde_json::json!({"data": []})
                        })
                    }
                },
            ),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        url
    });
    let s2 = Server::start_full(
        document_json(serde_json::json!({"proofs": 1}), Some(&api)),
        Some("sk_test_key"),
    );
    let elsewhere = workos("user_3", Some(BETA_ORG), "member", None, 300);
    assert_eq!(s2.get("/v1/teams/org_acme/health", &elsewhere).0, 200);
    assert_eq!(s2.get("/v1/teams/org_acme/health", &elsewhere).0, 200);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1, "cached");
    let stranger = workos("user_4", Some(BETA_ORG), "admin", None, 300);
    assert_eq!(s2.get("/v1/teams/org_acme/health", &stranger).0, 404);
    // The secret is never in an answer.
    let (_, b) = s2.get("/v1/teams/org_acme/health", &elsewhere);
    assert!(!b.contains("sk_test_key"));
}

/// An unauthenticated caller cannot make the server read a large body:
/// runner endpoints authenticate from the headers first, so a request
/// that announces 100 MB and sends none of it is answered 401 at once.
#[test]
fn s12_auth_before_the_body() {
    use std::io::{Read, Write};
    let s = Server::start(1);
    let addr = s.url.trim_start_matches("http://").to_string();
    for path in ["/v1/claim", "/v1/commit", "/v1/blobs/missing"] {
        let mut c = std::net::TcpStream::connect(&addr).unwrap();
        c.set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        write!(
            c,
            "POST {path} HTTP/1.1\r\nhost: x\r\nquod-protocol: quod.v1\r\ncontent-type: application/json\r\ncontent-length: 100000000\r\n\r\n{{"
        )
        .unwrap();
        let mut buf = [0u8; 64];
        let n = c.read(&mut buf).expect("answered before the body was sent");
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        assert!(head.starts_with("HTTP/1.1 401"), "{path}: {head}");
    }
}

/// The review's go-live limits: a result whose stdout is over 64 MiB is
/// refused at commit; step-key JSON must hash to its key; OIDC repository
/// ledgers live under GitHub's repository id; future-dated tokens are
/// refused; stale uploads are swept.
#[test]
fn s13_go_live_limits() {
    let s = Server::start(1);
    let t = tree(&files());
    let mut r = Runner::token(&s, ACME_RUNNER);
    r.policy = policy(1, 0.0);
    let post = |path: &str, tok: &str, body: serde_json::Value| -> (u16, String) {
        match ureq::post(&format!("{}{path}", s.url))
            .set("quod-protocol", "quod.v1")
            .set("authorization", &format!("Bearer {tok}"))
            .send_json(body)
        {
            Ok(r) => (r.status(), r.into_string().unwrap()),
            Err(ureq::Error::Status(c, r)) => (c, r.into_string().unwrap_or_default()),
            Err(e) => panic!("{e}"),
        }
    };

    // A stdout past 64 MiB: uploaded, then refused by size at commit.
    let key = key_of(CMD);
    let m = manifest(key, &t);
    let big = vec![b'x'; (64 << 20) + 1];
    let out = r.cas.put(&big).unwrap();
    let err = r.cas.put(b"").unwrap();
    r.ledger().upload_blobs(&[out, err]).unwrap();
    let res = quod_manifest::StepResult {
        v: quod_manifest::VERSION,
        step_key: key,
        manifest_id: quod_manifest::manifest_id(&m).unwrap(),
        run_id: "BIG1".into(),
        recorded_at: String::new(),
        exit_code: 0,
        stdout: out,
        stderr: err,
        outputs: vec![],
        root_outputs: vec![],
        duration_ms: 1,
        commands: vec![],
    };
    let ctx = quod_wire::Context {
        step: quod_wire::WireStep {
            key,
            cmd: CMD.into(),
            ignore: vec![],
        },
        policy: policy(1, 0.0),
        ci: None,
        scope: None,
    };
    let body = quod_wire::CommitRequest {
        ctx: ctx.clone(),
        run_id: "BIG1".into(),
        manifest: m,
        result: res,
        hit: None,
        reason: Reason::Miss,
        run: quod_ledger::RunRecord {
            id: "BIG1".into(),
            ..Default::default()
        },
    };
    let (c, b) = post(
        "/v1/commit",
        ACME_RUNNER,
        serde_json::to_value(&body).unwrap(),
    );
    assert_eq!(c, 413, "{b}");
    assert!(b.contains("stdout"), "{b}");

    // Step-key JSON that is not the key's: refused.
    let req = quod_wire::StepKeyRequest {
        ctx: ctx.clone(),
        json: "{\"not\":\"the key's\"}".into(),
    };
    assert_eq!(
        post(
            "/v1/step-keys",
            ACME_RUNNER,
            serde_json::to_value(&req).unwrap()
        )
        .0,
        400
    );
    let json = "{\"v\":1}".to_string();
    let mut ctx2 = ctx.clone();
    ctx2.step.key = Hash::of(json.as_bytes());
    let req = quod_wire::StepKeyRequest { ctx: ctx2, json };
    assert_eq!(
        post(
            "/v1/step-keys",
            ACME_RUNNER,
            serde_json::to_value(&req).unwrap()
        )
        .0,
        200
    );

    // A GitHub OIDC writer's ledgers are under the repository id.
    let mut push = Runner::new(&s, Auth::Token(oidc("push", "refs/heads/main")));
    push.policy = policy(1, 0.0);
    step(&push, CMD, &t, "ok\n");
    let by_id = quod_server::store::RepoKey {
        name: "acme/app",
        github_id: Some(7),
    };
    assert!(s.store.has_ledger("acme", by_id, "trusted").unwrap());
    assert!(
        s.store
            .scopes("acme")
            .unwrap()
            .iter()
            .any(|x| x.repository == "acme/app" && x.scope == "trusted")
    );
    if !cloud() {
        let scopes = s.data.path().join("data/teams/acme/scopes");
        assert!(scopes.join("github-7/trusted/ledger.sqlite").is_file());
        assert_eq!(
            std::fs::read_to_string(scopes.join("github-7/repository")).unwrap(),
            "acme/app"
        );
    }

    // Tokens dated in the future are refused.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let sign = |claims: serde_json::Value| {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some("quod-test-key".into());
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(
            &std::fs::read(fixture("test-oidc-key.pem")).unwrap(),
        )
        .unwrap();
        jsonwebtoken::encode(&header, &claims, &key).unwrap()
    };
    let iss = format!("https://api.workos.com/user_management/{WORKOS_CLIENT}");
    let ok = sign(
        serde_json::json!({"iss": iss, "sub": "u", "org_id": ACME_ORG, "iat": now, "exp": now + 3600}),
    );
    assert_eq!(s.get("/v1/teams/org_acme/health", &ok).0, 200);
    let not_yet = sign(
        serde_json::json!({"iss": iss, "sub": "u", "org_id": ACME_ORG, "iat": now, "nbf": now + 3000, "exp": now + 7200}),
    );
    assert_eq!(s.get("/v1/teams/org_acme/health", &not_yet).0, 401);
    let future = sign(
        serde_json::json!({"iss": iss, "sub": "u", "org_id": ACME_ORG, "iat": now + 3000, "exp": now + 7200}),
    );
    assert_eq!(s.get("/v1/teams/org_acme/health", &future).0, 401);

    // Staging files older than a day are swept; newer ones stay.
    let uploads = s.data.path().join("data/teams/acme/uploads");
    std::fs::create_dir_all(&uploads).unwrap();
    let old = uploads.join(format!("{}.part", Hash::of(b"old").hex()));
    let new = uploads.join(format!("{}.part", Hash::of(b"new").hex()));
    std::fs::write(&old, b"x").unwrap();
    std::fs::write(&new, b"x").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 86_400))
        .unwrap();
    let n = quod_server::blobs::sweep_uploads(
        &s.data.path().join("data"),
        quod_server::blobs::STALE_UPLOAD,
    );
    assert_eq!(n, 1);
    assert!(!old.exists() && new.exists());
}

/// The scope ledgers and `team.sqlite` of a data dir.
fn ledgers_in(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(ledgers_in(&p));
        } else if p.file_name().is_some_and(|n| n == "ledger.sqlite") {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// `Ledger::schema_sql` without opening it as a ledger (which migrates).
fn raw_schema(db: &rusqlite::Connection) -> String {
    let mut st = db
        .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type DESC, name")
        .unwrap();
    let rows: Vec<String> = st
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.iter()
        .map(|s| format!("{};\n", s.split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect()
}

/// S14: the live server's data dir moves forward at start. The deployed
/// build (exp/cloud-ledger-v2) wrote scope ledgers at quod-ledger v6 (the
/// schema in tests/fixtures/ledger_schema_v6.sql, that build's golden),
/// `team.sqlite` at 2, and the signing key on the volume. This build's boot
/// migrates every scope ledger to v8 under the instance lock, before
/// serving, keeps every row, and reuses the key file byte for byte: a
/// fresh runner pinned to the key it had before replays a result proved
/// before the upgrade. A ledger newer than the binary stops the boot and
/// the key is still untouched.
#[test]
fn s14_a_v6_data_dir_migrates_at_start_and_keeps_its_key() {
    use quod_server::boot::boot;
    use quod_server::config::Env;

    // SQLite files on the volume; the Postgres path's start-up (its own
    // migrations, the import of such a data dir) is tests/cloud.rs.
    if cloud() {
        return;
    }

    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("data");
    let env = || {
        let d = dir.to_str().unwrap().to_string();
        Env::from_env(move |k| match k {
            "QUOD_DATA_DIR" => Some(d.clone()),
            "QUOD_DATA_DIR_PERSISTENT" => Some("1".into()),
            _ => None,
        })
        .unwrap()
    };
    // First boot: the key is generated on the volume, nothing to migrate.
    let b1 = boot(&env()).unwrap();
    assert!(b1.key_generated);
    assert_eq!(b1.migrated, quod_server::store::Migrated::default());
    let key_file = dir.join("signing.ed25519");
    let key_bytes = std::fs::read(&key_file).unwrap();
    let pk = hex::encode(b1.settings.signing_key.verifying_key().to_bytes());
    let mut settings = b1.settings;
    settings.doc = document(2);
    let lock = b1.lock;
    let s = Server::on(settings, data);

    // A result proved, and a quarantine recorded, before the upgrade.
    let t = tree(&files());
    let r1 = Runner::at(&s.url, Some(&pk), Auth::Token(ACME_RUNNER.into()));
    step(&r1, CMD, &t, "ok\n");
    assert!(proven(&step(&r1, CMD, &t, "ok\n")));
    let data = s.stop();
    drop(lock);

    // Back to what the deployed build wrote: v6, the v6 golden exactly.
    let ledgers = ledgers_in(&dir.join("teams"));
    assert!(!ledgers.is_empty());
    for p in &ledgers {
        let db = rusqlite::Connection::open(p).unwrap();
        db.execute_batch("DROP TABLE audits_pending; DROP TABLE network; PRAGMA user_version = 6;")
            .unwrap();
        assert_eq!(
            raw_schema(&db),
            include_str!("fixtures/ledger_schema_v6.sql"),
            "{}",
            p.display()
        );
    }

    // This build's boot: every scope ledger migrated, the key reused.
    let b2 = boot(&env()).unwrap();
    assert!(!b2.key_generated);
    assert_eq!(
        hex::encode(b2.settings.signing_key.verifying_key().to_bytes()),
        pk
    );
    assert_eq!(std::fs::read(&key_file).unwrap(), key_bytes);
    assert_eq!(
        b2.migrated,
        quod_server::store::Migrated {
            teams: 1,
            ledgers: ledgers.len(),
            upgraded: ledgers.len(),
            failed: 0,
        }
    );
    for p in &ledgers {
        let l = quod_ledger::Ledger::open(p).unwrap();
        assert_eq!(l.schema_version().unwrap(), quod_ledger::SCHEMA_VERSION);
        assert_eq!(
            l.schema_sql().unwrap(),
            include_str!("../../../fixtures/ledger_schema.sql")
        );
    }
    let team_db = rusqlite::Connection::open(dir.join("teams/acme/team.sqlite")).unwrap();
    let v: i64 = team_db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, quod_server::store::ACTIVITY_VERSION);
    drop(team_db);

    // Served again: a fresh runner, pinned to the old key, replays the
    // result proved before the migration.
    let mut settings = b2.settings;
    settings.doc = document(2);
    let lock = b2.lock;
    let s = Server::on(settings, data);
    let r2 = Runner::at(&s.url, Some(&pk), Auth::Token(ACME_RUNNER.into()));
    match step(&r2, CMD, &t, "never executed") {
        Outcome::Replayed(out) => assert_eq!(out, b"ok\n"),
        o => panic!("the pre-migration proof should replay: {o:?}"),
    }
    let data = s.stop();
    drop(lock);

    // A third boot migrates nothing.
    let b3 = boot(&env()).unwrap();
    assert_eq!(b3.migrated.upgraded, 0);
    assert_eq!(b3.migrated.ledgers, ledgers.len());
    drop(b3);

    // A ledger from a newer build (a rollback): the boot is refused, the
    // file and the key are left as they were.
    let db = rusqlite::Connection::open(&ledgers[0]).unwrap();
    db.pragma_update(None, "user_version", 99).unwrap();
    drop(db);
    let e = boot(&env()).err().unwrap();
    assert!(e.contains("data dir migration"), "{e}");
    let db = rusqlite::Connection::open(&ledgers[0]).unwrap();
    let v: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 99);
    assert_eq!(std::fs::read(&key_file).unwrap(), key_bytes);
    // The error names the team, the scope and the file.
    assert!(e.contains("team acme") && e.contains("scope") && e.contains("ledger.sqlite"), "{e}");
    // The dry run says so too, and writes nothing.
    let plan = quod_server::store::Store::new(&dir).plan();
    assert!(
        plan.iter()
            .any(|p| p.action == quod_server::store::Plan::TooNew(99, quod_ledger::SCHEMA_VERSION))
    );

    // A corrupt ledger (not a newer one) does not stop the boot: it is
    // logged and counted, the other scopes serve, and the file is left
    // as it was for an operator to look at.
    let db = rusqlite::Connection::open(&ledgers[0]).unwrap();
    db.pragma_update(None, "user_version", quod_ledger::SCHEMA_VERSION as i64)
        .unwrap();
    drop(db);
    std::fs::write(&ledgers[0], b"not a database").unwrap();
    let b4 = boot(&env()).unwrap();
    assert_eq!(b4.migrated.failed, 1);
    assert_eq!(b4.migrated.ledgers, ledgers.len() - 1);
    assert_eq!(std::fs::read(&ledgers[0]).unwrap(), b"not a database");
    drop(b4);
    drop(data);
}

/// S15: deferred audits stay local. A client asking for main's default
/// (decay audits deferred) gets every audit inline from the server: the
/// claim says execute with the audit as its reason, the commit compares
/// and settles it, and the server's ledger holds no pending audit. The
/// same hit on a local ledger with a deferral callback defers. A commit
/// naming a deferred audit is refused and records nothing.
#[test]
fn s15_a_remote_runner_never_leaves_a_pending_audit() {
    use quod_trust::{Claimed, DeferInput, LocalLedger, Policy, StepLedger, Trust};

    // Every hit due (audit 1) and no shadow: each would-be hit is a
    // decay-phase audit, the case a local run defers.
    let s = Server::start_with(document_with(
        serde_json::json!({"proofs": 1, "shadow": 0, "audit": 1.0}),
    ));
    let t = tree(&files());
    let mut r = Runner::token(&s, ACME_RUNNER);
    r.policy = Policy {
        mode: quod_trust::Mode::Replay,
        proofs: 1,
        shadow: 0,
        audit: 1.0,
        replay_failures: false,
        defer: true,
    };
    assert!(matches!(
        step(&r, CMD, &t, "ok\n"),
        Outcome::Executed(v) if v.reason == Reason::Miss
    ));
    for nth in 1..=3 {
        match step(&r, CMD, &t, "ok\n") {
            Outcome::Executed(v) => {
                assert_eq!(v.reason, Reason::Audit { nth, shadow: false });
                assert!(v.divergence.is_none() && !v.repaid && v.superseded == 0);
            }
            o => panic!("hit {nth} must be audited inline: {o:?}"),
        }
    }
    let server_ledger = s
        .store
        .ledger("acme", "acme/app", "trusted")
        .unwrap();
    let key = key_of(CMD);
    assert_eq!(
        server_ledger
            .backlog(&key, &quod_trust::stale_cutoff())
            .unwrap(),
        quod_ledger::Backlog::default()
    );
    assert_eq!(server_ledger.pending_total().unwrap(), 0);
    assert_eq!(server_ledger.trust(&key).unwrap().unwrap().audits, 3);

    // The same policy on a local ledger that can store a deferral defers.
    let dir = tempfile::tempdir().unwrap();
    let cas = quod_cas::Cas::open(dir.path().join("cas")).unwrap();
    let ledger = quod_ledger::Ledger::open(&dir.path().join("ledger.sqlite")).unwrap();
    // Recorded inline first (`drive` stores no deferral).
    let inline = Trust::new(
        &ledger,
        &cas,
        Policy {
            defer: false,
            ..r.policy
        },
    )
    .unwrap();
    drive(
        &LocalLedger {
            ledger: &ledger,
            trust: &inline,
        },
        &cas,
        CMD,
        &t,
        "ok\n",
    );
    let trust = Trust::new(&ledger, &cas, r.policy).unwrap();
    let local = LocalLedger {
        ledger: &ledger,
        trust: &trust,
    };
    let step_ = quod_trust::Step {
        key,
        cmd: CMD,
        ignore: &[],
    };
    let mut defer = |_nth: u64| {
        Ok(DeferInput {
            tree: Hash::of(b"tree"),
            request: b"{}".to_vec(),
        })
    };
    let (_, claimed) = local
        .claim(
            &step_,
            &t,
            &mut |_| quod_ledger::RunRecord {
                id: run_id(),
                step_key: Some(key),
                ..quod_ledger::RunRecord::default()
            },
            Some(&mut defer),
        )
        .unwrap();
    assert!(
        matches!(claimed, Claimed::Replay { deferred: Some(d), .. } if d.nth == 1),
        "{claimed:?}"
    );
    // Without a way to store one, a deferring policy is refused up front.
    assert!(
        local
            .claim(&step_, &t, &mut |_| quod_ledger::RunRecord::default(), None)
            .is_err()
    );

    // A commit claiming to settle a deferred audit: 400, nothing recorded.
    let l = r.ledger();
    let rid = run_id();
    let m = manifest(key, &t);
    let mid = quod_manifest::manifest_id(&m).unwrap();
    let res = quod_manifest::StepResult {
        v: quod_manifest::VERSION,
        step_key: key,
        manifest_id: mid,
        run_id: rid.clone(),
        recorded_at: quod_ledger::now_rfc3339(),
        exit_code: 0,
        stdout: r.cas.put(b"ok\n").unwrap(),
        stderr: r.cas.put(b"").unwrap(),
        outputs: vec![],
        root_outputs: vec![],
        duration_ms: 1,
        commands: vec![],
    };
    let v = l
        .commit_execution(
            &step_,
            &rid,
            &m,
            &res,
            None,
            &Reason::DeferredAudit { nth: 4, pending: 1 },
            quod_ledger::RunRecord {
                id: rid.clone(),
                step_key: Some(key),
                ..quod_ledger::RunRecord::default()
            },
        )
        .unwrap();
    assert_eq!(v.streak, 0, "unrecorded: {v:?}");
    assert!(!l.is_online(), "the refusal takes the server out of use");
    assert!(server_ledger.results_for_run(&rid).unwrap().is_empty());
    assert_eq!(server_ledger.trust(&key).unwrap().unwrap().audits, 3);
}
