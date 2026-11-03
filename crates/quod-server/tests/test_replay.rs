//! Test-level replay in Quod Cloud: the `quod vitest` ledger directory
//! saved and looked up per repository, suite, scope and commit, and the run
//! reports the dashboard reads.
//!
//! T1  a writer's ledger reaches a fresh pull-request runner byte for byte, signed
//! T2  a branch's ledger is its own: the default branch never reads it, the branch reads it first
//! T3  jobs on someone else's input, and fork scopes, read the default branch's and never save
//! T4  another team never sees a ledger
//! T5  the nearest recorded ancestor wins; else the newest of the branch, else the default branch's
//! T6  a lookup signed by another key, or for another lookup, is refused
//! T7  saves are refused with missing blobs, bad paths or a wrong size; nothing is half-saved
//! T8  run reports: the list, the detail (files that ran and why, spot-checks), stats, the live event
//! T9  incremental: a second save uploads only what changed

mod common;

use common::*;
use quod_wire::client::{Auth, ServerConfig};
use quod_wire::testreplay::*;
use quod_wire::testreplay_client::TestReplayClient;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccc";

/// A GitHub OIDC token for acme/app.
fn oidc(event: &str, git_ref: &str, sha: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let claims = serde_json::json!({
        "iss": "https://token.actions.githubusercontent.com",
        "aud": "quodci",
        "sub": format!("repo:acme/app:ref:{git_ref}"),
        "iat": now - 10, "nbf": now - 10, "exp": now + 300,
        "repository": "acme/app", "repository_id": "7",
        "repository_owner": "acme", "repository_owner_id": ACME_OWNER.to_string(),
        "ref": git_ref, "ref_protected": "false", "event_name": event,
        "workflow": "CI", "job_workflow_ref": "acme/app/.github/workflows/ci.yml@refs/heads/main",
        "sha": sha, "run_id": "2002", "run_number": "7", "run_attempt": "1", "actor": "octocat",
    });
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("quod-test-key".into());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(
        &std::fs::read(fixture("test-oidc-key.pem")).unwrap(),
    )
    .unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

fn client_with(s: &Server, auth: Auth, key: Option<&str>) -> TestReplayClient {
    TestReplayClient::new(ServerConfig {
        url: s.url.clone(),
        auth,
        server_key: key.map(|k| quod_wire::verifying_key_from_hex(k).unwrap()),
        timeout: std::time::Duration::from_secs(20),
        scope: None,
        ci: Some(quod_wire::CiContext {
            repository: Some("acme/app".into()),
            job: Some("test".into()),
            ..Default::default()
        }),
    })
}

fn client(s: &Server, auth: Auth) -> TestReplayClient {
    client_with(s, auth, Some(&s.public_key()))
}

fn job(s: &Server, event: &str, git_ref: &str, sha: &str) -> TestReplayClient {
    client(s, Auth::Token(oidc(event, git_ref, sha)))
}

/// A ledger directory with `files` (path under it, content).
fn ledger_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, b) in files {
        let f = d.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, b).unwrap();
    }
    d
}

fn sample(tag: &str) -> tempfile::TempDir {
    ledger_dir(&[
        ("ledger.json", &format!("{{\"v\":\"{tag}\"}}")),
        ("results/a.json", &format!("result a {tag}")),
        ("results/b.json", "result b"),
        ("blobs/0001", "a blob"),
        ("render/stdout", "  ✓ 2 files"),
        ("plan.json", "not sent"),
    ])
}

/// Looks up and fetches into a fresh directory: (the answer, the dir).
fn fetch(c: &TestReplayClient, ancestors: &[&str]) -> (Option<FoundLedger>, tempfile::TempDir) {
    let anc: Vec<String> = ancestors.iter().map(|s| s.to_string()).collect();
    let r = c.lookup(".", &anc).unwrap();
    let d = tempfile::tempdir().unwrap();
    if let Some(f) = &r.ledger {
        c.fetch(&f.files, d.path()).unwrap();
    }
    (r.ledger, d)
}

fn read(d: &tempfile::TempDir, p: &str) -> String {
    std::fs::read_to_string(d.path().join(p)).unwrap()
}

