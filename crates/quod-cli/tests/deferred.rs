//! Deferred audits (docs/trust-v2.md, "Deferred audits"), end to end
//! through the binary. Gated on QUOD_INTEGRATION=1 like the rest.
//!
//! D1 a decay-phase audit serves the replay at once (the job's wall time
//! has no execution in it) and `quod audit` executes and compares it
//! later; the shadow audits before it stay inline; D2 a deferred audit that
//! diverges quarantines the key and marks the replay it checked suspect;
//! D3 a lost audit is retried when its worker died, expires as not passed
//! when nobody ran it, and is then repaid by the key's next would-be hit,
//! inline; D4 `--inline-audits` keeps every audit inline; D5 `quod run`
//! starts the worker itself, detached.

mod common;
use std::process::Command;
use std::time::{Duration, Instant};

use common::*;

/// Replay mode with every would-be hit on the schedule once the shadow is
/// over (floor 1): hits 1 and 2 are shadow audits, inline; from hit 3 on
/// every hit is a decay audit, deferred by default.
const ALL: &[&str] = &["--mode", "replay", "--audit", "1"];
const NO_SPAWN: &[(&str, &str)] = &[("QUOD_AUDIT_SPAWN", "0")];

fn warm_dir(env: &Env) -> std::path::PathBuf {
    let warm = env.state().join("warm");
    let image = std::fs::read_dir(&warm)
        .unwrap()
        .flatten()
        .next()
        .expect("one warm image dir");
    image.path().join("gocache")
}

/// `quod run` with the audit worker left to the test.
fn run(env: &Env, step: &str, extra: &[&str]) -> Out {
    let repo = env.repo.to_str().unwrap().to_string();
    let mut args = vec!["run", "--repo", &repo, "--step", step];
    if let Some(tc) = env.toolchain {
        args.extend(["--toolchain", tc.name()]);
    }
    args.extend_from_slice(extra);
    env.quod_in(&env.state(), &args, NO_SPAWN)
}

fn sql(env: &Env, q: &str) -> String {
    let out = Command::new("sqlite3")
        .arg(env.state().join("ledger.sqlite"))
        .arg(q)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{q}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `quod audit` (the worker), returning its stderr.
fn audit(env: &Env, extra: &[&str]) -> String {
    let mut args = vec!["audit"];
    args.extend_from_slice(extra);
    let o = env.quod_in(&env.state(), &args, NO_SPAWN);
    assert_eq!(o.code, 0, "{}", o.stderr);
    o.stderr
}

/// One record and the two inline shadow audits: the key is ready to
/// replay blind, and its next would-be hit (3) is a deferred audit.
fn through_the_shadow(env: &Env, step: &str, extra: &[&str]) {
    let r1 = run(env, step, extra);
    assert!(r1.executed() && r1.code == 0, "{}", r1.stderr);
    for nth in 1..=2 {
        let r = run(env, step, extra);
        assert!(
            r.executed()
                && r.stderr
                    .contains(&format!("(shadow audit: would-be hit {nth} re-executed)"))
                && r.stderr.contains("trust: shadow audit passed"),
            "shadow audits stay inline: {}",
            r.stderr
        );
    }
}

#[test]
fn d1_a_decay_audit_is_deferred_and_the_worker_settles_it() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let step = "sleep 3; echo hi; echo err >&2";
    through_the_shadow(&env, step, ALL);
    // Hit 3: the replay is served; nothing executes in this run.
    let t = Instant::now();
    let r = run(&env, step, ALL);
    let wall = t.elapsed();
    assert!(r.hit() && r.code == 0, "{}", r.stderr);
    assert_eq!(r.stdout, b"hi\n");
    assert!(
        r.stderr.contains("trust: proven by 3 identical executions, audited 2 times, 0 divergences")
            && r.stderr.contains(
                "trust: audit deferred: would-be hit 3 is re-executed and compared in the background (pending audit 1)"
            ),
        "{}",
        r.stderr
    );
    let ev = last_event(&env.state(), "run").unwrap();
    assert_eq!(ev["outcome"], "replayed");
    assert_eq!(ev["deferred_audit"], 1);
    assert_eq!(ev["exec_ms"], 0.0, "no execution in the job: {ev}");
    assert_eq!(
        sql(&env, "SELECT state, nth, attempts FROM audits_pending"),
        "pending|3|0"
    );
    assert_eq!(
        sql(&env, "SELECT eligible, replays, audits FROM trust"),
        "3|1|2",
        "the replay counted as a would-be hit; the audit is not counted until it runs"
    );
    let status = env.status();
    assert!(
        status.contains("deferred audits: 1 pending, 0 running, 0 lost"),
        "{status}"
    );
    // The worker: executes the step on the recorded tree, compares, settles.
    let w = audit(&env, &[]);
    assert!(
        w.contains("(deferred audit 1: would-be hit 3 re-executed after its replay was served)")
            && w.contains("trust: deferred audit passed, byte-identical to run")
            && w.contains(
                "1 executed (0 diverged), 0 lost; 0 pending, 0 running, 0 lost and not repaid"
            ),
        "{w}"
    );
    assert_eq!(
        sql(
            &env,
            "SELECT state, attempts, audit_id IS NOT NULL FROM audits_pending"
        ),
        "passed|1|1"
    );
    assert_eq!(
        sql(&env, "SELECT eligible, replays, audits FROM trust"),
        "3|1|3"
    );
    assert_eq!(
        sql(&env, "SELECT phase FROM audits ORDER BY id"),
        "shadow\nshadow\ndeferred"
    );
    let ev = last_event(&env.state(), "audit").unwrap();
    assert_eq!(
        (ev["outcome"].as_str(), ev["diverged"].as_bool()),
        (Some("audit"), Some(false))
    );
    assert!(
        ev["exec_ms"].as_u64().unwrap() >= 3000,
        "the audit ran the step: {ev}"
    );
    let status = env.status();
    assert!(
        status.contains("audited 3x  0 divergences  replayed 1x")
            && !status.contains("deferred audits:"),
        "{status}"
    );
    // An idle worker says so and exits.
    assert!(audit(&env, &[]).contains("0 executed (0 diverged), 0 lost; 0 pending"));
    println!(
        "D1 ok: the replay was served in {} while the step takes 3 s; the audit ran afterwards and agreed",
        quod_replay::fmt_duration(wall)
    );
}

