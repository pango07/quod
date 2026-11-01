//! A1-A7 (+A7b) through the hooks path. The test plays actions/runner: it
//! writes the hook inputs the runner would (prepare_job, a CAS checkout from
//! a local git "server", `run:` steps as scripts under RUNNER_TEMP,
//! cleanup_job) and feeds them to `quod hooks`. Same criteria as
//! tests/acceptance.rs, so the two paths can be compared (gate (a) compares
//! the recorded manifests' f/a sets with the CLI path's on the same tree).
//!
//! Runs on the tiny Go module by default, or on the spike repo when
//! QUOD_SPIKE_REPO is set (a checkout whose .quod/gomod step A
//! filled). Gated on QUOD_INTEGRATION=1. Host-root mode
//! (QUOD_ROOTFS=host) by default; QUOD_HOOKS_IMAGE=<image> with
//! QUOD_HOOKS_ROOTFS=<a state's rootfs dir holding it, unpacked> runs
//! every step in that image instead, as a real job does.

mod common;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use common::*;
use quod_cas::{Cas, Entry, Hash, Kind, child_path};
use quod_ledger::{Ledger, first_match};

const STEP_DEFAULT: &str = "go test ./...";
const WORKSPACE: &str = "/__w/o/r";

/// The runner's side of the protocol.
struct Runner {
    env: Env,
    work: PathBuf,
    state: PathBuf,
    job: Option<String>,
    steps: u32,
    mode: &'static str,
    /// The trust policy: the matcher criteria (A1-A7) predate the trust
    /// layer and run with one proof and no audit; T1-T2 use the real one.
    proofs: u32,
    audit: f64,
    /// The job container's create options (who the steps run as).
    user: &'static str,
    /// The event: its name (GITHUB_EVENT_NAME) and the payload the runner
    /// writes to `_temp/_github_workflow/event.json`.
    event: (&'static str, serde_json::Value),
}

/// A push to the default branch: a writer job of the trusted scope.
fn push_to_main() -> (&'static str, serde_json::Value) {
    (
        "push",
        serde_json::json!({
            "ref": "refs/heads/main",
            "after": "0000000000000000000000000000000000000001",
            "pusher": {"name": "o"},
            "repository": {"full_name": "o/r", "default_branch": "main"},
        }),
    )
}

/// A pull request from a branch of the repository itself: trusted (the
/// same ledger), never a writer.
fn same_repo_pull_request() -> (&'static str, serde_json::Value) {
    (
        "pull_request",
        serde_json::json!({
            "action": "synchronize",
            "number": 7,
            "pull_request": {"head": {"ref": "dependabot/x", "repo": {"full_name": "o/r"}}},
            "repository": {"full_name": "o/r", "default_branch": "main"},
        }),
    )
}

struct StepOut {
    code: i32,
    stdout: Vec<u8>,
    stderr: String,
    summary: String,
}

impl StepOut {
    fn executed(&self) -> bool {
        self.stderr.contains("QUOD  EXECUTED")
    }
    fn replayed(&self) -> bool {
        self.stderr.contains("QUOD  REPLAYED")
    }
}

impl Runner {
    fn new(env: Env) -> Runner {
        let work = env.tmp.path().join("_work");
        for d in [
            "_temp/_runner_file_commands",
            "_temp/_github_workflow",
            "_actions/actions/checkout/abc/dist",
            "o/r",
            "_tool",
        ] {
            std::fs::create_dir_all(work.join(d)).unwrap();
        }
        // The spike repo is the host workspace; jobs check out of the local
        // git server. Its .quod/ (the module cache step A filled) is the
        // operator's seed the scope's first job starts from: the seeds
        // directory of the state, never the runner's work directory.
        let st = Command::new("cp")
            .arg("-a")
            .arg(env.repo.join("."))
            .arg(work.join("o/r"))
            .status()
            .unwrap();
        assert!(st.success());
        if env.repo.join(".quod").is_dir() {
            let seed = env.state().join("seeds/o_r");
            std::fs::create_dir_all(&seed).unwrap();
            let st = Command::new("cp")
                .arg("-a")
                .arg(env.repo.join(".quod"))
                .arg(&seed)
                .status()
                .unwrap();
            assert!(st.success());
        }
        if image().is_some() {
            // The image, unpacked once, shared read-only with this state.
            let src = std::env::var("QUOD_HOOKS_ROOTFS")
                .expect("QUOD_HOOKS_IMAGE needs QUOD_HOOKS_ROOTFS");
            std::fs::create_dir_all(env.state()).unwrap();
            std::os::unix::fs::symlink(src, env.state().join("rootfs")).unwrap();
        }
        Runner {
            state: env.state(),
            env,
            work,
            job: None,
            steps: 0,
            mode: "observe",
            proofs: 1,
            audit: 0.0,
            user: "--user 1001:1001",
            event: push_to_main(),
        }
    }

    fn mounts(&self) -> serde_json::Value {
        let m = |src: PathBuf, dst: &str, ro: bool| serde_json::json!({"sourceVolumePath": src, "targetVolumePath": dst, "readOnly": ro});
        serde_json::json!([
            m(self.work.clone(), "/__w", false),
            m(self.work.join("_temp"), "/__w/_temp", false),
            m(
                self.work.join("_temp/_github_workflow"),
                "/github/workflow",
                false
            ),
            m(self.work.join("_actions"), "/__w/_actions", false),
            m(self.work.join("_tool"), "/__w/_tool", false),
        ])
    }