#[test]
fn t1_a_writer_ledger_reaches_a_fresh_pull_request_runner() {
    let s = Server::start(1);
    let main = job(&s, "push", "refs/heads/main", SHA_A);
    let src = sample("main");
    let (r, uploaded, _) = main.save(".", src.path()).unwrap();
    assert_eq!(r.scope, "trusted");
    assert_eq!(uploaded, 5, "every travelling file, plan.json left out");
    // The writer-trust warning: main is not protected here.
    assert!(r.notice.unwrap().contains("not a protected branch"));

    let pr = job(&s, "pull_request", "refs/pull/3/merge", SHA_B);
    let (found, d) = fetch(&pr, &[SHA_B, SHA_A]);
    let f = found.unwrap();
    assert_eq!(
        (f.scope.as_str(), f.sha.as_deref(), f.matched.as_str()),
        ("trusted", Some(SHA_A), "ancestor")
    );
    assert_eq!(read(&d, "ledger.json"), "{\"v\":\"main\"}");
    assert_eq!(read(&d, "results/a.json"), "result a main");
    assert_eq!(read(&d, "render/stdout"), "  ✓ 2 files");
    assert!(!d.path().join("plan.json").exists());
}

#[test]
fn t2_a_branch_ledger_is_its_own() {
    let s = Server::start(1);
    job(&s, "push", "refs/heads/main", SHA_A)
        .save(".", sample("main").path())
        .unwrap();
    let feat = job(&s, "push", "refs/heads/feature", SHA_B);
    let (r, _, _) = feat.save(".", sample("feature").path()).unwrap();
    assert!(r.scope.starts_with("ref-heads_feature-"), "{}", r.scope);
    assert!(r.notice.is_none());
    // The default branch reads only its own, whatever commits it names.
    let main2 = job(&s, "push", "refs/heads/main", SHA_C);
    let (f, d) = fetch(&main2, &[SHA_B]);
    assert_eq!(f.unwrap().scope, "trusted");
    assert_eq!(read(&d, "ledger.json"), "{\"v\":\"main\"}");
    // The branch reads its own first.
    let feat2 = job(&s, "push", "refs/heads/feature", SHA_C);
    let (f, d) = fetch(&feat2, &[SHA_C]);
    assert_eq!(f.unwrap().matched, "branch");
    assert_eq!(read(&d, "ledger.json"), "{\"v\":\"feature\"}");
    // A pull request of another ref does not read the feature branch's.
    let pr = job(&s, "pull_request", "refs/pull/9/merge", SHA_C);
    let (f, d) = fetch(&pr, &[SHA_B]);
    assert_eq!(f.unwrap().scope, "trusted");
    assert_eq!(read(&d, "ledger.json"), "{\"v\":\"main\"}");
}

#[test]
fn t3_someone_elses_input_reads_the_default_branch_and_never_saves() {
    let s = Server::start(1);
    job(&s, "push", "refs/heads/main", SHA_A)
        .save(".", sample("main").path())
        .unwrap();
    let prt = job(&s, "pull_request_target", "refs/heads/main", SHA_B);
    let err = prt.save(".", sample("evil").path()).unwrap_err();
    assert!(err.0.contains("403"), "{}", err.0);
    let (f, d) = fetch(&prt, &[]);
    assert_eq!(f.unwrap().scope, "trusted");
    assert_eq!(read(&d, "ledger.json"), "{\"v\":\"main\"}");
    assert!(prt.lookup(".", &[]).unwrap().write_scope.is_none());
    // A fork scope's token.
    let fork = client(&s, Auth::Token(ACME_FORK.into()));
    assert!(
        fork.save(".", sample("evil").path())
            .unwrap_err()
            .0
            .contains("403")
    );
    // Nothing they sent is served to anyone.
    let main = job(&s, "push", "refs/heads/main", SHA_C);
    let (_, d) = fetch(&main, &[SHA_B]);
    assert_eq!(read(&d, "ledger.json"), "{\"v\":\"main\"}");
}

