//! Acceptance of the trust layer and the robustness fixes, end to end
//! through the binary. Gated on QUOD_INTEGRATION=1 like the rest.
//!
//! T1 a nondeterministic step never becomes replayable; T2 a deterministic
//! one replays after two identical executions, with the trust facts in its
//! receipt; T3 observe mode (the default) never replays and reports what it
//! would have saved; T4 an audit that finds a different result quarantines
//! the key until it re-proves itself; T5 a timed-out step is killed and
//! never recorded, and under audit restarts the proof (no divergence); T6
//! the agent ergonomics (`quod run -- cmd...` from inside the repo);
//! T7, a panic in the observer, is in observer_faults.rs (it needs the
//! `fault-injection` build); T8 a
//! network step leaves the checkout clean and observed steps see its module
//! cache from the state dir; T9 a repeated identical failure never replays;
//! T10 nondeterministic writes under HOME or GOPATH never replay; T11 the
//! observer's configuration (sealing on or off) never counts as a divergence.

mod common;
use std::process::Command;
use std::time::{Duration, Instant};

use common::*;

const REPLAY: &[&str] = &["--mode", "replay"];

/// The warm GOCACHE directory of the (only) toolchain in this state dir:
/// a host directory bound into warm steps, unobserved. The tests use it as
/// an input the observer cannot see, to make a step change its result.
fn warm_dir(env: &Env) -> std::path::PathBuf {
    let warm = env.state().join("warm");
    let image = std::fs::read_dir(&warm)
        .unwrap()
        .flatten()
        .next()
        .expect("one warm image dir");
    image.path().join("gocache")
}

#[test]
fn t1_a_nondeterministic_step_never_becomes_replayable() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let step = "head -c 8 /dev/urandom | od -An -tx1";
    let mut outs = Vec::new();
    for i in 0..4 {
        let o = env.run_step(step, REPLAY);
        assert!(o.executed() && o.code == 0, "run {i}: {}", o.stderr);
        assert!(!o.hit(), "run {i} must not replay: {}", o.stderr);
        if i == 0 {
            // Trust v2: recorded once, replayable, but the key's first
            // would-be hits are shadow audits.
            assert!(
                o.stderr.contains(
                    "trust: proof 1 of 1; replayable from the next matching run; its next 2 would-be hits are audited"
                ),
                "{}",
                o.stderr
            );
        } else {
            assert!(
                o.stderr.contains("QUOD  DIVERGENCE  step key quarantined")
                    && o.stderr.contains("stdout differs from line 1"),
                "run {i}: {}",
                o.stderr
            );
        }
        if i == 1 {
            assert!(
                o.stderr
                    .contains("QUOD  EXECUTED  (shadow audit: would-be hit 1 re-executed)"),
                "{}",
                o.stderr
            );
        }
        outs.push(o.stdout);
    }
    assert!(outs.windows(2).all(|w| w[0] != w[1]), "random output");
    let status = env.status();
    assert!(
        status.contains("quarantined") && status.contains("3 divergences"),
        "{status}"
    );
    assert!(
        status.contains("divergence rate: 3 of 3 audits"),
        "{status}"
    );
    println!(
        "T1 ok: a random step executed 4 times, never replayed, quarantined after 3 divergences"
    );
}