    fn hooks(
        &self,
        input: &serde_json::Value,
        extra_env: &[(&str, &str)],
    ) -> (i32, Vec<u8>, String) {
        let policy = serde_json::json!({
            "mode": self.mode,
            "proofs": self.proofs,
            "audit": self.audit,
            "keep_jobs": true,
            "rootfs": if image().is_some() { "image" } else { "host" },
        });
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_quod"));
        cmd.arg("--state")
            .arg(&self.state)
            .arg("hooks")
            .arg("--policy")
            .arg(policy.to_string())
            // The step's environment reaches the hook process; a workflow
            // that sets these must change nothing (the policy above wins).
            .env("QUOD_MODE", "replay")
            .env("QUOD_PROOFS", "1")
            .env("QUOD_AUDIT", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(serde_json::to_string(input).unwrap().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        (out.status.code().unwrap_or(-1), out.stdout, stderr)
    }

    fn prepare_job(&mut self) {
        std::fs::write(
            self.work.join("_temp/_github_workflow/event.json"),
            self.event.1.to_string(),
        )
        .unwrap();
        let resp = self.work.join("_temp/response.json");
        std::fs::write(&resp, "").unwrap();
        let input = serde_json::json!({
            "command": "prepare_job",
            "responseFile": resp,
            "state": {},
            "args": {
                "container": {
                    "image": image().unwrap_or_else(|| "golang@sha256:test".into()),
                    "workingDirectory": WORKSPACE,
                    "createOptions": self.user,
                    "environmentVariables": {"HOME": "/github/home"},
                    "systemMountVolumes": self.mounts(),
                    "userMountVolumes": []
                },
                "services": []
            }
        });
        let (code, _, stderr) = self.hooks(&input, &[]);
        assert_eq!(code, 0, "prepare_job: {stderr}");
        let r: serde_json::Value = serde_json::from_slice(&std::fs::read(&resp).unwrap()).unwrap();
        assert_eq!(r["isAlpine"], false);
        self.job = Some(r["state"]["job"].as_str().unwrap().to_string());
        self.steps = 0;
        eprintln!("prepare_job: job {}", self.job.as_ref().unwrap());
    }

    fn step_env(&self, n: u32) -> serde_json::Value {
        let cmds = "/__w/_temp/_runner_file_commands";
        for f in [
            "set_env",
            "add_path",
            "set_output",
            "save_state",
            "step_summary",
        ] {
            std::fs::write(
                self.work
                    .join(format!("_temp/_runner_file_commands/{f}_{n}")),
                "",
            )
            .unwrap();
        }
        serde_json::json!({
            "CI": "true",
            "GITHUB_ACTIONS": "true",
            "GITHUB_REPOSITORY": "o/r",
            "GITHUB_EVENT_NAME": self.event.0,
            "GITHUB_WORKSPACE": WORKSPACE,
            "GITHUB_SERVER_URL": format!("file://{}", self.env.tmp.path().display()),
            "GITHUB_SHA": self.head(),
            "GITHUB_REF": "refs/heads/main",
            "GITHUB_RUN_ID": format!("{}", 1000 + n),
            "GITHUB_RUN_ATTEMPT": "1",
            "RUNNER_TEMP": "/__w/_temp",
            "RUNNER_TOOL_CACHE": "/__w/_tool",
            "RUNNER_OS": "Linux",
            "GITHUB_ENV": format!("{cmds}/set_env_{n}"),
            "GITHUB_PATH": format!("{cmds}/add_path_{n}"),
            "GITHUB_OUTPUT": format!("{cmds}/set_output_{n}"),
            "GITHUB_STATE": format!("{cmds}/save_state_{n}"),
            "GITHUB_STEP_SUMMARY": format!("{cmds}/step_summary_{n}"),
        })
    }

    fn head(&self) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.env.repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn run_step(&mut self, args: serde_json::Value, n: u32) -> StepOut {
        let input = serde_json::json!({
            "command": "run_script_step",
            "responseFile": self.work.join("_temp/response.json"),
            "state": {"job": self.job.as_ref().expect("prepare_job first")},
            "args": args,
        });
        let (code, stdout, stderr) = self.hooks(&input, &[]);
        let summary = std::fs::read_to_string(
            self.work
                .join(format!("_temp/_runner_file_commands/step_summary_{n}")),
        )
        .unwrap_or_default();
        eprintln!(
            "$ step {n}: exit {code}\n{}",
            stderr
                .lines()
                .filter(|l| l.starts_with("QUOD") || l.starts_with("quod") || l.starts_with("  "))
                .collect::<Vec<_>>()
                .join("\n")
        );
        StepOut {
            code,
            stdout,
            stderr,
            summary,
        }
    }

    /// A `run:` step with `script`, plus `extra` env (NAME=VALUE).
    fn script(&mut self, script: &str, extra: &[(&str, &str)]) -> StepOut {
        self.steps += 1;
        let n = self.steps;
        std::fs::write(
            self.work.join(format!("_temp/step-{n}.sh")),
            format!("{script}\n"),
        )
        .unwrap();
        let mut env = self.step_env(n);
        for (k, v) in extra {
            env[k] = serde_json::Value::String(v.to_string());
        }
        let args = serde_json::json!({
            "entryPoint": "bash",
            "entryPointArgs": ["--noprofile", "--norc", "-e", "-o", "pipefail", format!("/__w/_temp/step-{n}.sh")],
            "environmentVariables": env,
            "prependPath": [],
            "workingDirectory": WORKSPACE,
        });
        self.run_step(args, n)
    }

    /// actions/checkout of the repo's HEAD, served from the CAS.
    fn checkout(&mut self) -> StepOut {
        self.steps += 1;
        let n = self.steps;
        let mut env = self.step_env(n);
        for (k, v) in [
            ("INPUT_REPOSITORY", "o/r"),
            ("INPUT_REF", ""),
            ("INPUT_TOKEN", ""),
            ("INPUT_PATH", ""),
            ("INPUT_SUBMODULES", "false"),
            ("INPUT_LFS", "false"),
        ] {
            env[k] = serde_json::Value::String(v.into());
        }
        let args = serde_json::json!({
            "entryPoint": "/__e/node20/bin/node",
            "entryPointArgs": ["/__w/_actions/actions/checkout/abc/dist/index.js"],
            "environmentVariables": env,
            "prependPath": [],
            "workingDirectory": WORKSPACE,
        });
        self.run_step(args, n)
    }

    fn cleanup_job(&mut self) {
        let input = serde_json::json!({
            "command": "cleanup_job",
            "responseFile": null,
            "state": {"job": self.job.as_ref().unwrap()},
            "args": {},
        });
        let (code, _, stderr) = self.hooks(&input, &[]);
        assert_eq!(code, 0, "cleanup_job: {stderr}");
        self.job = None;
    }

    /// A whole job: checkout HEAD, run `step`, close.
    fn job(&mut self, step: &str, extra: &[(&str, &str)]) -> StepOut {
        self.prepare_job();
        let co = self.checkout();
        assert_eq!(co.code, 0, "checkout: {}", co.stderr);
        assert!(co.summary.contains("[checkout]"), "{}", co.summary);
        let out = self.script(step, extra);
        self.cleanup_job();
        out
    }

    /// The tree the last job's last step started from (kept by
    /// QUOD_KEEP_JOBS): what that step's manifest was matched against.
    fn tree(&self) -> quod_cas::Tree {
        let jobs = self.state.join("jobs");
        let mut ids: Vec<String> = std::fs::read_dir(&jobs)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        ids.sort();
        let job = jobs.join(ids.last().expect("a kept job"));
        let mut steps: Vec<PathBuf> = std::fs::read_dir(&job)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.join("tree.json").is_file())
            .collect();
        steps.sort();
        let bytes = std::fs::read(
            steps
                .last()
                .expect("a step with its tree")
                .join("tree.json"),
        )
        .unwrap();
        let entries: Vec<Entry> = serde_json::from_slice(&bytes).unwrap();
        quod_cas::Tree::from_leaves(entries)
    }

    /// The job's scope: a push to main or a pull request from the
    /// repository itself, both `trusted`.
    fn scope(&self) -> PathBuf {
        self.state.join("scopes/o_r/trusted")
    }

    fn ledger(&self) -> Ledger {
        Ledger::open(&self.scope().join("ledger.sqlite")).unwrap()
    }
}

#[test]
fn hooks_acceptance() {
    if !enabled() {
        return;
    }
    let spike = std::env::var("QUOD_SPIKE_REPO").ok();
    let env = match &spike {
        Some(src) => Env::from_copy(Path::new(src), Some(quod_key::Toolchain::Go)),
        None => Env::new(),
    };
    // The local "GitHub": file://<tmp>/o/r is the repo.
    std::fs::create_dir_all(env.tmp.path().join("o")).unwrap();
    std::os::unix::fs::symlink(&env.repo, env.tmp.path().join("o/r")).unwrap();
    let step = std::env::var("QUOD_STEP").unwrap_or_else(|_| STEP_DEFAULT.into());
    let step = step.as_str();
    let mut r = Runner::new(env);

    // A1: first job executes and records one manifest and result.
    let a1 = r.job(step, &[]);
    assert!(a1.executed() && a1.code == 0, "A1: {}", a1.stderr);
    assert!(
        a1.summary.contains("would replay: no, first run"),
        "A1 summary: {}",
        a1.summary
    );
    let step_keys = keys(&r);
    assert_eq!(step_keys.len(), 1, "A1: one step key: {step_keys:?}");
    let key = step_keys[0];
    assert_eq!(
        r.ledger().manifest_count(&key).unwrap(),
        1,
        "A1: one manifest"
    );
    // N3: the key names the withheld variables under the v2 line (they are
    // set to a placeholder, not removed as under v1).
    let key_json = r.ledger().step_key_json(&key).unwrap().unwrap();
    assert!(
        key_json.contains("quod:withheld v3 "),
        "A1: the key's withheld line is v2: {key_json}"
    );
    println!("A1 ok: EXECUTED through the hooks, one manifest");

    // A2: the same tree again. Observe mode executes and says it would
    // replay; replay mode replays byte for byte.
    let a2 = r.job(step, &[]);
    assert!(a2.executed(), "A2 observe: {}", a2.stderr);
    assert!(
        a2.summary.contains("would replay: yes"),
        "A2 observe summary: {}",
        a2.summary
    );
    r.mode = "replay";
    let a2r = r.job(step, &[]);
    assert!(
        a2r.replayed() && a2r.summary.contains("[replayed]"),
        "A2 replay: {}",
        a2r.stderr
    );
    // The replayed result is the newest one recorded for the manifest:
    // the observe-mode run just before.
    assert_eq!(a2r.code, a2.code, "A2: exit code");
    assert_eq!(a2r.stdout, a2.stdout, "A2: stdout bytes");
    println!(
        "A2 ok: observe reports would-replay, replay mode REPLAYED {} stdout bytes identical",
        a2r.stdout.len()
    );

    let manifest = r.ledger().candidates(&key, 1).unwrap().remove(0);
    assert!(
        !manifest.volatile.is_empty(),
        "A2: volatile names recorded: {:?}",
        manifest.volatile
    );
    assert!(manifest.volatile.iter().any(|v| v == "GITHUB_RUN_ID"));
    let recorded: BTreeSet<&str> = manifest.entries.iter().map(|e| e.p.as_str()).collect();

    // A3: a commit touching only README.md replays (through a real CAS checkout).
    assert!(!recorded.contains("README.md"), "A3 precondition");
    let readme = std::fs::read_to_string(r.env.repo.join("README.md")).unwrap_or_default();
    r.env.write("README.md", &format!("{readme}\nA3 edit\n"));
    r.env.commit("A3: README only");
    let a3 = r.job(step, &[]);
    // A step that reads the checkout's .git (go's VCS probing reads HEAD)
    // depends on the commit itself, so a new commit is a miss there, as it
    // must be; README.md must not be what differs.
    let a3_rest = outside_git(&manifest, &r.tree());
    assert!(
        a3.replayed() || a3_rest.is_ok(),
        "A3: {a3_rest:?}: {}",
        a3.stderr
    );
    println!(
        "A3 ok: README-only commit {}",
        if a3.replayed() {
            "REPLAYED"
        } else {
            "missed on the commit's .git only, never on README.md"
        }
    );

    // A4: a commit changing a Go file the tests read misses and names it.
    let go_file = manifest
        .entries
        .iter()
        .find(|e| {
            e.k == Kind::File
                && e.p.ends_with(".go")
                && !e.p.starts_with(".quod/")
                && !e.p.ends_with("_test.go")
        })
        .expect("A4: a non-test .go file in the manifest")
        .p
        .clone();
    let src = std::fs::read_to_string(r.env.repo.join(&go_file)).unwrap();
    r.env.write(&go_file, &format!("{src}\n// A4 edit\n"));
    r.env.commit("A4: touch a Go file");
    let a4 = r.job(step, &[]);
    assert!(a4.executed(), "A4: {}", a4.stderr);
    let a4_mm = outside_git(&manifest, &r.tree());
    assert!(
        a4.summary.contains(&format!("differed at {go_file}"))
            || a4_mm.as_ref().is_err_and(|m| m.path == go_file),
        "A4 must name {go_file}: {a4_mm:?}: {}",
        a4.summary
    );
    println!("A4 ok: MISS, the first mismatch outside .git is {go_file}");

    // A5: adding a file to a listed directory misses and names the listing.
    let dir = Path::new(&go_file)
        .parent()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let newest = r.ledger().candidates(&key, 1).unwrap().remove(0);
    assert!(
        newest
            .entries
            .iter()
            .any(|e| e.p == dir && e.k == Kind::Dir),
        "A5 precondition: {dir} listed"
    );
    let added = child_path(&dir, "zz_quod_a5.txt");
    r.env.write(&added, "new\n");
    r.env.commit("A5: add a file");
    let a5 = r.job(step, &[]);
    assert!(a5.executed(), "A5: {}", a5.stderr);
    let a5_mm = outside_git(&newest, &r.tree());
    assert!(
        (a5.summary.contains(&format!("differed at {dir} "))
            && a5.summary.contains("directory listing changed"))
            || a5_mm
                .as_ref()
                .is_err_and(|m| m.path == dir && m.expected.k == Kind::Dir),
        "A5: {a5_mm:?}: {}",
        a5.summary
    );
    println!("A5 ok: MISS, the first mismatch outside .git is the listing of {dir}");

    // A6: a changed (keyed) env var is a different key with no manifests.
    let a6 = r.job(step, &[("QUOD_A6", "1")]);
    assert!(
        a6.executed() && a6.summary.contains("first run of this step"),
        "A6: {}",
        a6.summary
    );
    assert_eq!(keys(&r).len(), 2, "A6: a second key");
    // A volatile one is not: same key, and it replays.
    let a6v = r.job(step, &[("GITHUB_RUN_ID", "999999")]);
    assert!(a6v.replayed(), "A6 volatile: {}", a6v.stderr);
    println!("A6 ok: keyed env is a new key; volatile env is not");

    // C1: who the step runs as is part of the key: the same step as
    // another user never replays the first user's result. (As root,
    // cli/cli's permission tests fail, and a failure is never recorded.)
    let before = keys(&r).len();
    r.user = "--user 1234:1234";
    let c1 = r.job(step, &[]);
    assert!(
        c1.executed() && !c1.replayed() && !c1.summary.contains("would replay: yes"),
        "C1 uid 1234: {}",
        c1.summary
    );
    r.user = "";
    let c1r = r.job(step, &[]);
    assert!(
        c1r.executed() && !c1r.replayed() && !c1r.summary.contains("would replay: yes"),
        "C1 root: {}",
        c1r.summary
    );
    let recorded_now = keys(&r).len();
    assert!(
        recorded_now > before && recorded_now <= before + 2,
        "C1: a new key per identity that recorded ({before} -> {recorded_now})"
    );
    r.user = "--user 1001:1001";
    println!(
        "C1 ok: the same step as uid 1234 and as root never replays uid 1001's result ({} new keys recorded)",
        recorded_now - before
    );

    // C2: an offline result that is not the step's own is never recorded:
    // a failure, and a step that reached for the network (and did not
    // fail, `|| true`). Both run again with the runner's environment.
    let recorded = |r: &Runner| -> usize {
        let out = Command::new("sqlite3")
            .arg(r.scope().join("ledger.sqlite"))
            .arg("SELECT count(*) FROM results")
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    };
    let n0 = recorded(&r);
    let fail = r.job("echo c2; exit 3", &[]);
    assert_eq!(fail.code, 3, "C2: the job still fails: {}", fail.stderr);
    assert!(
        fail.summary.contains("exited 3 offline") && fail.summary.contains("tainted"),
        "C2 failure: {}",
        fail.summary
    );
    let net = r.job(
        "curl -sS -m 3 http://192.0.2.1/ >/dev/null 2>&1 || true; echo done",
        &[],
    );
    assert_eq!(net.code, 0);
    assert!(
        net.summary.contains("tried to reach the network offline")
            && net.summary.contains("tainted"),
        "C2 network: {}",
        net.summary
    );
    let dns = r.job("getent hosts example.com || true", &[]);
    assert!(
        dns.summary.contains("tried to reach the network offline"),
        "C2 dns: {}",
        dns.summary
    );
    assert_eq!(recorded(&r), n0, "C2: nothing recorded");
    println!(
        "C2 ok: a failure, a `curl || true` and a DNS lookup offline are run again, never recorded"
    );

    // C3 (MAJOR 1): a secret under a name the sign list misses is still
    // caught by its value shape. A lab probe confirmed a mapped secret
    // reaches the step env and the protocol gives the hook no mask list, so
    // the value itself is the signal. The step prints the value; the stored
    // output (read back by replaying) must be masked, never the secret.
    let secret = "aB3kZ9qR7tN2wX5vY8uJ1mD4cF6hL0pS2eG7d";
    r.mode = "observe";
    let c3o = r.job(
        "printf 'v=%s' \"$RELEASE_UPLOAD\"",
        &[("RELEASE_UPLOAD", secret)],
    );
    assert!(
        c3o.executed(),
        "C3 observe (records the first run): {}",
        c3o.summary
    );
    assert!(
        String::from_utf8_lossy(&c3o.stdout).contains(secret),
        "C3: the live step really did print the value"
    );
    r.mode = "replay";
    let c3r = r.job(
        "printf 'v=%s' \"$RELEASE_UPLOAD\"",
        &[("RELEASE_UPLOAD", secret)],
    );
    assert!(c3r.replayed(), "C3 replay: {}", c3r.stderr);
    let stored = String::from_utf8_lossy(&c3r.stdout);
    assert!(
        !stored.contains(secret),
        "C3: the stored output leaked the mis-named secret: {stored:?}"
    );
    assert!(
        stored.contains("***"),
        "C3: the stored output was not masked: {stored:?}"
    );
    println!(
        "C3 ok: a secret under a plain name is caught by value shape, masked in the stored output"
    );

    // A7: every recorded input, mutated, misses. Zero false hits.
    let cas = Cas::open(r.state.join("cas")).unwrap();
    let tree = r.tree();
    let cands = r
        .ledger()
        .candidates(&key, quod_ledger::CANDIDATES)
        .unwrap();
    let base = first_match(&cands, &tree).expect("A7: the current tree must match before mutating");
    let m = &cands[base];
    let t = Instant::now();
    let mut counts = [0usize; 4];
    let mut false_hits = Vec::new();
    for e in &m.entries {
        let (slot, mutated) = match e.k {
            Kind::File => {
                let mut bytes = cas.get(&e.h.unwrap()).unwrap().unwrap();
                match bytes.first_mut() {
                    Some(b) => *b ^= 0x01,
                    None => bytes.push(b'x'),
                }
                (
                    0,
                    tree.with_leaf_changed(Entry::file(
                        e.p.clone(),
                        Hash::of(&bytes),
                        e.m.unwrap(),
                    )),
                )
            }
            Kind::Dir => (
                1,
                tree.with_outputs(&[Entry::file(
                    child_path(&e.p, "zz_quod_a7"),
                    Hash::of(b"x"),
                    0o644,
                )]),
            ),
            Kind::Absent => (
                2,
                tree.with_outputs(&[Entry::file(e.p.clone(), Hash::of(b"x"), 0o644)]),
            ),
            Kind::Symlink => (
                3,
                tree.with_leaf_changed(Entry::symlink(e.p.clone(), "zz_quod_a7")),
            ),
            Kind::Deleted => unreachable!("manifests never hold x"),
        };
        counts[slot] += 1;
        if first_match(&cands, &mutated).is_some() {
            false_hits.push(format!("{} {:?}", e.p, e.k));
        }
    }
    println!(
        "A7: mutated {} f, {} d, {} a, {} l entries in {:.1}s: {} false hits",
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        t.elapsed().as_secs_f64(),
        false_hits.len()
    );
    assert!(false_hits.is_empty(), "A7 false hits: {false_hits:?}");

    // A7b: unread paths mutated on disk (and committed, so the checkout
    // serves them) must replay, and a fresh execution must agree.
    let samples: usize = std::env::var("QUOD_A7B_SAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let recorded: BTreeSet<&str> = m.entries.iter().map(|e| e.p.as_str()).collect();
    let unread: Vec<&Entry> = tree
        .iter()
        .filter(|e| {
            // Not .git: the checkout serves its own, and the files at those
            // paths on disk are the test repository's.
            e.k == Kind::File
                && !recorded.contains(e.p.as_str())
                && !e.p.starts_with(".quod/")
                && !e.p.starts_with(".git/")
        })
        .filter(|e| r.env.repo.join(&e.p).is_file())
        .collect();
    let stride = (unread.len() / samples.max(1)).max(1);
    let picked: Vec<String> = unread
        .iter()
        .step_by(stride)
        .take(samples)
        .map(|e| e.p.clone())
        .collect();
    for (i, p) in picked.iter().enumerate() {
        std::fs::write(r.env.repo.join(p), b"\x00QUOD_A7B corrupted\n").unwrap();
        r.env.commit(&format!("A7b {i}: corrupt {p}"));
        let hit = r.job(step, &[]);
        let rest = outside_git(m, &r.tree());
        assert!(
            hit.replayed() || rest.is_ok(),
            "A7b: mutating unread {p} should replay, or miss only on the commit's .git: {rest:?}: {}",
            hit.stderr
        );
        let fresh = r.job(step, &[("QUOD_A7B", &i.to_string())]);
        assert!(fresh.executed(), "A7b: {}", fresh.stderr);
        let fresh_key = newest_key(&r);
        let fresh_m = r.ledger().candidates(&fresh_key, 1).unwrap().remove(0);
        let fp: BTreeSet<&str> = fresh_m.entries.iter().map(|e| e.p.as_str()).collect();
        assert!(
            !fp.contains(p.as_str()),
            "A7b CRITICAL: the fresh execution read {p}, which the replayed manifest never recorded"
        );
        assert_eq!(fresh.code, hit.code, "A7b: {p} changed the exit code");
        assert_eq!(
            normalize(&fresh.stdout),
            normalize(&hit.stdout),
            "A7b: {p} changed the output"
        );
        let only_fresh: Vec<&str> = fp.difference(&recorded).copied().collect();
        let only_hit: Vec<&str> = recorded.difference(&fp).copied().collect();
        println!(
            "A7b ok: unread {p} mutated, replay agrees with a fresh execution ({} paths; {} only fresh, {} only replayed)",
            fp.len(),
            only_fresh.len(),
            only_hit.len()
        );
    }
    // A7b beyond the repository: what the step cannot see must not change
    // its result. A file in the runner's own RUNNER_TEMP (the step gets a
    // fresh one), and the warm GOCACHE (wiped: a fresh run with an empty
    // cache must agree with the recorded result).
    std::fs::write(r.work.join("_temp/zz_quod_a7b"), "planted\n").unwrap();
    let hit = r.job(step, &[]);
    assert!(hit.replayed(), "A7b RUNNER_TEMP: {}", hit.stderr);
    let fresh = r.job(step, &[("QUOD_A7B", "temp")]);
    assert!(
        fresh.executed() && fresh.code == hit.code,
        "A7b RUNNER_TEMP: {}",
        fresh.stderr
    );
    assert_eq!(
        normalize(&fresh.stdout),
        normalize(&hit.stdout),
        "A7b RUNNER_TEMP"
    );
    std::fs::remove_file(r.work.join("_temp/zz_quod_a7b")).unwrap();
    println!("A7b ok: a file in the runner's RUNNER_TEMP changes nothing");
    let warm = r.scope().join("warm");
    if warm.exists() {
        quod_cas::remove_any(&warm).unwrap();
    }
    let hit = r.job(step, &[]);
    assert!(hit.replayed(), "A7b GOCACHE: {}", hit.stderr);
    let fresh = r.job(step, &[("QUOD_A7B", "gocache")]);
    assert!(
        fresh.executed() && fresh.code == hit.code,
        "A7b GOCACHE: {}",
        fresh.stderr
    );
    assert_eq!(
        normalize(&fresh.stdout),
        normalize(&hit.stdout),
        "A7b GOCACHE"
    );
    println!("A7b ok: an empty warm GOCACHE gives the recorded result");

    // After A7 and A7b, which mutate the main step's recorded tree: these
    // run other steps, and on a real module (cli/cli) `go list` can add to
    // the persisted module cache, which would move the tree A7 starts from.
    // B1: which withheld or placeholder variables a job has is in the key.
    // A step that branches on a variable's presence (a test that skips
    // without GITHUB_TOKEN; an action that asks for an OIDC token only when
    // ACTIONS_ID_TOKEN_REQUEST_URL is set) must not replay a result recorded
    // without it for a job that has it. The scripts build the names
    // indirectly, as code that reads them generically does, so no name scan
    // sees them: only the key can tell the jobs apart.
    r.mode = "replay";
    let token = r#"v=GITHUB_; eval "t=\${${v}TOKEN:-}"; if [ -z "$t" ]; then echo "SKIP: no token"; else echo "ran with a token"; fi"#;
    let oidc = r#"v=ACTIONS_ID_TOKEN_; eval "u=\${${v}REQUEST_URL:-}"; if [ -n "$u" ]; then echo oidc; else echo "no oidc"; fi"#;
    let gh_token = ("GITHUB_TOKEN", "ghs_0123456789abcdefghijABCDEFGHIJ0123");
    let id_url = (
        "ACTIONS_ID_TOKEN_REQUEST_URL",
        "https://pipelines.actions.githubusercontent.com/oidc",
    );
    for (script, var, what) in [
        (token, gh_token, "GITHUB_TOKEN"),
        (oidc, id_url, "ACTIONS_ID_TOKEN_REQUEST_URL"),
    ] {
        let without = r.job(script, &[]);
        assert!(
            without.executed(),
            "B1 {what} absent, first: {}",
            without.stderr
        );
        assert!(
            r.job(script, &[]).replayed(),
            "B1 {what} absent, again: replays its own result"
        );
        let with = r.job(script, &[var]);
        assert!(
            with.executed() && !with.replayed(),
            "B1: a job with {what} must not replay the result recorded without it: {}",
            with.stderr
        );
        assert!(
            r.job(script, &[var]).replayed(),
            "B1 {what} present, again: replays its own result"
        );
    }
    // Both variables reach the step as placeholders, so a job that has one
    // gets the result GitHub would give it: the token test runs, the OIDC
    // branch is taken. (Their real values never reach a keyed step.)
    let with = r.job(token, &[gh_token]);
    assert_eq!(String::from_utf8_lossy(&with.stdout), "ran with a token\n");
    let without = r.job(token, &[]);
    assert_eq!(String::from_utf8_lossy(&without.stdout), "SKIP: no token\n");
    let with = r.job(oidc, &[id_url]);
    assert_eq!(String::from_utf8_lossy(&with.stdout), "oidc\n");
    println!(
        "B1 ok: a job with GITHUB_TOKEN or ACTIONS_ID_TOKEN_REQUEST_URL never replays a result recorded without it"
    );

    // N4: a withheld variable a step reaches by a name no scan sees (so it
    // runs keyed, with the placeholder) and puts in its result (stdout,
    // GITHUB_OUTPUT, a file) gave a result GitHub never would. Such a run is
    // never recorded: the step runs again with the runner's environment, and
    // the job gets the real value.
    let head = r.head();
    for (what, script) in [
        ("stdout", r#"v=GITHUB_; eval "echo \${${v}SHA}""#),
        (
            "GITHUB_OUTPUT",
            r#"v=GITHUB_; eval "echo sha=\${${v}SHA}" >> "$GITHUB_OUTPUT""#,
        ),
        (
            "an output file",
            r#"v=GITHUB_; eval "echo \${${v}SHA}" > quod-n4.txt"#,
        ),
    ] {
        let first = r.job(script, &[]);
        assert!(
            first
                .summary
                .contains("holds a withheld variable's placeholder")
                && first.summary.contains("tainted"),
            "N4 {what}: {}\n{}",
            first.summary,
            first.stderr
        );
        let again = r.job(script, &[]);
        assert!(
            !again.replayed() && again.summary.contains("placeholder"),
            "N4 {what}: never recorded: {}",
            again.summary
        );
        match what {
            "stdout" => assert!(
                String::from_utf8_lossy(&again.stdout).contains(&head),
                "N4: the job gets the real value"
            ),
            "GITHUB_OUTPUT" => {
                let out = std::fs::read_to_string(
                    r.work.join("_temp/_runner_file_commands/set_output_2"),
                )
                .unwrap();
                assert!(
                    out.contains(&format!("sha={head}")) && !out.contains("quod-withheld"),
                    "N4: GITHUB_OUTPUT holds the real value: {out:?}"
                );
            }
            _ => {}
        }
    }
    println!(
        "N4 ok: a result holding a withheld variable's placeholder is never recorded; the step runs again with the real value"
    );

    // M1: the scope's warm build cache is bound only into steps that run
    // go, and what a step leaves there must be Go's cache layout: a step
    // that stashes a file in it is not recorded, and the cache is wiped.
    let warm_root = r.scope().join("warm");
    let find = |name: &str| -> bool {
        let mut stack = vec![warm_root.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                if e.file_name() == name {
                    return true;
                }
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(e.path());
                }
            }
        }
        false
    };
    let nogo = r.job(r#"echo planted > "$GOCACHE/quod-nogo""#, &[]);
    assert!(nogo.executed(), "M1 no go: {}", nogo.stderr);
    assert!(
        !find("quod-nogo"),
        "M1: a step that does not run go never sees the warm cache"
    );
    let stash = r#"go list ./... >/dev/null && echo planted > "$GOCACHE/quod-stash""#;
    let m1 = r.job(stash, &[]);
    assert!(
        m1.executed() && m1.summary.contains("not Go's cache layout"),
        "M1 stash: {}",
        m1.summary
    );
    assert!(!find("quod-stash"), "M1: the warm cache was wiped");
    assert!(!r.job(stash, &[]).replayed(), "M1: never recorded");
    // `go run` caches its executable as Go 1.24+ does, `<hh>/<id>-d/<exe>`
    // with the `-d` a directory: Go's layout, so recorded and replayed,
    // and the cache kept.
    let gorun = r#"mkdir -p /tmp/hello && printf 'package main\n\nimport "fmt"\n\nfunc main() { fmt.Println("hello from go run") }\n' > /tmp/hello/main.go && go run /tmp/hello/main.go"#;
    let first = r.job(gorun, &[]);
    assert!(
        first.executed() && first.code == 0 && !first.summary.contains("not Go's cache layout"),
        "M1 go run: {}\n{}",
        first.summary,
        first.stderr
    );
    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        "hello from go run\n"
    );
    let mut exe = None;
    let mut stack = vec![warm_root.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if e.file_name().to_string_lossy().ends_with("-d")
                    && e.path().join("main").is_file()
                {
                    exe = Some(e.path());
                }
                stack.push(e.path());
            }
        }
    }
    assert!(
        exe.is_some(),
        "M1 go run: the executable is cached in a -d directory"
    );
    let again = r.job(gorun, &[]);
    assert!(again.replayed(), "M1 go run replays: {}", again.stderr);
    assert_eq!(
        String::from_utf8_lossy(&again.stdout),
        "hello from go run\n"
    );
    println!(
        "M1 ok: only go steps see the warm build cache; a stash in it is not recorded and wipes it; go run's cached executable is Go's layout"
    );