#[test]
fn t4_another_team_never_sees_a_ledger() {
    let s = Server::start(1);
    job(&s, "push", "refs/heads/main", SHA_A)
        .save(".", sample("main").path())
        .unwrap();
    let beta = client(&s, Auth::Token(BETA_RUNNER.into()));
    assert!(beta.lookup(".", &[SHA_A.into()]).unwrap().ledger.is_none());
    // An unlinked owner's token is refused outright (the step suite's S7);
    // a viewer token cannot act as a runner.
    let viewer = client(&s, Auth::Token(ACME_VIEWER.into()));
    assert!(viewer.lookup(".", &[]).is_err());
}

#[test]
fn t5_the_nearest_recorded_ancestor_wins() {
    let s = Server::start(1);
    job(&s, "push", "refs/heads/main", SHA_A)
        .save(".", sample("A").path())
        .unwrap();
    job(&s, "push", "refs/heads/main", SHA_B)
        .save(".", sample("B").path())
        .unwrap();
    let c = job(&s, "push", "refs/heads/main", SHA_C);
    let (_, d) = fetch(&c, &[SHA_C, SHA_A, SHA_B]);
    assert_eq!(read(&d, "ledger.json"), "{\"v\":\"A\"}");
    // No named commit recorded: the newest.
    let (f, d) = fetch(&c, &[SHA_C]);
    assert_eq!(f.unwrap().matched, "default");
    assert_eq!(read(&d, "ledger.json"), "{\"v\":\"B\"}");
    // Another suite has nothing.
    assert!(c.lookup("packages/web", &[]).unwrap().ledger.is_none());
}

#[test]
fn t6_an_answer_signed_by_another_key_is_refused() {
    let s = Server::start(1);
    job(&s, "push", "refs/heads/main", SHA_A)
        .save(".", sample("main").path())
        .unwrap();
    let other = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&[9; 32])
            .verifying_key()
            .to_bytes(),
    );
    let c = client_with(
        &s,
        Auth::Token(oidc("push", "refs/heads/main", SHA_B)),
        Some(&other),
    );
    let e = c.lookup(".", &[]).unwrap_err();
    assert!(e.0.contains("not trusted"), "{}", e.0);
}

#[test]
fn t7_bad_saves_are_refused() {
    let s = Server::start(1);
    let c = job(&s, "push", "refs/heads/main", SHA_A);
    let post = |files: serde_json::Value| -> u16 {
        let r = ureq::post(&format!("{}/v1/test-ledgers", s.url))
            .set("quod-protocol", "quod.v1")
            .set(
                "authorization",
                &format!("Bearer {}", oidc("push", "refs/heads/main", SHA_A)),
            )
            .send_json(serde_json::json!({"suite": ".", "files": files}));
        match r {
            Ok(r) => r.status(),
            Err(ureq::Error::Status(c, _)) => c,
            Err(e) => panic!("{e}"),
        }
    };
    let h = quod_cas::Hash::of(b"never uploaded");
    assert_eq!(
        post(serde_json::json!([{"path": "ledger.json", "hash": h, "size": 14}])),
        409
    );
    assert_eq!(
        post(serde_json::json!([{"path": "../ledger.json", "hash": h, "size": 14}])),
        400
    );
    assert_eq!(
        post(serde_json::json!([{"path": "results/a.json", "hash": h, "size": 14}])),
        400
    );
    // A blob the server has, listed with another size.
    c.save(".", sample("x").path()).unwrap();
    let real = quod_cas::Hash::of(b"{\"v\":\"x\"}");
    assert_eq!(
        post(serde_json::json!([{"path": "ledger.json", "hash": real, "size": 3}])),
        400
    );
    // A bad suite.
    let r = c.lookup("../x", &[]);
    assert!(r.unwrap_err().0.contains("400"));
    // Unauthenticated.
    let r = ureq::post(&format!("{}/v1/test-ledgers/lookup", s.url))
        .set("quod-protocol", "quod.v1")
        .send_json(serde_json::json!({"suite": ".", "nonce": "n"}));
    assert!(matches!(r, Err(ureq::Error::Status(401, _))));
}