/// Trust v2: a result recorded once is replayable; the key's first two
/// would-be hits are shadow audits (executed and compared), the third
/// replays, with the trust facts in its receipt. Under v1 the second run
/// was "proof 2 of 2" and the third the first replay.
#[test]
fn t2_a_deterministic_step_replays_after_the_first_record_and_two_shadow_audits() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let step = "echo hi; echo err >&2";
    let r1 = env.run_step(step, REPLAY);
    assert!(
        r1.executed()
            && r1.stderr.contains(
                "trust: proof 1 of 1; replayable from the next matching run; its next 2 would-be hits are audited"
            ),
        "{}",
        r1.stderr
    );
    let status = env.status();
    assert!(
        status.contains("proven") && status.contains("proofs 1/1  audited 0x"),
        "{status}"
    );
    let r2 = env.run_step(step, REPLAY);
    assert!(
        r2.executed()
            && r2
                .stderr
                .contains("QUOD  EXECUTED  (shadow audit: would-be hit 1 re-executed)")
            && r2
                .stderr
                .contains("trust: shadow audit passed, byte-identical to run")
            && r2.stderr.contains("one more shadow audit before replays"),
        "{}",
        r2.stderr
    );
    let r3 = env.run_step(step, REPLAY);
    assert!(
        r3.executed()
            && r3
                .stderr
                .contains("(shadow audit: would-be hit 2 re-executed)")
            && r3.stderr.contains("the shadow phase is over"),
        "{}",
        r3.stderr
    );
    let r4 = env.run_step(step, REPLAY);
    assert!(r4.hit(), "{}", r4.stderr);
    assert!(
        r4.stderr
            .contains("trust: proven by 3 identical executions, audited 2 times, 0 divergences"),
        "{}",
        r4.stderr
    );
    assert_eq!(r4.stdout, b"hi\n");
    assert!(r4.stderr.ends_with("err\n"));
    let status = env.status();
    assert!(
        status.contains("replayed 1 (saved") && status.contains("proven"),
        "{status}"
    );
    assert!(
        status.contains("proofs 3/1  audited 2x  0 divergences  replayed 1x")
            && status.contains("output exact"),
        "{status}"
    );
    // inspect shows the trust state of the key.
    let key = status
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("b3:"))
        .map(|l| l.split_whitespace().next().unwrap().to_string())
        .expect("a key in status");
    let full = Command::new("sqlite3")
        .arg(env.state().join("ledger.sqlite"))
        .arg(format!(
            "SELECT step_key FROM trust WHERE step_key LIKE 'b3:{key}%'"
        ))
        .output()
        .unwrap();
    let full = String::from_utf8_lossy(&full.stdout).trim().to_string();
    let i = env.quod(&["inspect", &full]);
    let text = String::from_utf8_lossy(&i.stdout);
    assert!(
        text.contains("trust: proven")
            && text.contains("proofs 3")
            && text.contains("output exact")
            && text.contains("shadow"),
        "{text}"
    );
    println!(
        "T2 ok: one record, two shadow audits, then a replay whose receipt carries the trust facts"
    );
}

#[test]
fn t3_observe_mode_is_the_default_and_never_replays() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let step = "sleep 1; echo observed";
    for i in 0..3 {
        let o = env.run_step(step, &[]);
        assert!(
            o.executed() && !o.hit() && o.stdout == b"observed\n",
            "run {i}: {}",
            o.stderr
        );
        // Trust v2: recorded once, the second run would already replay.
        if i >= 1 {
            assert!(
                o.stderr
                    .contains("QUOD  EXECUTED  (observe mode: would have replayed,")
                    && o.stderr.contains("not saved)")
                    && o.stderr.contains("`quod run --mode replay` replays this"),
                "run {i}: {}",
                o.stderr
            );
        }
    }
    let status = env.status();
    assert!(
        status.contains("would have replayed 2 (observe mode:"),
        "{status}"
    );
    assert!(status.contains("replayed 0 (saved 0.0s)"), "{status}");
    // Switching to replay mode replays what observe mode proved: its two
    // would-be hits were compared (a free shadow phase), so this is hit 3.
    let o = env.run_step(step, REPLAY);
    assert!(o.hit() && o.stdout == b"observed\n", "{}", o.stderr);
    println!("T3 ok: observe mode executed 3 times and reported the replays it did not take");
}