    // D1: a hit whose fresh run, on the same inputs, is not the step's own
    // result (here it fails offline because of a file planted in the warm
    // build cache, an input the key cannot see) is a divergence: the key is
    // quarantined and the run row says `diverged`, so `quod status`
    // counts it.
    r.mode = "observe";
    let flip =
        r#"go list ./... >/dev/null; if [ -e "$GOCACHE/quod-flip" ]; then exit 3; fi; echo stable"#;
    let d1 = r.job(flip, &[]);
    assert!(d1.executed() && d1.code == 0, "D1 first: {}", d1.stderr);
    let mut planted = None;
    let mut stack = vec![warm_root.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if e.file_name() == "gocache" {
                    planted = Some(e.path().join("quod-flip"));
                }
                stack.push(e.path());
            }
        }
    }
    let planted = planted.expect("D1: the go step had a warm build cache");
    std::fs::write(&planted, "flip").unwrap();
    let d2 = r.job(flip, &[]);
    std::fs::remove_file(&planted).unwrap();
    assert!(
        d2.stderr.contains("DIVERGENCE") && d2.stderr.contains("exited 3 offline"),
        "D1 second run diverges: {}",
        d2.stderr
    );
    let sql = |q: &str| -> String {
        let out = Command::new("sqlite3")
            .arg(r.scope().join("ledger.sqlite"))
            .arg(q)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert!(
        sql("SELECT count(*) FROM runs WHERE outcome = 'diverged'")
            .parse::<u32>()
            .unwrap()
            >= 1,
        "D1: a diverged run row"
    );
    assert!(
        sql("SELECT count(*) FROM trust WHERE state = 'quarantined'")
            .parse::<u32>()
            .unwrap()
            >= 1
    );
    println!(
        "D1 ok: a hit that behaves otherwise on the same inputs is quarantined, with a diverged run row"
    );

    // W1 (MAJOR 3): only a writer, a push, dispatch or schedule on the
    // default branch, writes the scope's warm build cache and persisted
    // workspace. A pull request from the repository itself (a Dependabot
    // bump) shares the trusted ledger but works on a copy of both that goes
    // with the job: what its go steps put in the cache, an entry the
    // default branch uses rewritten included, never reaches it; a later go
    // step of the same job, which reads what an earlier one wrote, is not
    // recorded (it may still replay a clean run); and what it leaves in
    // the module cache does not persist.
    r.mode = "replay";
    // Path to content hash (never the bytes: on cli/cli the warm cache is
    // gigabytes).
    let files_under = |root: &Path| -> BTreeMap<PathBuf, quod_cas::Hash> {
        let mut out = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let t = e.file_type().unwrap();
                if t.is_dir() {
                    stack.push(e.path());
                } else {
                    let bytes = std::fs::read(e.path()).unwrap_or_default();
                    out.insert(e.path(), quod_cas::Hash::of(&bytes));
                }
            }
        }
        out
    };
    let warm_before = files_under(&warm_root);
    assert!(
        warm_before.keys().any(|p| p.ends_with("main")),
        "W1: the default branch's go run executable is in the warm cache"
    );
    let workspaces = r.scope().join("workspaces");
    let slots_before = files_under(&workspaces);
    assert!(!slots_before.is_empty(), "W1: a persisted workspace");
    let writer_event = std::mem::replace(&mut r.event, same_repo_pull_request());
    r.prepare_job();
    let co = r.checkout();
    assert_eq!(co.code, 0, "W1 checkout: {}", co.stderr);
    // Its own build, and a rewrite of the cached executable main replays.
    let poison = r#"mkdir -p /tmp/pr && printf 'package main\n\nimport "fmt"\n\nfunc main() { fmt.Println("hello from a pull request") }\n' > /tmp/pr/main.go && go run /tmp/pr/main.go && for f in "$GOCACHE"/*/*-d/main; do printf 'poisoned' > "$f"; done"#;
    let w1 = r.script(poison, &[]);
    assert!(
        w1.code == 0 && !w1.summary.contains("not Go's cache layout"),
        "W1 pull request build: {}\n{}",
        w1.summary,
        w1.stderr
    );
    let now = files_under(&warm_root);
    let changed: Vec<String> = now
        .iter()
        .filter(|(p, b)| warm_before.get(*p) != Some(*b))
        .map(|(p, _)| p.display().to_string())
        .collect();
    assert!(
        changed.is_empty() && now.len() == warm_before.len(),
        "W1: the pull request's go step changed the scope's warm build cache: {changed:?}"
    );
    // A go step main recorded: still replays.
    let w2 = r.script(gorun, &[]);
    assert!(
        w2.replayed(),
        "W1: a pull request replays a clean run: {}",
        w2.stderr
    );
    // A go step nobody recorded: runs over this job's writes, not recorded.
    let fresh = r#"go list ./... >/dev/null && echo after-the-pull-request-build"#;
    let w3 = r.script(fresh, &[]);
    assert!(
        w3.executed()
            && w3
                .summary
                .contains("not recorded: an earlier step of this job wrote"),
        "W1: {}",
        w3.summary
    );
    let w4 = r.script(r#"echo poison > "$GOMODCACHE/quod-pull-request""#, &[]);
    assert_eq!(w4.code, 0, "W1 module cache: {}", w4.stderr);
    r.cleanup_job();
    r.event = writer_event;
    assert_eq!(
        files_under(&warm_root),
        warm_before,
        "W1: the pull request changed nothing in the scope's warm build cache"
    );
    assert_eq!(
        files_under(&workspaces),
        slots_before,
        "W1: the pull request's workspace did not persist"
    );
    let after = r.job(fresh, &[]);
    assert!(
        after.executed() && after.summary.contains("first run of this step"),
        "W1: the pull request recorded nothing for the default branch: {}",
        after.summary
    );
    let clean = r.job(
        r#"cat "$GOMODCACHE/quod-pull-request" 2>/dev/null || echo clean"#,
        &[],
    );
    assert_eq!(String::from_utf8_lossy(&clean.stdout), "clean\n");
    println!(
        "W1 ok: a same-repository pull request writes only its own copy of the warm cache and workspace; its go step over its own writes is not recorded"
    );

    // W2 (N2): taking something away from the cache is a write to the
    // job's layer too. A pull request that empties the cache (`go clean
    // -cache`: whiteouts), removes and remakes one of its directories (an
    // opaque directory) or changes an entry's mode makes every later go step
    // of its job read a cache the default branch's runs never had, so that
    // step is not recorded (it could otherwise record, prove or quarantine
    // the default branch's keys with what a cold or altered cache gives).
    let later = r#"go list ./... >/dev/null && echo after-the-cache-was-altered"#;
    for (what, alter) in [
        (
            "go clean -cache",
            r#"go list ./... >/dev/null && go clean -cache"#,
        ),
        (
            "a directory removed and made again",
            r#"go list ./... >/dev/null && d=$(ls -d "$GOCACHE"/[0-9a-f][0-9a-f] | head -n 1) && rm -rf "$d" && mkdir "$d""#,
        ),
        (
            "a mode changed",
            r#"go list ./... >/dev/null && for f in "$GOCACHE"/*/*-a; do chmod o-r "$f"; break; done"#,
        ),
    ] {
        r.event = same_repo_pull_request();
        r.prepare_job();
        let co = r.checkout();
        assert_eq!(co.code, 0, "W2 checkout: {}", co.stderr);
        let first = r.script(alter, &[]);
        assert_eq!(first.code, 0, "W2 {what}: {}", first.stderr);
        let after = r.script(later, &[]);
        assert!(
            after.executed()
                && after
                    .summary
                    .contains("not recorded: an earlier step of this job wrote"),
            "W2 {what}: {}\n{}",
            after.summary,
            after.stderr
        );
        r.cleanup_job();
    }
    r.event = push_to_main();
    let fresh_run = r.job(later, &[]);
    assert!(
        fresh_run.summary.contains("first run of this step"),
        "W2: no pull request recorded the later step: {}",
        fresh_run.summary
    );
    println!(
        "W2 ok: a pull request that empties, remakes or re-modes the warm cache leaves its later go steps unrecorded"
    );

    // T1 (the trust layer, as `quod run` has it): a result seen once
    // never replays; two identical executions prove it; an audit
    // re-executes a would-be hit and compares instead of replaying.
    r.mode = "replay";
    r.proofs = 2;
    let stable = "echo trust-$((40 + 2))";
    let t1 = r.job(stable, &[]);
    assert!(
        t1.executed()
            && t1
                .summary
                .contains("first run of this step (1 of 2 identical proofs)"),
        "T1 first: {}",
        t1.summary
    );
    let t1 = r.job(stable, &[]);
    assert!(
        t1.executed()
            && t1.summary.contains("not yet (2 of 2 identical proofs)")
            && t1.summary.contains("replayable from the next matching run"),
        "T1 second: {}",
        t1.summary
    );
    let t1 = r.job(stable, &[]);
    assert!(t1.replayed(), "T1 third replays: {}", t1.stderr);
    assert_eq!(String::from_utf8_lossy(&t1.stdout), "trust-42\n");
    r.audit = 1.0;
    let t1 = r.job(stable, &[]);
    assert!(
        t1.executed() && t1.summary.contains("audit: agrees with run"),
        "T1 audit: {}",
        t1.summary
    );
    r.audit = 0.0;
    println!("T1 ok: once never replays, twice proves, the third replays; an audit agrees");

    // T2: a step whose result differs on the same inputs never replays:
    // its second execution diverges from the first, which quarantines the
    // key, and it executes from then on.
    let noisy = "date +%s%N";
    let mut outs = Vec::new();
    for _ in 0..3 {
        let o = r.job(noisy, &[]);
        assert!(o.executed() && !o.replayed(), "T2: {}", o.stderr);
        outs.push(o);
    }
    assert!(
        outs[1].summary.contains("audit DIVERGED") && outs[1].stderr.contains("DIVERGENCE"),
        "T2 second run diverges: {}",
        outs[1].stderr
    );
    assert!(
        outs[2].summary.contains("quarantined"),
        "T2 third run is quarantined: {}",
        outs[2].summary
    );
    r.proofs = 1;
    println!("T2 ok: a nondeterministic step diverges, is quarantined, never replays");

    println!(
        "hooks acceptance: A1-A7, A7b, C1-C3, B1, N4, M1, D1, W1-W2 and T1-T2 pass{}",
        if spike.is_some() {
            " on the spike repo"
        } else {
            ""
        }
    );
}

