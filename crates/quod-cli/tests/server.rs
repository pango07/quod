//! `quod run` against a Quod server (QUOD_SERVER): the whole binary,
//! sandbox and observer included, with two state dirs standing in for two
//! fresh runners. Gated on QUOD_INTEGRATION=1 like the rest.
//!
//! Q1 runner 2 (an empty state dir) replays the result runner 1 proved,
//!    byte for byte, and its own ledger stays empty: the server's is used
//! Q2 an unreachable server: the step runs, exits as it would, and nothing
//!    is recorded anywhere
//! Q3 a changed input executes on runner 2 (the server matched the tree)

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