#[test]
fn t4_an_audit_divergence_quarantines_the_key_until_it_reproves() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    // The step reads a file the observer cannot see (the warm build cache
    // is a host directory bound into the sandbox, unobserved by design).
    let step = r#"cat "$GOCACHE/flag" 2>/dev/null || echo none"#;
    let audit_all = &["--mode", "replay", "--warm", "--audit", "1"];
    let r1 = env.run_step(step, audit_all);
    let r2 = env.run_step(step, audit_all);
    assert!(
        r1.executed() && r2.executed() && r2.stdout == b"none\n",
        "{}",
        r2.stderr
    );
    // Trust v2: the second run is the key's first would-be hit, a shadow audit.
    assert!(
        r2.stderr
            .contains("(shadow audit: would-be hit 1 re-executed)")
            && r2.stderr.contains("trust: shadow audit passed"),
        "{}",
        r2.stderr
    );
    // The hidden input changes; the next would-be hit is audited (rate 1,
    // and still in the shadow).
    std::fs::write(warm_dir(&env).join("flag"), "changed\n").unwrap();
    let r3 = env.run_step(step, audit_all);
    assert!(r3.executed() && !r3.hit(), "{}", r3.stderr);
    assert_eq!(r3.stdout, b"changed\n", "the user gets the fresh result");
    assert!(
        r3.stderr
            .contains("QUOD  EXECUTED  (shadow audit: would-be hit 2 re-executed)")
            && r3
                .stderr
                .contains("QUOD  DIVERGENCE  step key quarantined")
            && r3
                .stderr
                .contains("stdout differs from line 1: \"none\" vs \"changed\"")
            && r3.stderr.contains(
                "executes until 2 identical executions prove it again; audited 2 times, 1 divergence"
            )
            && !r3.stderr.contains("may have served a wrong result"),
        "nothing was replayed, so nothing is suspect: {}",
        r3.stderr
    );
    // Quarantined: executes (no audit sampling applies), re-proving; the
    // bar is proofs + 1 after one divergence.
    let r4 = env.run_step(step, &["--mode", "replay", "--warm", "--audit", "0"]);
    assert!(
        r4.executed()
            && r4.stderr.contains("trust: quarantined since")
            && r4.stderr.contains("re-proof 2 of 2")
            && r4.stderr.contains("proven again"),
        "{}",
        r4.stderr
    );
    let r5 = env.run_step(step, &["--mode", "replay", "--warm", "--audit", "0"]);
    assert!(r5.hit() && r5.stdout == b"changed\n", "{}", r5.stderr);
    assert!(
        r5.stderr
            .contains("trust: proven by 2 identical executions, audited 3 times, 1 divergence"),
        "{}",
        r5.stderr
    );
    let status = env.status();
    assert!(
        status.contains("1 proven")
            && status.contains("1 divergence")
            && status.contains("output divergent"),
        "{status}"
    );
    assert!(
        status.contains("divergence rate: 1 of 3 audits (33.3%)"),
        "{status}"
    );
    println!(
        "T4 ok: a shadow audit that diverged quarantined the key; two identical runs re-proved it"
    );
}