/// Does `m` match `tree` everywhere outside `.git` (the commit's own
/// identity)? `Err` holds the first mismatch elsewhere.
fn outside_git(
    m: &quod_manifest::Manifest,
    tree: &quod_cas::Tree,
) -> Result<(), Box<quod_manifest::Mismatch>> {
    let mut rest = m.clone();
    rest.entries
        .retain(|e| e.p != ".git" && !e.p.starts_with(".git/"));
    quod_manifest::matches(&rest, tree)
}

fn image() -> Option<String> {
    std::env::var("QUOD_HOOKS_IMAGE").ok()
}

fn keys(r: &Runner) -> Vec<Hash> {
    let out = Command::new("sqlite3")
        .arg(r.scope().join("ledger.sqlite"))
        .arg("SELECT DISTINCT step_key FROM manifests ORDER BY step_key")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.parse().unwrap())
        .collect()
}

fn newest_key(r: &Runner) -> Hash {
    let out = Command::new("sqlite3")
        .arg(r.scope().join("ledger.sqlite"))
        .arg("SELECT step_key FROM manifests ORDER BY created_at DESC, rowid DESC LIMIT 1")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

fn normalize(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(|l| match l.rsplit_once('\t') {
            Some((head, tail))
                if tail.ends_with('s') && tail[..tail.len() - 1].parse::<f64>().is_ok() =>
            {
                format!("{head}\t<dur>")
            }
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The observer's request counts for the last workspace mount (its event
/// in events.jsonl): lookups, opens, MiB read.
fn last_mount(r: &Runner) -> (u64, u64, u64) {
    let events = std::fs::read_to_string(r.state.join("events.jsonl")).unwrap_or_default();
    let ev: serde_json::Value = events
        .lines()
        .rev()
        .filter(|l| l.contains("\"mounted_ms\""))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .next()
        .unwrap_or_default();
    let f = |k: &str| {
        ev["fields"][k]
            .as_u64()
            .or_else(|| ev[k].as_u64())
            .unwrap_or(0)
    };
    (f("lookup"), f("open"), f("read_mb"))
}

/// G1-G4: go build (which stamps VCS information by running `git status`)
/// and go test through the hooks over the served index (DECISIONS.md,
/// exp/git-index). With QUOD_GIT_INDEX_MEASURE=1 it only prints what it
/// sees, for the same numbers on a branch without the served index.
#[test]
fn hooks_git_index() {
    if !enabled() {
        return;
    }
    let measure = std::env::var("QUOD_GIT_INDEX_MEASURE").is_ok_and(|v| v == "1");
    let spike = std::env::var("QUOD_SPIKE_REPO").ok();
    let env = match &spike {
        Some(src) => Env::from_copy(Path::new(src), Some(quod_key::Toolchain::Go)),
        None => Env::new(),
    };
    // A branch to push to, as on GitHub (the suite's copy may be detached).
    git(&env.repo, &["checkout", "-q", "-B", "main"]);
    if spike.is_none() {
        env.write("cmd/x/main.go", "package main\n\nfunc main() {}\n");
        env.write(".github/workflows/ci.yml", "name: ci\n");
        env.commit("a main package and a workflow");
    }
    std::fs::create_dir_all(env.tmp.path().join("o")).unwrap();
    std::os::unix::fs::symlink(&env.repo, env.tmp.path().join("o/r")).unwrap();
    let (pkg, test) = if spike.is_some() {
        (
            "./cmd/gh",
            "go test ./pkg/cmdutil/... ./pkg/cmd/issue/create/...",
        )
    } else {
        ("./cmd/x", "go test ./...")
    };
    let build = format!(
        "go build -o bin/out {pkg} && go version -m bin/out | grep -E 'vcs\\.' | sed 's/^[[:space:]]*//'"
    );
    let mut r = Runner::new(env);
    let report = |what: &str, o: &StepOut| {
        let line = o
            .stderr
            .lines()
            .find(|l| l.contains("QUOD  EXECUTED") || l.contains("QUOD  REPLAYED"))
            .unwrap_or("")
            .to_string();
        println!("{what}: {line}");
        for l in o
            .stderr
            .lines()
            .filter(|l| l.contains("ran again") || l.contains("differed at"))
        {
            println!("  {l}");
        }
    };
    let manifest_size = |r: &Runner| {
        let m = r.ledger().candidates(&newest_key(r), 1).unwrap().remove(0);
        let files = m.entries.iter().filter(|e| e.k == Kind::File).count();
        (m.entries.len(), files)
    };

    // G1: go build records (no taint from a file git only stat()ed) and
    // stamps the commit; git opens no file it has an entry for.
    let g1 = r.job(&build, &[]);
    report("G1 go build", &g1);
    let (lookups, opens, mb) = last_mount(&r);
    let stamp = String::from_utf8_lossy(&g1.stdout).to_string();
    println!("G1 vcs: {}", stamp.lines().collect::<Vec<_>>().join(", "));
    let recorded = !g1.stderr.contains("ran again");
    let sizes = recorded.then(|| manifest_size(&r));
    println!(
        "G1 manifest (entries, files): {}; observer: {lookups} lookups, {opens} opens, {mb} MiB read",
        sizes.map_or("not recorded".into(), |(a, b)| format!("{a}, {b}"))
    );
    // G0: what git does over the served checkout: entries it re-checked by
    // reading the file (trace2's refresh/sum_scan; 0 in a fresh checkout).
    let g0 = r.job(
        "GIT_TRACE2_PERF=1 git status --porcelain 2>&1 >/dev/null | grep -o 'refresh/sum_scan:[0-9]*'",
        &[],
    );
    let scans = String::from_utf8_lossy(&g0.stdout).to_string();
    println!(
        "G0 git status re-checked: {}",
        scans.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    r.mode = "replay";
    // The first build put the module cache's download directory in the
    // workspace state the next job starts from: one more run, then the
    // same commit replays.
    let g1a = r.job(&build, &[]);
    report("G1 next job, go build", &g1a);
    let g1b = r.job(&build, &[]);
    report("G1 same commit again, go build", &g1b);
    let g2t = r.job(test, &[]);
    report("G2 go test", &g2t);
    let g2tb = r.job(test, &[]);
    report("G2 same commit again, go test", &g2tb);

    // G3: a README-only commit: go test replays (HEAD names the branch);
    // go build runs (its binary embeds the new revision) and misses on .git.
    let readme = std::fs::read_to_string(r.env.repo.join("README.md")).unwrap_or_default();
    r.env.write("README.md", &format!("{readme}\nG3 edit\n"));
    r.env.commit("G3: README only");
    let g3t = r.job(test, &[]);
    report("G3 README commit, go test", &g3t);
    let g3b = r.job(&build, &[]);
    report("G3 README commit, go build", &g3b);
    // G4: an edited .github file, then a Go source and go.mod: go test
    // replays the first and runs the others; go build runs.
    let wf = std::fs::read_dir(r.env.repo.join(".github/workflows"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .min()
        .expect("a workflow file");
    let text = std::fs::read_to_string(&wf).unwrap();
    std::fs::write(&wf, format!("{text}\n# G4 edit\n")).unwrap();
    r.env.commit("G4: a .github file");
    let g4t = r.job(test, &[]);
    report("G4 .github commit, go test", &g4t);
    let src = if spike.is_some() {
        "pkg/cmdutil/args.go"
    } else {
        "calc/calc.go"
    };
    let text = std::fs::read_to_string(r.env.repo.join(src)).unwrap();
    r.env.write(src, &format!("{text}\n// G4 edit\n"));
    r.env.commit("G4: a Go source");
    let g4s = r.job(test, &[]);
    report("G4 Go source commit, go test", &g4s);
    let g4b = r.job(&build, &[]);
    report("G4 Go source commit, go build", &g4b);
    let gomod = std::fs::read_to_string(r.env.repo.join("go.mod")).unwrap();
    r.env.write("go.mod", &format!("{gomod}\n// G4 edit\n"));
    r.env.commit("G4: go.mod");
    let g4m = r.job(test, &[]);
    report("G4 go.mod commit, go test", &g4m);
    // G5 (reported, not asserted): go build in a job that has GITHUB_TOKEN,
    // which offline steps are not given: the withheld-name scan then reads
    // every file the step was served. Which file it names, if any.
    let g5 = r.job(
        &build,
        &[("GITHUB_TOKEN", "ghs_0123456789abcdefghijABCDEFGHIJ0123")],
    );
    report("G5 go build with GITHUB_TOKEN withheld", &g5);
    for l in g5.stderr.lines().filter(|l| l.contains("names ")) {
        println!("  {l}");
    }
    if measure {
        return;
    }
    assert!(g1.executed() && g1.code == 0, "G1: {}", g1.stderr);
    assert!(recorded, "G1: go build must be recorded: {}", g1.stderr);
    assert!(
        stamp.contains("vcs.revision=") && stamp.contains("vcs.modified=false"),
        "G1: {stamp}"
    );
    assert!(
        !scans.trim().is_empty() && scans.split_whitespace().all(|s| s == "refresh/sum_scan:0"),
        "G0: git re-checked entries over the served index: {scans}"
    );
    assert!(
        g1b.replayed(),
        "G1: the same commit replays: {}",
        g1b.stderr
    );
    assert!(g2tb.replayed(), "G2: {}", g2tb.stderr);
    assert!(
        g3t.replayed(),
        "G3: a README commit replays go test: {}",
        g3t.stderr
    );
    assert!(g3b.executed() && !g3b.replayed(), "G3: {}", g3b.stderr);
    assert!(
        g3b.stderr.contains("differed at .git/"),
        "G3: go build misses on .git: {}",
        g3b.stderr
    );
    assert!(
        g4t.replayed(),
        "G4: a .github commit replays go test: {}",
        g4t.stderr
    );
    for (what, o) in [
        ("Go source, go test", &g4s),
        ("Go source, go build", &g4b),
        ("go.mod, go test", &g4m),
    ] {
        assert!(o.executed() && !o.replayed(), "G4 {what}: {}", o.stderr);
    }
    println!("hooks git index: G1-G4 pass");
}
