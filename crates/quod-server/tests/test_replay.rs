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
//! T10 checks count skips; record runs say why a file always runs
//! T11 stats count checks: critical runs and inline recordings; a legacy pair once
//! T12 test suites carry each record's witnesses and whether it is trusted
//! T13 the optional recording fields are kept and read back; old reports read as before

mod common;

use common::*;
use quod_wire::client::{Auth, ServerConfig};
use quod_wire::testreplay::*;
use quod_wire::testreplay_client::TestReplayClient;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccc";
const SHA_D: &str = "dddddddddddddddddddddddddddddddddddddddd";

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

/// A client and the job it runs in, as the job's own variables say.
struct J {
    c: TestReplayClient,
    me: Requester,
}

impl std::ops::Deref for J {
    type Target = TestReplayClient;
    fn deref(&self) -> &TestReplayClient {
        &self.c
    }
}

impl J {
    fn lookup(
        &self,
        suite: &str,
        ancestors: &[String],
    ) -> Result<LookupResponse, quod_wire::client::CallError> {
        self.c.lookup(suite, ancestors, &self.me)
    }
}

fn client_with(s: &Server, auth: Auth, key: Option<&str>) -> J {
    let c = TestReplayClient::new(ServerConfig {
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
    });
    J {
        c,
        me: Requester::default(),
    }
}

fn client(s: &Server, auth: Auth) -> J {
    client_with(s, auth, Some(&s.public_key()))
}

fn me(git_ref: &str) -> Requester {
    Requester {
        repository: Some("acme/app".into()),
        repository_id: Some(7),
        git_ref: Some(git_ref.into()),
    }
}