#[test]
fn t5_a_timed_out_step_is_killed_and_never_recorded() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let t = Instant::now();
    let o = env.run_step(
        "sleep 120; echo late",
        &["--mode", "replay", "--timeout", "2s"],
    );
    assert_eq!(o.code, 124, "{}", o.stderr);
    assert!(
        o.stderr.contains("QUOD  TIMED OUT") && o.stderr.contains("not memoized"),
        "{}",
        o.stderr
    );
    assert!(!o.stderr.contains("ERROR"), "{}", o.stderr);
    // Generous: the box is shared; the point is 2s, not 120s.
    assert!(t.elapsed() < Duration::from_secs(60), "{:?}", t.elapsed());
    assert_eq!(env.manifests(), 0, "nothing recorded");
    assert!(o.stdout.is_empty());
    // A network step honors the timeout too.
    let t = Instant::now();
    let n = env.run_step("sleep 120", &["--allow-net", "--timeout", "1s"]);
    assert_eq!(n.code, 124, "{}", n.stderr);
    assert!(
        n.stderr.contains("TIMED OUT  the network step"),
        "{}",
        n.stderr
    );
    assert!(t.elapsed() < Duration::from_secs(60), "{:?}", t.elapsed());

    // A proven step that stops finishing (load, a slow observer, or the
    // step): not a result and not a divergence, but the expected result did
    // not reproduce, so the manifest proves again from 0.
    let step = r#"test -f "$GOCACHE/hang" && sleep 120; echo ok"#;
    let flags = &[
        "--mode",
        "replay",
        "--warm",
        "--audit",
        "1",
        "--timeout",
        "10s",
    ];
    assert!(env.run_step(step, flags).executed());
    let r2 = env.run_step(step, flags);
    assert!(
        r2.executed() && r2.stderr.contains("shadow audit passed"),
        "{}",
        r2.stderr
    );
    std::fs::write(warm_dir(&env).join("hang"), "").unwrap();
    let r3 = env.run_step(step, flags);
    assert_eq!(r3.code, 124, "{}", r3.stderr);
    assert!(
        r3.stderr.contains("QUOD  TIMED OUT")
            && !r3.stderr.contains("DIVERGENCE")
            && r3
                .stderr
                .contains("the step timed out after 10.0s, which is not a result")
            && r3.stderr.contains("proves again from 0 of 1"),
        "{}",
        r3.stderr
    );
    assert_eq!(env.manifests(), 1, "the timed-out run recorded no manifest");
    let status = env.status();
    assert!(
        !status.contains("quarantined  ")
            && status.contains("0 quarantined")
            && status.contains("3 timed out"),
        "{status}"
    );
    // It finishes again: two identical executions re-prove it, then it replays.
    std::fs::remove_file(warm_dir(&env).join("hang")).unwrap();
    let no_audit = &[
        "--mode",
        "replay",
        "--warm",
        "--audit",
        "0",
        "--timeout",
        "10s",
    ];
    // (Trust v2: one identical execution re-proves it; --audit 0 means no
    // shadow audits either, so the next run replays.)
    let r4 = env.run_step(step, no_audit);
    assert!(
        r4.executed() && r4.stderr.contains("proof 1 of 1"),
        "{}",
        r4.stderr
    );
    assert!(env.run_step(step, no_audit).hit());
    println!(
        "T5 ok: timeouts kill the step, record nothing, and restart the proof without a divergence"
    );
}

#[test]
fn t6_run_takes_a_trailing_command_and_defaults_the_repo_to_the_checkout() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    // No --repo, no --step: the repo is the checkout around the current
    // directory, the step is the command after `--`.
    let run_in = |dir: &str, args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_quod"))
            .arg("--state")
            .arg(env.state())
            .args([
                "run", "--mode", "replay", "--proofs", "1", "--audit", "0", "--",
            ])
            .args(args)
            .current_dir(env.repo.join(dir))
            .output()
            .unwrap();
        Out {
            code: out.status.code().unwrap_or(-1),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).into(),
        }
    };
    let o = run_in(".", &["go", "test", "./..."]);
    assert!(o.executed() && o.code == 0, "{}", o.stderr);
    let text = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        text.contains("example.com/tiny/calc") && text.contains("example.com/tiny/other"),
        "{text}"
    );
    // The same step spelled with --step is the same key: it replays.
    let o = env.run_step("go test ./...", &[]);
    assert!(o.hit(), "{}", o.stderr);
    // From a subdirectory the command runs there, as it would without
    // Quod: `./...` is calc's packages only, and it is another key.
    let o = run_in("calc", &["go", "test", "./..."]);
    assert!(o.executed() && o.code == 0, "{}", o.stderr);
    let text = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        text.contains("example.com/tiny/calc") && !text.contains("example.com/tiny/other"),
        "{text}"
    );
    let o = run_in("calc", &["go", "test", "./..."]);
    assert!(o.hit(), "{}", o.stderr);
    // Arguments with spaces are quoted into the script, and hyphens pass.
    let o = run_in(".", &["sh", "-c", "echo a b; exit 3"]);
    assert!(
        o.executed() && o.code == 3 && o.stdout == b"a b\n",
        "{}",
        o.stderr
    );
    let o = run_in(".", &["go", "test", "-run", "TestAdd", "-v", "./calc"]);
    assert!(o.executed() && o.code == 0, "{}", o.stderr);
    assert!(String::from_utf8_lossy(&o.stdout).contains("--- PASS: TestAdd"));
    // Both at once, or neither, is an error.
    let o = env.quod(&[
        "run",
        "--repo",
        env.repo.to_str().unwrap(),
        "--step",
        "true",
        "--",
        "true",
    ]);
    assert_eq!(o.code, 2);
    let o = env.quod(&["run", "--repo", env.repo.to_str().unwrap()]);
    assert_eq!(o.code, 2);
    // status from inside the checkout finds the repo and names the keys.
    let s = Command::new(env!("CARGO_BIN_EXE_quod"))
        .arg("--state")
        .arg(env.state())
        .arg("status")
        .current_dir(env.repo.join("calc"))
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&s.stdout);
    assert!(
        s.contains("replayed 2")
            && s.contains("4 step key(s)")
            && s.contains("cd calc; go test ./..."),
        "{s}"
    );
    println!("T6 ok: quod run -- <cmd> from inside the checkout, in the directory it was run from");
}