#[test]
fn t8_run_reports_reach_the_dashboard() {
    let s = Server::start(1);
    let crit = job(&s, "push", "refs/heads/main", SHA_A);
    let report = TestReport {
        tool: "vitest".into(),
        kind: "critical".into(),
        total: 184,
        ran: 13,
        replayed: 171,
        narrowed: 4,
        wall_ms: Some(21_000.0),
        select_ms: Some(40.0),
        est_plain_ms: Some(100_000.0),
        saved_ms: Some(79_000.0),
        exit: Some(0),
        ran_files: vec![RanFile {
            file: "src/a.test.ts".into(),
            why: "changed: src/a.ts".into(),
            duration_ms: Some(120.0),
            state: "pass".into(),
            ..Default::default()
        }],
        ledger: Some(LedgerUsed {
            found: true,
            sha: Some(SHA_B.into()),
            git_ref: Some("refs/heads/main".into()),
            scope: Some("trusted".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let id = crit.report("packages/backend", &report).unwrap().id;
    let shadow = TestReport {
        kind: "shadow".into(),
        audited: 2,
        audit_files: vec![
            AuditFile {
                file: "src/b.test.ts".into(),
                ok: true,
            },
            AuditFile {
                file: "src/c.test.ts".into(),
                ok: true,
            },
        ],
        ..report.clone()
    };
    job(&s, "push", "refs/heads/main", SHA_A)
        .report("packages/backend", &shadow)
        .unwrap();
    let j = |path: &str| -> serde_json::Value {
        let (c, b) = s.get(path, ACME_VIEWER);
        assert_eq!(c, 200, "{path}: {b}");
        serde_json::from_str(&b).unwrap()
    };
    let list = j("/v1/teams/acme/test-runs");
    assert_eq!(list["runs"].as_array().unwrap().len(), 2);
    let d = j(&format!("/v1/teams/acme/test-runs/{id}"));
    assert_eq!(d["suite"], "packages/backend");
    assert_eq!(d["files"]["replayed"], 171);
    assert_eq!(d["timing"]["savedMs"], 79_000.0);
    assert_eq!(
        d["sha"], SHA_A,
        "the commit is the token's, not the client's"
    );
    assert_eq!(d["branch"], "main");
    assert_eq!(d["runId"], 2002);
    assert_eq!(
        d["githubUrl"],
        "https://github.com/acme/app/actions/runs/2002/attempts/1"
    );
    assert_eq!(d["ranFiles"][0]["why"], "changed: src/a.ts");
    assert_eq!(d["audits"]["checked"], 2);
    assert_eq!(d["auditFiles"].as_array().unwrap().len(), 2);
    assert_eq!(d["shadow"]["kind"], "shadow");
    assert_eq!(d["ledger"]["sha"], SHA_B);
    let st = j("/v1/teams/acme/test-stats");
    assert_eq!(st["wrongResults"], 0);
    assert_eq!(st["audits"]["passed"], 2);
    assert_eq!(st["week"]["filesSkipped"], 171);
    assert_eq!(st["today"]["savedMs"], 79_000.0);
    assert_eq!(st["recent"].as_array().unwrap().len(), 1);
    let r = j("/v1/teams/acme/test-runs?repo=app&limit=1");
    assert_eq!(r["runs"].as_array().unwrap().len(), 1);
    // Another team's viewer sees nothing of it.
    let (c, _) = s.get(&format!("/v1/teams/acme/test-runs/{id}"), BETA_VIEWER);
    assert_eq!(c, 404);
    let (_, b) = s.get("/v1/teams/beta/test-runs", BETA_VIEWER);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&b).unwrap()["runs"],
        serde_json::json!([])
    );
    // A bad report.
    let bad = TestReport {
        kind: "other".into(),
        ..report.clone()
    };
    assert!(crit.report(".", &bad).is_err());
}

#[test]
fn t9_a_second_save_uploads_only_what_changed() {
    let s = Server::start(1);
    let c = job(&s, "push", "refs/heads/main", SHA_A);
    let d = sample("one");
    assert_eq!(c.save(".", d.path()).unwrap().1, 5);
    std::fs::write(d.path().join("results/a.json"), "result a two").unwrap();
    std::fs::write(d.path().join("ledger.json"), "{\"v\":\"two\"}").unwrap();
    assert_eq!(c.save(".", d.path()).unwrap().1, 2);
    let (_, got) = fetch(&job(&s, "push", "refs/heads/main", SHA_B), &[]);
    assert_eq!(read(&got, "results/a.json"), "result a two");
}