fn job(s: &Server, event: &str, git_ref: &str, sha: &str) -> J {
    let mut j = client(s, Auth::Token(oidc(event, git_ref, sha)));
    j.me = me(git_ref);
    j
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
fn fetch(c: &J, ancestors: &[&str]) -> (Option<FoundLedger>, tempfile::TempDir) {
    let anc: Vec<String> = ancestors.iter().map(|s| s.to_string()).collect();
    let r = c.lookup(".", &anc).unwrap();
    let d = tempfile::tempdir().unwrap();
    if let Some(f) = &r.ledger {
        // Every save stores a bundle: one download.
        assert!(f.bundle.is_some(), "no bundle");
        let (_, how) = c.fetch(&f.files, d.path(), f.bundle.as_ref()).unwrap();
        assert_eq!(how, "bundle");
        // File by file gives the same files.
        let d2 = tempfile::tempdir().unwrap();
        let (_, how) = c.fetch(&f.files, d2.path(), None).unwrap();
        assert_eq!(how, "pack");
        for x in &f.files {
            assert_eq!(
                std::fs::read(d.path().join(&x.path)).unwrap(),
                std::fs::read(d2.path().join(&x.path)).unwrap()
            );
        }
        // A bundle that is not what it says falls back to the pack.
        let d3 = tempfile::tempdir().unwrap();
        let wrong = BundleRef {
            hash: f.files[0].hash,
            size: 1,
        };
        let (_, how) = c.fetch(&f.files, d3.path(), Some(&wrong)).unwrap();
        assert_eq!(how, "pack");
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
    // Nor report (reports are numbers people trust).
    let rep = TestReport {
        tool: "vitest".into(),
        kind: "shadow".into(),
        ..Default::default()
    };
    assert!(prt.report(".", &rep).unwrap_err().0.contains("403"));
    // A delegated token cannot save (it states its own scope).
    let del = client(&s, Auth::Token(ACME_DELEGATE.into()));
    assert!(
        del.save(".", sample("evil").path())
            .unwrap_err()
            .0
            .contains("403")
    );
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
fn t6b_an_answer_for_another_job_is_refused() {
    // A proxy relays a main job's lookup (its nonce) under a feature
    // branch's token: the answer is the branch's ledger, signed for the
    // branch's job, and main's client refuses it.
    let s = Server::start(1);
    let feat = job(&s, "push", "refs/heads/feature", SHA_B);
    feat.save(".", sample("poison").path()).unwrap();
    let mut relayed = job(&s, "push", "refs/heads/feature", SHA_B);
    relayed.me = me("refs/heads/main");
    let e = relayed.lookup(".", &[SHA_B.into()]).unwrap_err();
    assert!(e.0.contains("not trusted"), "{}", e.0);
    // Another repository's job.
    let mut other = job(&s, "push", "refs/heads/feature", SHA_B);
    other.me.repository = Some("acme/other".into());
    assert!(other.lookup(".", &[]).is_err());
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
    assert_eq!(
        d["ranFiles"][0]["durationMs"], 120.0,
        "the dashboard's names"
    );
    assert_eq!(d["audits"]["checked"], 2);
    assert_eq!(d["auditFiles"].as_array().unwrap().len(), 2);
    assert_eq!(d["shadow"]["kind"], "shadow");
    assert_eq!(d["ledger"]["sha"], SHA_B);
    // Trust: the suite, with no recording yet, has its ran file learning.
    let su = j("/v1/teams/acme/test-suites");
    assert_eq!(su["suites"][0]["suite"], "packages/backend");
    assert!(su["suites"][0]["ledger"].is_null());
    assert_eq!(su["suites"][0]["files"][0]["file"], "src/a.test.ts");
    assert_eq!(su["suites"][0]["files"][0]["state"], "learning");
    assert_eq!(su["suites"][0]["totals"]["learning"], 1);
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

#[test]
fn t10_checks_count_skips_and_record_runs_say_why() {
    let s = Server::start(1);
    let job = job(&s, "push", "refs/heads/main", SHA_A);
    let base = TestReport {
        tool: "vitest".into(),
        kind: "critical".into(),
        total: 3,
        ran: 2,
        replayed: 1,
        ..Default::default()
    };
    // Two checks reuse a.test.ts; the record run finds b unrecordable and
    // c's recording different from the check.
    for _ in 0..2 {
        let r = TestReport {
            replayed_files: vec![ReplayedFile {
                file: "a.test.ts".into(),
                duration_ms: Some(400.0),
            }],
            ran_files: vec![
                RanFile {
                    file: "b.test.ts".into(),
                    why: "new test file (not recorded yet)".into(),
                    state: "pass".into(),
                    ..Default::default()
                },
                RanFile {
                    file: "c.test.ts".into(),
                    why: "new test file (not recorded yet)".into(),
                    state: "pass".into(),
                    ..Default::default()
                },
            ],
            ..base.clone()
        };
        job.report("apps/web", &r).unwrap();
    }
    let shadow = TestReport {
        kind: "shadow".into(),
        ran_files: vec![RanFile {
            file: "b.test.ts".into(),
            why: "new test file (not recorded yet)".into(),
            state: "pass".into(),
            unrecordable: Some("reads the per-run variable X_EVENT".into()),
            ..Default::default()
        }],
        record_differs: vec!["c.test.ts".into()],
        ..base.clone()
    };
    job.report("apps/web", &shadow).unwrap();
    // An old name of the same suite, from before the rename.
    job.report("apps_web-4f53cda1", &base).unwrap();
    let j = |path: &str| -> serde_json::Value {
        let (c, b) = s.get(path, ACME_VIEWER);
        assert_eq!(c, 200, "{path}: {b}");
        serde_json::from_str(&b).unwrap()
    };
    let su = j("/v1/teams/acme/test-suites");
    let suites = su["suites"].as_array().unwrap();
    assert_eq!(suites.len(), 1, "the renamed suite counts once: {su}");
    let files = suites[0]["files"].as_array().unwrap();
    let get = |n: &str| files.iter().find(|f| f["file"] == n).cloned().unwrap();
    assert_eq!(get("b.test.ts")["state"], "always");
    assert_eq!(
        get("b.test.ts")["reason"],
        "reads the per-run variable X_EVENT"
    );
    assert_eq!(get("c.test.ts")["recordDiffers"], true);
    assert_eq!(suites[0]["totals"]["recordDiffers"], 1);
    // The check's "why" uses what the record run found.
    let runs = j("/v1/teams/acme/test-runs");
    let crit = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "critical" && r["suite"] == "apps/web")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let d = j(&format!("/v1/teams/acme/test-runs/{crit}"));
    let why_b = d["ranFiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["file"] == "b.test.ts")
        .unwrap()["why"]
        .clone();
    assert_eq!(why_b, "always runs: reads the per-run variable X_EVENT");
}

/// A dashboard read as acme's viewer.
fn view(s: &Server, path: &str) -> serde_json::Value {
    let (c, b) = s.get(path, ACME_VIEWER);
    assert_eq!(c, 200, "{path}: {b}");
    serde_json::from_str(&b).unwrap()
}

/// A run report: `kind`, `total` files of which `replayed` were reused,
/// `saved_ms` of recorded time, `inline` as the client says (None: an
/// older client).
fn run(kind: &str, total: u64, replayed: u64, saved_ms: f64, inline: Option<bool>) -> TestReport {
    TestReport {
        tool: "vitest".into(),
        kind: kind.into(),
        total,
        ran: total - replayed,
        replayed,
        saved_ms: Some(saved_ms),
        inline,
        ..Default::default()
    }
}

/// Posts each (commit, report) for suite "." and returns /test-stats.
fn stats_after(reports: &[(&str, TestReport)]) -> serde_json::Value {
    let s = Server::start(1);
    for (sha, r) in reports {
        job(&s, "push", "refs/heads/main", sha)
            .report(".", r)
            .unwrap();
    }
    view(&s, "/v1/teams/acme/test-stats")
}

#[test]
fn t11_stats_count_inline_recordings_and_a_legacy_pair_once() {
    // Inline only: the developer's job recorded (an older client: no
    // `inline`, and no critical run of its commit). It is a check.
    let st = stats_after(&[(SHA_B, run("shadow", 8, 5, 200.0, None))]);
    assert_eq!(st["today"]["runs"], 1, "{st}");
    assert_eq!(st["week"]["runs"], 1);
    assert_eq!(st["week"]["filesTotal"], 8);
    assert_eq!(st["week"]["filesSkipped"], 5);
    assert_eq!(st["today"]["savedMs"], 200.0);
    assert_eq!(st["recent"].as_array().unwrap().len(), 1);
    assert_eq!(st["recent"][0]["kind"], "shadow");

    // A legacy pair: a check and its background recording, one commit.
    // One check; the recording's files and time are not counted again.
    let st = stats_after(&[
        (SHA_A, run("critical", 20, 10, 1000.0, None)),
        (SHA_A, run("shadow", 20, 10, 1000.0, None)),
    ]);
    assert_eq!(st["today"]["runs"], 1, "{st}");
    assert_eq!(st["week"]["runs"], 1);
    assert_eq!(st["week"]["filesTotal"], 20);
    assert_eq!(st["week"]["filesSkipped"], 10);
    assert_eq!(st["week"]["savedMs"], 1000.0);
    let recent = st["recent"].as_array().unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0]["kind"], "critical");

    // Mixed: a legacy pair (A), an older client's inline recording (B), a
    // newer client's (C, `inline: true`), and a background recording that
    // says so (D, `inline: false`, its check's report lost): three checks.
    let st = stats_after(&[
        (SHA_A, run("critical", 20, 10, 1000.0, None)),
        (SHA_A, run("shadow", 20, 10, 1000.0, None)),
        (SHA_B, run("shadow", 8, 5, 200.0, None)),
        (SHA_C, run("shadow", 4, 3, 30.0, Some(true))),
        (SHA_D, run("shadow", 100, 100, 99_999.0, Some(false))),
    ]);
    assert_eq!(st["today"]["runs"], 3, "{st}");
    assert_eq!(st["week"]["runs"], 3);
    assert_eq!(st["week"]["filesTotal"], 32);
    assert_eq!(st["week"]["filesSkipped"], 18);
    assert_eq!(st["week"]["savedMs"], 1230.0);
    let recent = st["recent"].as_array().unwrap();
    assert_eq!(recent.len(), 3);
    let shas: Vec<&str> = recent.iter().map(|r| r["sha"].as_str().unwrap()).collect();
    assert_eq!(shas, [SHA_C, SHA_B, SHA_A], "newest first, A's check only");
    assert_eq!(recent[2]["kind"], "critical");
    assert_eq!(recent[0]["inline"], true);
    assert!(
        recent[1]["inline"].is_null(),
        "an older client's: {}",
        recent[1]
    );
}

#[test]
fn t12_test_suites_carry_each_records_witnesses() {
    let s = Server::start(1);
    let j = job(&s, "push", "refs/heads/main", SHA_A);
    let rec = |file: &str, w: serde_json::Value, quarantined: bool| {
        let mut r = serde_json::json!({
            "file": file, "project": "", "state": "pass", "duration": 10.0,
            "quarantined": quarantined, "runs": 1, "replays": 0,
        });
        if !w.is_null() {
            r["w"] = w;
        }
        r
    };
    let w =
        |n: u32, d: u32| serde_json::json!({"n": n, "key": "k", "tests": "t", "disagreements": d});
    // The default policy: two witnesses (three after a disagreement).
    let ledger = serde_json::json!({
        "version": 1,
        "witnesses_needed": 2,
        "files": {
            "a": rec("a.test.ts", w(2, 0), false),
            "b": rec("b.test.ts", w(1, 0), false),
            "c": rec("c.test.ts", w(2, 1), false),
            "d": rec("d.test.ts", serde_json::Value::Null, false),
            "e": rec("e.test.ts", w(1, 1), true),
        },
    });
    let d = ledger_dir(&[("ledger.json", &ledger.to_string())]);
    j.save(".", d.path()).unwrap();
    // A ledger that does not say (an older client): 2. One that trusts at
    // once (QUOD_VITEST_WITNESSES=1): every record, with no count kept.
    let old = serde_json::json!({"files": {"x": rec("x.test.ts", w(1, 0), false)}});
    let d = ledger_dir(&[("ledger.json", &old.to_string())]);
    j.save("old", d.path()).unwrap();
    let one = serde_json::json!({"witnesses_needed": 1, "files": {"y": rec("y.test.ts", w(0, 0), false)}});
    let d = ledger_dir(&[("ledger.json", &one.to_string())]);
    j.save("one", d.path()).unwrap();

    let su = view(&s, "/v1/teams/acme/test-suites");
    let suite = |name: &str| {
        su["suites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["suite"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no suite {name}: {su}"))
    };
    let file = |suite: &serde_json::Value, f: &str| {
        suite["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["file"] == f)
            .cloned()
            .unwrap()
    };
    let root = suite(".");
    let got = |f: &str| {
        let x = file(&root, f);
        (
            x["witnesses"].clone(),
            x["witnessesNeeded"].clone(),
            x["trusted"].clone(),
        )
    };
    use serde_json::json;
    assert_eq!(got("a.test.ts"), (json!(2), json!(2), json!(true)));
    assert_eq!(got("b.test.ts"), (json!(1), json!(2), json!(false)));
    assert_eq!(got("c.test.ts"), (json!(2), json!(3), json!(false)));
    assert_eq!(
        got("d.test.ts"),
        (json!(null), json!(null), json!(null)),
        "from before the witness policy"
    );
    assert_eq!(got("e.test.ts").2, json!(false), "quarantined");
    // The states are as before; the witness fields are only added.
    assert_eq!(file(&root, "b.test.ts")["state"], "ready");
    assert_eq!(file(&root, "d.test.ts")["state"], "ready");
    assert_eq!(file(&root, "e.test.ts")["state"], "always");
    assert_eq!(root["totals"]["trusted"], 1, "{root}");
    assert_eq!(
        root["totals"]["awaitingWitness"], 2,
        "b and c, not quarantined e"
    );
    assert_eq!(root["totals"]["ready"], 4);
    let x = file(&suite("old"), "x.test.ts");
    assert_eq!(
        (x["witnessesNeeded"].clone(), x["trusted"].clone()),
        (json!(2), json!(false))
    );
    let y = file(&suite("one"), "y.test.ts");
    assert_eq!(
        (y["witnessesNeeded"].clone(), y["trusted"].clone()),
        (json!(1), json!(true))
    );
}

#[test]
fn t13_recording_fields_are_kept_and_old_reports_read_as_before() {
    let s = Server::start(1);
    let j = job(&s, "push", "refs/heads/main", SHA_A);
    // A recording of a.test.ts, so the suite page lists its skips.
    let ledger = serde_json::json!({"witnesses_needed": 2, "files": {"a": {
        "file": "a.test.ts", "project": "", "state": "pass", "duration": 400.0,
        "quarantined": false, "runs": 2, "replays": 0, "w": {"n": 2, "disagreements": 0},
    }}});
    let d = ledger_dir(&[("ledger.json", &ledger.to_string())]);
    j.save(".", d.path()).unwrap();
    let reused = vec![ReplayedFile {
        file: "a.test.ts".into(),
        duration_ms: Some(400.0),
    }];
    // An older client's recording (no new fields) of another commit.
    let old = TestReport {
        replayed_files: reused.clone(),
        ..run("shadow", 3, 1, 400.0, None)
    };
    let old_id = job(&s, "push", "refs/heads/main", SHA_B)
        .report(".", &old)
        .unwrap()
        .id;
    // A newer client's inline recording.
    let new = TestReport {
        replayed_files: reused,
        record_ms: Some(812.5),
        witness: Some(WitnessCounts {
            needed: 2,
            promoted: 4,
            awaiting: 7,
            disagreed: 1,
        }),
        ..run("shadow", 3, 1, 400.0, Some(true))
    };
    let new_id = j.report(".", &new).unwrap().id;

    let d = view(&s, &format!("/v1/teams/acme/test-runs/{new_id}"));
    assert_eq!(d["inline"], true, "{d}");
    assert_eq!(d["recordMs"], 812.5);
    assert_eq!(
        d["witness"],
        serde_json::json!({"needed": 2, "promoted": 4, "awaiting": 7, "disagreed": 1})
    );
    let d = view(&s, &format!("/v1/teams/acme/test-runs/{old_id}"));
    for k in ["inline", "recordMs", "witness"] {
        assert!(d[k].is_null(), "{k}: {d}");
    }
    assert_eq!(d["files"]["replayed"], 1, "the rest reads as before");
    let list = view(&s, "/v1/teams/acme/test-runs");
    let row = |id: i64| {
        list["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id.to_string())
            .cloned()
            .unwrap()
    };
    assert_eq!(row(new_id)["recordMs"], 812.5);
    assert!(row(old_id)["inline"].is_null());
    let h = view(&s, "/v1/teams/acme/test-history");
    let hrow = |id: i64| {
        h["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id.to_string())
            .cloned()
            .unwrap()
    };
    assert_eq!(hrow(new_id)["inline"], true);
    assert_eq!(hrow(new_id)["recordMs"], 812.5);
    assert!(hrow(old_id)["recordMs"].is_null());
    // Per-file skips: the inline recording's reuse counts as a check's; the
    // older client's shadow (no `inline`) does not.
    let su = view(&s, "/v1/teams/acme/test-suites");
    let a = su["suites"][0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["file"] == "a.test.ts")
        .cloned()
        .unwrap();
    assert_eq!(a["skipped"], 1, "{a}");
    assert_eq!(a["savedMs"], 400.0);
    assert_eq!(a["state"], "skipping");
    assert_eq!(a["trusted"], true);
    // Counts out of range are refused.
    let bad = TestReport {
        witness: Some(WitnessCounts {
            needed: 5000,
            ..Default::default()
        }),
        ..new.clone()
    };
    assert!(j.report(".", &bad).is_err());
}
