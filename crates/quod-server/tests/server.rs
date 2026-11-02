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

mod common;

use common::*;
use quod_cas::{Entry, Hash, Kind, Tree};
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
    let (lookup, claimed) =
        quod_trust::StepLedger::claim(&l, &step_, &t, &mut |_| quod_ledger::RunRecord::default())
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
    let (_, claimed) =
        quod_trust::StepLedger::claim(&l, &step_, &t, &mut |_| quod_ledger::RunRecord::default())
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

    let refused = |tok: String| {
        let (c, _) = s.get("/v1/teams/acme/health", &tok);
        // A runner on the dashboard is 403 once authenticated; refused
        // tokens are 401 before that.
        c
    };
    assert_eq!(refused(oidc("push", "refs/heads/main")), 403);
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
    assert_eq!(r.into_string().unwrap(), "ok");
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