#[test]
fn d2_a_deferred_audit_that_diverges_quarantines_and_marks_the_replay_suspect() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    // The step reads a file the observer cannot see (the warm build cache
    // is a host directory bound into the sandbox, unobserved by design).
    let step = r#"cat "$GOCACHE/flag" 2>/dev/null || echo none"#;
    let flags = &["--mode", "replay", "--warm", "--audit", "1"];
    through_the_shadow(&env, step, flags);
    let r = run(&env, step, flags);
    assert!(r.hit() && r.stdout == b"none\n", "{}", r.stderr);
    assert!(r.stderr.contains("pending audit 1"), "{}", r.stderr);
    let replay_run = sql(&env, "SELECT replay_run FROM audits_pending WHERE id = 1");
    assert_eq!(
        sql(
            &env,
            &format!("SELECT outcome, suspect IS NULL FROM runs WHERE id = '{replay_run}'")
        ),
        "replayed|1"
    );
    // The hidden input changes before the audit runs: what was served is
    // not what the step does now.
    std::fs::write(warm_dir(&env).join("flag"), "changed\n").unwrap();
    let w = audit(&env, &[]);
    assert!(
        w.contains("QUOD  DIVERGENCE  step key quarantined")
            && w.contains("stdout differs from line 1: \"none\" vs \"changed\"")
            && w.contains("1 earlier replay of this key may have served a wrong result: marked suspect in the ledger; re-run it")
            && w.contains("1 executed (1 diverged)"),
        "{w}"
    );
    assert_eq!(
        sql(&env, "SELECT state FROM audits_pending WHERE id = 1"),
        "diverged"
    );
    assert_eq!(
        sql(
            &env,
            &format!("SELECT suspect IS NOT NULL FROM runs WHERE id = '{replay_run}'")
        ),
        "1",
        "the replay the audit checked is suspect"
    );
    assert_eq!(sql(&env, "SELECT state FROM trust"), "quarantined");
    let status = env.status();
    assert!(
        status.contains("1 quarantined")
            && status.contains("suspect replays: 1")
            && status.contains("output divergent"),
        "{status}"
    );
    // Quarantined: the next run executes to re-prove (the bar is 2 now).
    let r = run(&env, step, flags);
    assert!(
        r.executed()
            && r.stdout == b"changed\n"
            && r.stderr.contains("trust: quarantined since")
            && r.stderr.contains("re-proof"),
        "{}",
        r.stderr
    );
    println!(
        "D2 ok: the deferred audit diverged, quarantined the key and marked the replay suspect"
    );
}