#[test]
fn t8_a_network_step_keeps_the_checkout_clean_and_fills_the_state_dir() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    // Stands in for `go mod download`: writes where the env points.
    let step_a = r#"mkdir -p "$GOMODCACHE/cache/download" "$GOMODCACHE/example.com/dep@v1.0.0" && echo dep > "$GOMODCACHE/example.com/dep@v1.0.0/dep.txt" && echo "$GOMODCACHE" && echo "$GOCACHE" && echo "$HOME" && echo touched > step-a.txt"#;
    let a = env.run_step(step_a, &["--allow-net"]);
    assert_eq!(a.code, 0, "{}", a.stderr);
    assert!(
        a.stderr.contains("network allowed, not memoized"),
        "{}",
        a.stderr
    );
    // The env is the profile's, unchanged: the state dir's `.quod` is
    // bound over the workspace's, so a tree that names its own path (a
    // venv) is built where it is used.
    assert_eq!(
        String::from_utf8_lossy(&a.stdout),
        "/workspace/.quod/gomod\n/workspace/.quod/gocache\n/workspace/.quod/home\n"
    );
    assert!(
        !env.repo.join(".quod").exists(),
        "the checkout gets no caches"
    );
    assert_eq!(
        std::fs::read_to_string(env.repo.join("step-a.txt")).unwrap(),
        "touched\n",
        "its own writes persist"
    );
    // The caches live where the banner says:
    // <state>/deps/<repo>/<image>/<toolchain> (or under QUOD_DEPS_DIR
    // when that is set).
    let reported = a
        .stderr
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("caches in "))
        .map(std::path::PathBuf::from)
        .expect("the banner names the cache dir");
    if std::env::var_os("QUOD_DEPS_DIR").is_none_or(|v| v.is_empty()) {
        assert!(
            reported.starts_with(env.state().join("deps")),
            "{reported:?}"
        );
    }
    let found: Vec<_> = walk(&reported)
        .into_iter()
        .filter(|p| p.ends_with("gomod/example.com/dep@v1.0.0/dep.txt"))
        .collect();
    assert_eq!(found.len(), 1, "{found:?}");
    let deps_dir = found[0].ancestors().nth(4).unwrap().to_path_buf();
    assert_eq!(deps_dir, reported);
    assert_eq!(deps_dir.file_name().unwrap(), "go", "{deps_dir:?}");
    assert!(
        deps_dir
            .ancestors()
            .nth(2)
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("git:"),
        "{deps_dir:?}"
    );

    // An observed step sees the module cache at its workspace path, served
    // from the state dir; the manifest records it (sealed, whole).
    let b = env.run_step(
        "cat .quod/gomod/example.com/dep@v1.0.0/dep.txt; ls .quod",
        &[],
    );
    assert!(b.executed() && b.code == 0, "{}", b.stderr);
    assert_eq!(String::from_utf8_lossy(&b.stdout), "dep\ngomod\n");
    let json = Command::new("sqlite3")
        .arg(env.state().join("ledger.sqlite"))
        .arg("SELECT json FROM manifests")
        .output()
        .unwrap();
    let json = String::from_utf8_lossy(&json.stdout);
    assert!(
        json.contains("\"p\":\".quod/gomod/example.com/dep@v1.0.0/dep.txt\""),
        "{json}"
    );
    assert!(!env.repo.join(".quod").exists());
    let o = env.run_step(
        "cat .quod/gomod/example.com/dep@v1.0.0/dep.txt; ls .quod",
        &[],
    );
    assert!(o.hit(), "{}", o.stderr);
    // A change to the cache in the state dir is a miss that names it.
    std::fs::write(found[0].clone(), "dep2\n").unwrap();
    let o = env.run_step(
        "cat .quod/gomod/example.com/dep@v1.0.0/dep.txt; ls .quod",
        &[],
    );
    assert!(
        o.executed()
            && o.stderr
                .contains("differed at .quod/gomod/example.com/dep@v1.0.0/dep.txt"),
        "{}",
        o.stderr
    );
    assert_eq!(String::from_utf8_lossy(&o.stdout), "dep2\ngomod\n");
    let status = env.status();
    assert!(status.contains("1 network"), "{status}");
    println!(
        "T8 ok: the network step wrote its caches to the state dir; observed steps read them from the tree"
    );
}

