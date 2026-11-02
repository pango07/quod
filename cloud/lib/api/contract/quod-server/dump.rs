//! How the files next to this one were recorded (not built here; it is a
//! quod-server test). Drives the server like its tests s7/s8 do (GitHub OIDC
//! runners on push to main, runs 1-3) and writes every dashboard answer to
//! $DUMP_DIR. To record again, on a checkout of the server's branch:
//!   cp cloud/lib/api/contract/quod-server/dump.rs crates/quod-server/tests/
//!   scripts/dev.sh "flock /work/tmp/heavy.lock env DUMP_DIR=/src/dump \
//!     cargo test -p quod-server --test dump"
//!   cp dump/* cloud/lib/api/contract/quod-server/   (then drop the .status files)

mod common;

use common::*;
use quod_wire::client::Auth;

const CMD: &str = "go test ./...";
const CMD2: &str = "go vet ./...";

fn files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("go.mod", "module acme/app"),
        ("main.go", "package main"),
        ("pkg/a/a.go", "package a"),
        ("pkg/a/a_test.go", "package a // tests"),
        ("pkg/b/b.go", "package b"),
    ]
}

fn token(run_number: u64, git_ref: &str, event: &str) -> String {
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
        "workflow": "CI",
        "job_workflow_ref": "acme/app/.github/workflows/ci.yml@refs/heads/main",
        "sha": "0123456789abcdef0123456789abcdef01234567",
        "run_id": format!("{}", 1000 + run_number),
        "run_number": run_number.to_string(),
        "run_attempt": "1", "actor": "octocat",
    });
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("quod-test-key".into());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(
        &std::fs::read(fixture("test-oidc-key.pem")).unwrap(),
    )
    .unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

fn runner(s: &Server, run: u64, step_name: &str, index: u32) -> Runner {
    let mut r = Runner::new(s, Auth::Token(token(run, "refs/heads/main", "push")));
    let ci = r.cfg.ci.as_mut().unwrap();
    ci.job = Some("test".into());
    ci.step_name = Some(step_name.into());
    ci.step_index = Some(index);
    ci.runner = Some("GitHub Actions 7".into());
    r
}

#[test]
fn dump() {
    let dir = std::path::PathBuf::from(std::env::var("DUMP_DIR").expect("DUMP_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let s = Server::start(2);
    let t = tree(&files());
    // Run 1: both steps run (first recorded). Run 2: both run again (proven).
    // Run 3: both replay.
    for run in 1..=3u64 {
        let a = runner(&s, run, "Test", 3);
        let _ = step(&a, CMD, &t, "ok  acme/app 0.4s\n");
        let b = runner(&s, run, "Vet", 4);
        let _ = step(&b, CMD2, &t, "");
    }
    let w = |name: &str, path: &str, tok: &str| {
        let (c, b) = s.get(path, tok);
        std::fs::write(dir.join(format!("{name}.json")), &b).unwrap();
        std::fs::write(dir.join(format!("{name}.status")), c.to_string()).unwrap();
        b
    };
    w("health", "/v1/teams/acme/health", ACME_VIEWER);
    w("overview-7d", "/v1/teams/acme/overview?range=7d", ACME_VIEWER);
    w("repos", "/v1/teams/acme/repos", ACME_VIEWER);
    w("repo", "/v1/teams/acme/repos/app", ACME_VIEWER);
    w("runs", "/v1/teams/acme/runs?limit=10", ACME_VIEWER);
    w("run-3", "/v1/teams/acme/repos/app/runs/3", ACME_VIEWER);
    w("run-1", "/v1/teams/acme/repos/app/runs/1", ACME_VIEWER);
    let steps = w("steps", "/v1/teams/acme/steps", ACME_VIEWER);
    let v: serde_json::Value = serde_json::from_str(&steps).unwrap();
    let key = v["steps"][0]["key"].as_str().unwrap().to_string();
    w("step", &format!("/v1/teams/acme/repos/app/steps/{key}"), ACME_VIEWER);
    w("search-index", "/v1/teams/acme/search-index", ACME_VIEWER);
    w("activity", "/v1/teams/acme/activity", ACME_VIEWER);
    w("runners-501", "/v1/teams/acme/runners", ACME_VIEWER);
    w("connect-501", "/v1/teams/acme/connect", ACME_VIEWER);
    w("repo-404", "/v1/teams/acme/repos/nope", ACME_VIEWER);
    w("team-404", "/v1/teams/beta/overview", ACME_VIEWER);
    w("unauthorized-401", "/v1/teams/acme/overview", "nope");
    w("runner-403", "/v1/teams/acme/overview", ACME_RUNNER);
    w("range-400", "/v1/teams/acme/overview?range=1y", ACME_VIEWER);
    let post = |name: &str, path: &str, body: serde_json::Value, put: bool| {
        let req = if put { ureq::put(&format!("{}{path}", s.url)) } else { ureq::post(&format!("{}{path}", s.url)) };
        let r = req
            .set("quod-protocol", "quod.v1")
            .set("authorization", &format!("Bearer {ACME_VIEWER}"))
            .send_json(body);
        let (c, b) = match r {
            Ok(r) => (r.status(), r.into_string().unwrap()),
            Err(ureq::Error::Status(c, r)) => (c, r.into_string().unwrap_or_default()),
            Err(e) => panic!("{e}"),
        };
        std::fs::write(dir.join(format!("{name}.json")), &b).unwrap();
        std::fs::write(dir.join(format!("{name}.status")), c.to_string()).unwrap();
    };
    post("resolve-501", "/v1/teams/resolve", serde_json::json!({"orgs": [{"orgId": "org_1", "name": "Acme"}]}), false);
    post("rerun-501", "/v1/teams/acme/repos/app/runs/3/rerun", serde_json::json!({"quod": false}), false);
    post("proofs-501", &format!("/v1/teams/acme/repos/app/steps/{key}/proofs"), serde_json::json!({"need": 3}), true);
    post("save-connect-501", "/v1/teams/acme/connect", serde_json::json!({"store": "cloud", "host": "github"}), true);
    // No protocol header.
    let e = ureq::get(&format!("{}/v1/teams/acme/health", s.url))
        .set("authorization", &format!("Bearer {ACME_VIEWER}"))
        .call();
    if let Err(ureq::Error::Status(c, r)) = e {
        std::fs::write(dir.join("protocol-400.json"), r.into_string().unwrap()).unwrap();
        std::fs::write(dir.join("protocol-400.status"), c.to_string()).unwrap();
    }
    let body = ureq::get(&format!("{}/v1/teams/acme/events", s.url))
        .set("authorization", &format!("Bearer {ACME_VIEWER}"))
        .set("quod-protocol", "quod.v1")
        .set("last-event-id", "0")
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    std::fs::write(dir.join("events.sse"), body).unwrap();
}