#[test]
fn d3_a_lost_audit_is_retried_then_expires_and_is_repaid_inline() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let step = "echo steady";
    through_the_shadow(&env, step, ALL);
    // Hit 3: deferred. Its worker died mid-way (a stale claim): the next
    // worker releases it and runs it again.
    assert!(run(&env, step, ALL).hit());
    sql(
        &env,
        "UPDATE audits_pending SET state = 'running', attempts = 1, claimed_at = '2000-01-01T00:00:00.000Z', claimed_by = 'dead:1' WHERE id = 1",
    );
    let w = audit(&env, &[]);
    assert!(
        w.contains("(deferred audit 1: would-be hit 3 re-executed")
            && w.contains("trust: deferred audit passed"),
        "retried: {w}"
    );
    assert_eq!(
        sql(
            &env,
            "SELECT state, attempts FROM audits_pending WHERE id = 1"
        ),
        "passed|2"
    );
    // Hit 4: deferred again, and nobody runs it for a day: lost, not passed.
    assert!(run(&env, step, ALL).hit());
    sql(
        &env,
        "UPDATE audits_pending SET enqueued_at = '2000-01-01T00:00:00.000Z' WHERE id = 2",
    );
    let w = audit(&env, &[]);
    assert!(
        w.contains("pending audit 2 lost: not executed since 2000-01-01")
            && w.contains("the next would-be hit of its key is audited inline")
            && w.contains(
                "0 executed (0 diverged), 1 lost; 0 pending, 0 running, 1 lost and not repaid"
            ),
        "{w}"
    );
    assert_eq!(
        sql(
            &env,
            "SELECT state, repaid FROM audits_pending WHERE id = 2"
        ),
        "lost|0"
    );
    assert_eq!(
        sql(&env, "SELECT audits FROM trust"),
        "3",
        "a lost audit never counts"
    );
    let status = env.status();
    assert!(
        status.contains("deferred audits: 0 pending, 0 running, 1 lost and not yet repaid"),
        "{status}"
    );
    // Hit 5 makes up for it: audited inline, whatever the schedule says.
    let r = run(&env, step, ALL);
    assert!(
        r.executed()
            && r.stderr.contains("(audit: would-be hit 5 re-executed)")
            && r.stderr
                .contains("trust: audit passed, byte-identical to run")
            && r.stderr.contains("(making up for a lost deferred audit)"),
        "{}",
        r.stderr
    );
    assert_eq!(
        sql(
            &env,
            "SELECT state, repaid FROM audits_pending WHERE id = 2"
        ),
        "lost|1"
    );
    assert!(!env.status().contains("deferred audits:"));
    // Attempts run out: an audit whose worker keeps dying is lost too.
    assert!(run(&env, step, ALL).hit(), "hit 6: deferred");
    sql(
        &env,
        "UPDATE audits_pending SET state = 'running', attempts = 3, claimed_at = '2000-01-01T00:00:00.000Z', claimed_by = 'dead:2' WHERE id = 3",
    );
    let w = audit(&env, &[]);
    assert!(
        w.contains("pending audit 3 lost: worker dead:2 did not finish it"),
        "{w}"
    );
    println!(
        "D3 ok: a stale claim was retried; an expired audit was lost, shown, and repaid inline"
    );
}

#[test]
fn d4_inline_audits_can_be_asked_for() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let step = "echo inline";
    let flags = &["--mode", "replay", "--audit", "1", "--inline-audits"];
    through_the_shadow(&env, step, flags);
    let r = run(&env, step, flags);
    assert!(
        r.executed()
            && r.stderr.contains("(audit: would-be hit 3 re-executed)")
            && r.stderr.contains("trust: audit passed"),
        "{}",
        r.stderr
    );
    assert_eq!(sql(&env, "SELECT count(*) FROM audits_pending"), "0");
    // The environment form, as a suite sets it.
    let repo = env.repo.to_str().unwrap().to_string();
    let o = env.quod_in(
        &env.state(),
        &[
            "run",
            "--repo",
            &repo,
            "--toolchain",
            "go",
            "--step",
            step,
            "--mode",
            "replay",
            "--audit",
            "1",
        ],
        &[("QUOD_INLINE_AUDITS", "1"), ("QUOD_AUDIT_SPAWN", "0")],
    );
    assert!(
        o.executed() && o.stderr.contains("(audit: would-be hit 4 re-executed)"),
        "{}",
        o.stderr
    );
    println!("D4 ok: --inline-audits executes every audit in the job");
}

#[test]
fn d5_quod_run_starts_the_worker_itself() {
    if !enabled() {
        return;
    }
    let mut env = Env::new();
    env.run_defaults.clear();
    let step = "echo detached";
    through_the_shadow(&env, step, ALL);
    // No QUOD_AUDIT_SPAWN=0: the run starts `quod audit`, detached.
    let r = env.run_step(step, ALL);
    assert!(
        r.hit() && r.stderr.contains("pending audit 1"),
        "{}",
        r.stderr
    );
    let t = Instant::now();
    let state = loop {
        let s = sql(&env, "SELECT state FROM audits_pending WHERE id = 1");
        if s == "passed" || s == "diverged" || s == "lost" || t.elapsed() > Duration::from_secs(120)
        {
            break s;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let log = std::fs::read_to_string(env.state().join("audit.log")).unwrap_or_default();
    assert_eq!(state, "passed", "worker log:\n{log}");
    assert!(log.contains("trust: deferred audit passed"), "{log}");
    println!(
        "D5 ok: the detached worker settled the audit in {}",
        quod_replay::fmt_duration(t.elapsed())
    );
}