/// The real case: cli/cli's TestHuhPrompterConfirm hung to go test's
/// 10-minute timeout under load, and on a busy box it can do so twice in a
/// row with identical output. A failure is never replayed by default, and
/// output with a kill or timeout marker never: the hang executes every
/// time, the pass that follows quarantines the key. The flake is driven by
/// a file in the warm build cache, an input the observer does not see.
#[test]
fn t9_a_repeated_identical_failure_never_replays() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let step = r#"if [ -f "$GOCACHE/pass" ]; then echo ok; else echo '--- FAIL: TestHuhPrompterConfirm (600.00s)'; echo 'panic: test timed out after 10m0s'; exit 1; fi"#;
    let flags = &["--mode", "replay", "--warm"];
    for i in 0..3 {
        let r = env.run_step(step, flags);
        assert!(
            r.executed()
                && !r.hit()
                && r.code == 1
                && r.stderr.contains(
                    "trust: not provable (its stdout says \"panic: test timed out after\""
                ),
            "identical failure {i}: {}",
            r.stderr
        );
    }
    // The flake clears; the recorded failure is not replayed.
    std::fs::write(warm_dir(&env).join("pass"), "").unwrap();
    let r2 = env.run_step(step, flags);
    assert!(r2.executed() && !r2.hit(), "{}", r2.stderr);
    assert_eq!(r2.code, 0, "the fresh pass, not the recorded failure");
    assert_eq!(r2.stdout, b"ok\n");
    assert!(
        r2.stderr.contains("QUOD  DIVERGENCE  step key quarantined")
            && r2.stderr.contains("exit code 1 -> 0"),
        "{}",
        r2.stderr
    );
    // Quarantined: it executes until two identical runs (proofs + one per
    // divergence) re-prove it.
    let r3 = env.run_step(step, flags);
    assert!(
        r3.executed() && r3.code == 0 && r3.stderr.contains("re-proof 2 of 2"),
        "{}",
        r3.stderr
    );

    // A failure without a kill marker: never replayed by default, and with
    // --replay-failures it proves itself like a pass.
    let fail = "echo FAIL; exit 1";
    for i in 0..3 {
        let r = env.run_step(fail, &["--mode", "replay"]);
        assert!(
            r.executed() && !r.hit() && r.stderr.contains("not provable (exit code 1"),
            "failure {i}: {}",
            r.stderr
        );
    }
    // Trust v2: recorded once, then the key's two shadow audits, then replay.
    let opt_in = &["--mode", "replay", "--replay-failures"];
    for i in 0..3 {
        let r = env.run_step(fail, opt_in);
        assert!(r.executed() && r.code == 1, "opt-in {i}: {}", r.stderr);
    }
    let r = env.run_step(fail, opt_in);
    assert!(r.hit() && r.code == 1, "{}", r.stderr);
    let r = env.run_step(fail, &["--mode", "replay"]);
    assert!(
        r.executed() && !r.hit(),
        "default policy again: {}",
        r.stderr
    );
    let status = env.status();
    assert!(
        status.contains("why: failing: its last run exited 1, and failures never prove"),
        "{status}"
    );
    println!(
        "T9 ok: three identical timed-out failures never replayed, the pass quarantined the key; a plain failure replays only with --replay-failures"
    );
}

