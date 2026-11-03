//! The task cache: turbo's Remote Cache API and Nx's self-hosted cache API
//! over a session the job's OIDC token buys.
//!
//! K1  a writer's artifact reaches a pull request's job byte for byte, with its duration
//! K2  a pull request's artifact is its own: the default branch and other pull requests never read it
//! K3  jobs on someone else's input read the default branch's and never write
//! K4  another team, another repository or another environment never reads it
//! K5  the first artifact of a hash stays
//! K6  no token, a forged or expired session, or a bare OIDC token: 401; a delegated token gets no session
//! K7  turbo's status, events and query endpoints; HEAD
//! K8  Nx: PUT, GET, a second PUT is 409

mod common;

use common::*;

/// A GitHub OIDC token for `repo` (id `repo_id`) of `owner`.
fn oidc_for(owner: u64, repo: &str, repo_id: u64, event: &str, git_ref: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let claims = serde_json::json!({
        "iss": "https://token.actions.githubusercontent.com",
        "aud": "quodci",
        "sub": format!("repo:{repo}:ref:{git_ref}"),
        "iat": now - 10, "nbf": now - 10, "exp": now + 300,
        "repository": repo, "repository_id": repo_id.to_string(),
        "repository_owner": repo.split('/').next().unwrap(), "repository_owner_id": owner.to_string(),
        "ref": git_ref, "ref_protected": "false", "event_name": event,
        "workflow": "CI", "job_workflow_ref": format!("{repo}/.github/workflows/ci.yml@refs/heads/main"),
        "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "run_id": "2002", "run_number": "7", "run_attempt": "1", "actor": "octocat",
    });
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("quod-test-key".into());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(
        &std::fs::read(fixture("test-oidc-key.pem")).unwrap(),
    )
    .unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

fn oidc(event: &str, git_ref: &str) -> String {
    oidc_for(ACME_OWNER, "acme/app", 7, event, git_ref)
}

const NODE22: &str = r#"{"os":"Linux","arch":"X64","node":"v22.1.0"}"#;

/// A session for `token` in environment `env` (JSON): (status, token).
fn session_env(s: &Server, token: &str, env: &str) -> (u16, String) {
    let r = ureq::post(&format!("{}/v1/task-cache/session", s.url))
        .set("quod-protocol", "quod.v1")
        .set("content-type", "application/json")
        .set("authorization", &format!("Bearer {token}"))
        .send_string(&format!(r#"{{"env":{env}}}"#));
    match r {
        Ok(r) => {
            let v: serde_json::Value = serde_json::from_str(&r.into_string().unwrap()).unwrap();
            (200, v["token"].as_str().unwrap().to_string())
        }
        Err(ureq::Error::Status(c, r)) => (c, r.into_string().unwrap_or_default()),
        Err(e) => panic!("{e}"),
    }
}

fn session(s: &Server, token: &str) -> String {
    let (c, t) = session_env(s, token, NODE22);
    assert_eq!(c, 200, "{t}");
    t
}

/// (status, body, headers we look at)
fn call(
    s: &Server,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&[u8]>,
    headers: &[(&str, &str)],
) -> (u16, Vec<u8>, Option<String>, Option<String>) {
    let mut r = ureq::request(method, &format!("{}{path}", s.url));
    if let Some(t) = token {
        r = r.set("authorization", &format!("Bearer {t}"));
    }
    for (k, v) in headers {
        r = r.set(k, v);
    }
    let out = match body {
        Some(b) => r.send_bytes(b),
        None => r.call(),
    };
    let resp = match out {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("{e}"),
    };
    let code = resp.status();
    let dur = resp.header("x-artifact-duration").map(String::from);
    let scope = resp.header("x-quod-scope").map(String::from);
    let mut b = vec![];
    std::io::Read::read_to_end(&mut resp.into_reader(), &mut b).unwrap();
    (code, b, dur, scope)
}

fn put(s: &Server, tok: &str, hash: &str, bytes: &[u8]) -> u16 {
    call(
        s,
        "PUT",
        &format!("/v8/artifacts/{hash}?slug=quod"),
        Some(tok),
        Some(bytes),
        &[
            ("content-type", "application/octet-stream"),
            ("x-artifact-duration", "4321"),
        ],
    )
    .0
}

fn get(s: &Server, tok: &str, hash: &str) -> (u16, Vec<u8>) {
    let (c, b, _, _) = call(
        s,
        "GET",
        &format!("/v8/artifacts/{hash}?slug=quod"),
        Some(tok),
        None,
        &[],
    );
    (c, b)
}

#[test]
fn k1_a_writers_artifact_reaches_a_pull_request() {
    let s = Server::start(1);
    let main = session(&s, &oidc("push", "refs/heads/main"));
    let pr = session(&s, &oidc("pull_request", "refs/pull/5/merge"));
    assert_eq!(get(&s, &pr, "abc123").0, 404);
    let art: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(put(&s, &main, "abc123", &art), 202);
    let (c, b, dur, scope) = call(&s, "GET", "/v8/artifacts/abc123", Some(&pr), None, &[]);
    assert_eq!(c, 200);
    assert_eq!(b, art);
    assert_eq!(dur.as_deref(), Some("4321"));
    assert_eq!(scope.as_deref(), Some("trusted"));
    assert_eq!(get(&s, &main, "abc123"), (200, art));
}

#[test]
fn k2_a_pull_requests_artifact_is_its_own() {
    let s = Server::start(1);
    let main = session(&s, &oidc("push", "refs/heads/main"));
    let pr5 = session(&s, &oidc("pull_request", "refs/pull/5/merge"));
    let pr6 = session(&s, &oidc("pull_request", "refs/pull/6/merge"));
    let branch = session(&s, &oidc("push", "refs/heads/feature"));
    // A pull request writes what it computed (say, a poisoned typecheck).
    assert_eq!(put(&s, &pr5, "h1", b"poison"), 202);
    assert_eq!(get(&s, &pr5, "h1"), (200, b"poison".to_vec()));
    assert_eq!(
        get(&s, &main, "h1").0,
        404,
        "the default branch never reads a pull request's"
    );
    assert_eq!(
        get(&s, &pr6, "h1").0,
        404,
        "another pull request never reads it"
    );
    assert_eq!(
        get(&s, &branch, "h1").0,
        404,
        "another branch never reads it"
    );
    // The default branch writes its own; the pull request still reads its own first.
    assert_eq!(put(&s, &main, "h1", b"main"), 202);
    assert_eq!(get(&s, &main, "h1"), (200, b"main".to_vec()));
    assert_eq!(get(&s, &pr6, "h1"), (200, b"main".to_vec()));
    assert_eq!(get(&s, &pr5, "h1"), (200, b"poison".to_vec()));
}

#[test]
fn k3_jobs_on_someone_elses_input_read_and_never_write() {
    let s = Server::start(1);
    let main = session(&s, &oidc("push", "refs/heads/main"));
    let prt = session(&s, &oidc("pull_request_target", "refs/heads/main"));
    assert_eq!(put(&s, &prt, "h2", b"evil"), 403);
    assert_eq!(get(&s, &main, "h2").0, 404);
    assert_eq!(put(&s, &main, "h2", b"good"), 202);
    assert_eq!(get(&s, &prt, "h2"), (200, b"good".to_vec()));
    // A static runner token with a fork's trust reads trusted, never writes.
    let fork = session(&s, ACME_FORK);
    assert_eq!(put(&s, &fork, "h3", b"evil"), 403);
}

#[test]
fn k4_other_teams_repositories_and_environments_never_read_it() {
    let s = Server::start(1);
    let main = session(&s, &oidc("push", "refs/heads/main"));
    assert_eq!(put(&s, &main, "h4", b"acme"), 202);
    let beta = session(
        &s,
        &oidc_for(BETA_OWNER, "beta/app", 9, "push", "refs/heads/main"),
    );
    assert_eq!(get(&s, &beta, "h4").0, 404, "another team");
    let other_repo = session(
        &s,
        &oidc_for(ACME_OWNER, "acme/other", 8, "push", "refs/heads/main"),
    );
    assert_eq!(get(&s, &other_repo, "h4").0, 404, "another repository");
    let (c, node20) = session_env(
        &s,
        &oidc("push", "refs/heads/main"),
        r#"{"os":"Linux","arch":"X64","node":"v20.1.0"}"#,
    );
    assert_eq!(c, 200);
    assert_eq!(get(&s, &node20, "h4").0, 404, "another node");
}

#[test]
fn k5_the_first_artifact_stays() {
    let s = Server::start(1);
    let main = session(&s, &oidc("push", "refs/heads/main"));
    assert_eq!(put(&s, &main, "h5", b"first"), 202);
    assert_eq!(put(&s, &main, "h5", b"second"), 202);
    assert_eq!(get(&s, &main, "h5"), (200, b"first".to_vec()));
}

#[test]
fn k6_tokens() {
    let s = Server::start(1);
    let main = session(&s, &oidc("push", "refs/heads/main"));
    assert_eq!(put(&s, &main, "h6", b"x"), 202);
    assert_eq!(call(&s, "GET", "/v8/artifacts/h6", None, None, &[]).0, 401);
    assert_eq!(
        get(&s, &oidc("push", "refs/heads/main"), "h6").0,
        401,
        "a bare OIDC token"
    );
    assert_eq!(get(&s, ACME_RUNNER, "h6").0, 401, "a static token");
    let mut forged = main.clone();
    let n = forged.len();
    forged.replace_range(n - 1.., if main.ends_with('0') { "1" } else { "0" });
    assert_eq!(get(&s, &forged, "h6").0, 401);
    let (c, _) = session_env(&s, ACME_DELEGATE, NODE22);
    assert_eq!(c, 403, "a delegated token states its own scope");
    let (c, _) = session_env(&s, ACME_VIEWER, NODE22);
    assert_eq!(c, 403);
    assert_eq!(get(&s, &main, "../etc").0, 404);
    assert_eq!(get(&s, &main, "a%2Fb").0, 400);
}

#[test]
fn k7_turbo_status_events_query_head() {
    let s = Server::start(1);
    let main = session(&s, &oidc("push", "refs/heads/main"));
    let (c, b, _, _) = call(&s, "GET", "/v8/artifacts/status", Some(&main), None, &[]);
    assert_eq!(c, 200);
    assert!(String::from_utf8(b).unwrap().contains("enabled"));
    let ev = br#"[{"source":"REMOTE","event":"HIT","hash":"h7","duration":1}]"#;
    assert_eq!(
        call(
            &s,
            "POST",
            "/v8/artifacts/events",
            Some(&main),
            Some(ev),
            &[("content-type", "application/json")]
        )
        .0,
        200
    );
    assert_eq!(
        call(&s, "HEAD", "/v8/artifacts/h7", Some(&main), None, &[]).0,
        404
    );
    assert_eq!(put(&s, &main, "h7", b"seven"), 202);
    let (c, _, dur, _) = call(&s, "HEAD", "/v8/artifacts/h7", Some(&main), None, &[]);
    assert_eq!((c, dur.as_deref()), (200, Some("4321")));
    let (c, b, _, _) = call(
        &s,
        "POST",
        "/v8/artifacts",
        Some(&main),
        Some(br#"{"hashes":["h7","nope"]}"#),
        &[("content-type", "application/json")],
    );
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["h7"]["size"], 5);
    assert!(v["nope"].is_null());
    // A body is never read without a session, and is capped with one.
    let big = vec![b' '; 4 << 20];
    let unauth = ureq::post(&format!("{}/v8/artifacts", s.url)).send_bytes(&big);
    assert!(
        matches!(unauth, Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Transport(_))),
        "{unauth:?}"
    );
    let over = vec![b' '; 80 << 10];
    assert_eq!(call(&s, "POST", "/v8/artifacts", Some(&main), Some(&over), &[]).0, 400);
}

#[test]
fn k8_nx() {
    let s = Server::start(1);
    let main = session(&s, &oidc("push", "refs/heads/main"));
    let pr = session(&s, &oidc("pull_request", "refs/pull/5/merge"));
    assert_eq!(
        call(&s, "GET", "/nx/v1/cache/n1", Some(&main), None, &[]).0,
        404
    );
    assert_eq!(
        call(&s, "PUT", "/nx/v1/cache/n1", Some(&main), Some(b"nx"), &[]).0,
        200
    );
    assert_eq!(
        call(&s, "PUT", "/nx/v1/cache/n1", Some(&main), Some(b"nx2"), &[]).0,
        409
    );
    assert_eq!(
        call(&s, "GET", "/nx/v1/cache/n1", Some(&pr), None, &[]).1,
        b"nx".to_vec()
    );
    // turbo and Nx are separate hash spaces.
    assert_eq!(get(&s, &main, "n1").0, 404);
}
