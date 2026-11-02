//! `quod run` against a Quod server (QUOD_SERVER): the whole binary,
//! sandbox and observer included, with two state dirs standing in for two
//! fresh runners. Gated on QUOD_INTEGRATION=1 like the rest.
//!
//! Q1 runner 2 (an empty state dir) replays the result runner 1 proved,
//!    byte for byte, and its own ledger stays empty: the server's is used
//! Q2 an unreachable server: the step runs, exits as it would, and nothing
//!    is recorded anywhere
//! Q3 a changed input executes on runner 2 (the server matched the tree)
//! Q4 a remote runner audits inline: no deferred audit, on the server or
//!    the runner, even where a local run would defer one

mod common;
use std::time::Duration;

use common::*;

const TOKEN: &str = "cli-test-runner-token";

fn server(data: &std::path::Path) -> (String, tokio::runtime::Runtime, String) {
    let doc = quod_server::config::parse_document(
        &serde_json::json!({
            "teams": [{"slug": "acme", "policy": {"proofs": 2, "audit": 0.0}}],
            "tokens": [{"blake3": blake3::hash(TOKEN.as_bytes()).to_hex().to_string(),
                        "team": "acme", "kind": "runner", "repository": "acme/app"}],
        })
        .to_string(),
    )
    .unwrap();
    let seed = "42".repeat(32);
    let key = quod_wire::signing_key_from_hex(&seed).unwrap();
    let settings = quod_server::config::Settings {
        bind: "127.0.0.1".into(),
        port: 0,
        data_dir: data.to_path_buf(),
        signing_key: key.clone(),
        doc,
        sse_hold: Duration::from_secs(2),
        max_blob: 1 << 30,
        max_json: 256 << 20,
        workos_api_key: None,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (url, _) = rt.block_on(quod_server::spawn_local(settings)).unwrap();
    (url, rt, hex::encode(key.verifying_key().to_bytes()))
}

fn run(env: &Env, state: &std::path::Path, step: &str, envs: &[(&str, &str)]) -> Out {
    let repo = env.repo.to_str().unwrap();
    env.quod_in(
        state,
        &[
            "run",
            "--repo",
            repo,
            "--step",
            step,
            "--toolchain",
            "go",
            "--mode",
            "replay",
            "--proofs",
            "2",
            "--audit",
            "0",
        ],
        envs,
    )
}

#[test]
fn q_runners_share_one_ledger_through_the_server() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let data = env.tmp.path().join("server-data");
    let (url, _rt, key) = server(&data);
    let envs = [
        ("QUOD_SERVER", url.as_str()),
        ("QUOD_SERVER_TOKEN", TOKEN),
        ("QUOD_SERVER_KEY", key.as_str()),
    ];
    let step = "cat go.mod; echo done >&2";
    let r1 = env.tmp.path().join("runner1");
    let a = run(&env, &r1, step, &envs);
    assert!(a.executed() && a.code == 0, "{}", a.stderr);
    assert!(a.stderr.contains("trust: proof 1 of 2"), "{}", a.stderr);
    let b = run(&env, &r1, step, &envs);
    assert!(
        b.executed() && b.stderr.contains("proof 2 of 2"),
        "{}",
        b.stderr
    );

    // Q1: a fresh runner replays it.
    let r2 = env.tmp.path().join("runner2");
    let c = run(&env, &r2, step, &envs);
    assert!(c.hit(), "{}", c.stderr);
    assert_eq!(c.stdout, a.stdout);
    assert!(c.stderr.ends_with("done\n"), "{}", c.stderr);
    // Neither runner's own ledger holds a manifest: the server's does.
    for st in [&r1, &r2] {
        let n = std::process::Command::new("sqlite3")
            .arg(st.join("ledger.sqlite"))
            .arg("SELECT count(*) FROM manifests")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&n.stdout).trim(), "0");
    }

    // Q3: a changed input executes.
    env.write("go.mod", "module example.com/changed\n\ngo 1.22\n");
    let d = run(&env, &r2, step, &envs);
    assert!(d.executed() && !d.hit(), "{}", d.stderr);

    // Q2: the server is gone; the step runs and says why.
    drop(_rt);
    let r3 = env.tmp.path().join("runner3");
    let e = run(&env, &r3, step, &envs);
    assert!(e.executed() && e.code == 0, "{}", e.stderr);
    assert!(
        e.stderr.contains("the Quod server is not used"),
        "{}",
        e.stderr
    );
    println!(
        "Q1-Q3 ok: runner 2 replayed runner 1's proven result through the server; a changed input and a dead server executed"
    );
}

/// Q4: deferred audits stay local. `quod run` defers decay-phase audits by
/// default (trust v2, "Deferred audits"), but against a Quod server every
/// audit runs inline in the job: the client turns deferral off and the
/// server never decides one (`quod_wire::effective_policy`), so no pending
/// audit is left that nothing could settle (an ephemeral runner is gone
/// before its worker would run).
#[test]
fn q4_a_remote_runner_audits_inline() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let data = env.tmp.path().join("server-data");
    let (url, _rt, key) = server(&data);
    let envs = [
        ("QUOD_SERVER", url.as_str()),
        ("QUOD_SERVER_TOKEN", TOKEN),
        ("QUOD_SERVER_KEY", key.as_str()),
        ("QUOD_AUDIT_SPAWN", "0"),
    ];
    let step = "cat go.mod; echo q4 >&2";
    let state = env.tmp.path().join("runner");
    let repo = env.repo.to_str().unwrap().to_string();
    // A miss, then the second proof; then the team's shadow (2 would-be
    // hits) and then the decay, where every hit is due (audit 1): each of
    // those is an inline audit, never a deferred one.
    for n in 1..=5 {
        let o = env.quod_in(
            &state,
            &[
                "run",
                "--repo",
                &repo,
                "--step",
                step,
                "--toolchain",
                "go",
                "--mode",
                "replay",
                "--proofs",
                "2",
                "--audit",
                "1",
            ],
            &envs,
        );
        assert!(o.executed() && o.code == 0, "run {n}: {}", o.stderr);
        assert!(
            !o.stderr.contains("audit deferred"),
            "run {n}: {}",
            o.stderr
        );
        if n == 5 {
            assert!(
                o.stderr.contains("(audit: would-be hit 3 re-executed)"),
                "run 5 is the first decay-phase audit, inline: {}",
                o.stderr
            );
        }
    }
    // No pending audit anywhere: not on the server, not in the runner's
    // own ledger.
    let count = |db: &std::path::Path| {
        let n = std::process::Command::new("sqlite3")
            .arg(db)
            .arg("SELECT count(*) FROM audits_pending")
            .output()
            .unwrap();
        String::from_utf8_lossy(&n.stdout).trim().to_string()
    };
    let mut server_ledgers = 0;
    for p in walk(&data.join("teams")) {
        if p.file_name().is_some_and(|n| n == "ledger.sqlite") {
            server_ledgers += 1;
            assert_eq!(count(&p), "0", "{}", p.display());
        }
    }
    assert!(server_ledgers > 0);
    if state.join("ledger.sqlite").is_file() {
        assert_eq!(count(&state.join("ledger.sqlite")), "0");
    }
    println!(
        "Q4 ok: a remote runner's decay-phase audit ran inline; no pending audit on the server or the runner"
    );
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