/// What a step leaves in HOME or GOPATH is a result unless it is on the
/// short list of volatile paths: random bytes written to `$HOME/tok`, or a
/// binary `go install` stamps with the time, diverge on every execution
/// and never become replayable.
#[test]
fn t10_nondeterministic_writes_under_home_or_gopath_never_replay() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let tok = r#"mkdir -p "$HOME" && head -c 16 /dev/urandom > "$HOME/tok"; echo done"#;
    for i in 0..3 {
        let o = env.run_step(tok, REPLAY);
        assert!(
            o.executed() && !o.hit() && o.code == 0,
            "run {i}: {}",
            o.stderr
        );
        if i > 0 {
            assert!(
                o.stderr.contains("QUOD  DIVERGENCE")
                    && o.stderr.contains(".quod/home/tok changed"),
                "run {i}: {}",
                o.stderr
            );
        }
    }
    env.write(
        "cmd/tool/main.go",
        "package main\n\nvar built string\n\nfunc main() { println(built) }\n",
    );
    let install =
        r#"go install -ldflags "-X main.built=$(date +%s%N)" ./cmd/tool && echo installed"#;
    for i in 0..3 {
        let o = env.run_step(install, REPLAY);
        assert!(
            o.executed() && !o.hit() && o.code == 0,
            "run {i}: {}",
            o.stderr
        );
        if i > 0 {
            assert!(
                o.stderr.contains("QUOD  DIVERGENCE")
                    && o.stderr.contains(".quod/gopath/bin/tool changed"),
                "run {i}: {}",
                o.stderr
            );
        }
    }
    println!("T10 ok: random bytes in HOME and a time-stamped go install never became replayable");
}

/// Quod's own configuration is not the step's behavior: a sealed and
/// an unsealed run (QUOD_SEAL_MODCACHE=0, or a snapshot that could not
/// be made) record the module cache whole either way, read equivalent
/// manifests, and never count as a divergence.
#[test]
fn t11_the_observer_configuration_never_diverges() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let step_a = r#"mkdir -p "$GOMODCACHE/cache/download" "$GOMODCACHE/example.com/dep@v1.0.0" && echo dep > "$GOMODCACHE/example.com/dep@v1.0.0/dep.txt""#;
    assert_eq!(env.run_step(step_a, &["--allow-net"]).code, 0);
    let step = "cat .quod/gomod/example.com/dep@v1.0.0/dep.txt; ls .quod/gomod/example.com; test -e .quod/gomod/example.com/dep@v1.0.0/go.mod || echo no-go-mod";
    let run = |seal: &str| {
        let repo = env.repo.to_str().unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_quod"))
            .arg("--state")
            .arg(env.state())
            .args(["run", "--repo", repo, "--step", step, "--mode", "replay"])
            .env("QUOD_SEAL_MODCACHE", seal)
            .output()
            .unwrap();
        Out {
            code: out.status.code().unwrap_or(-1),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).into(),
        }
    };
    let r1 = run("1");
    assert!(r1.executed() && r1.code == 0, "{}", r1.stderr);
    let r2 = run("0");
    assert!(
        r2.executed() && !r2.stderr.contains("DIVERGENCE") && r2.stdout == r1.stdout,
        "unsealed after sealed: {}",
        r2.stderr
    );
    let r3 = run("1");
    assert!(
        !r3.stderr.contains("DIVERGENCE"),
        "sealed after unsealed: {}",
        r3.stderr
    );
    let r4 = run("0");
    assert!(!r4.stderr.contains("DIVERGENCE"), "{}", r4.stderr);
    let status = env.status();
    assert!(
        status.contains("0 quarantined") && status.contains("0 divergences"),
        "{status}"
    );
    println!("T11 ok: sealed and unsealed runs of one tree never diverged");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}
