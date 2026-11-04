//! `quod vitest` end to end on a real vitest 4 project
//! (fixtures/vitest-e2e, installed with `npm ci`): a recording run records
//! every file with function-level data; an unchanged re-run replays them
//! all; a change inside a function only one test file executed re-runs that
//! file and replays the others (function-level); a change to a function
//! whose source a test name quotes re-runs that test. Every run's JSON
//! report matches plain vitest's. A recorder crash fails here, not in a
//! benchmark. Skipped when there is no node/npm (the lint gate installs
//! node at /usr/local/node).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn node_bin() -> Option<PathBuf> {
    for dir in ["/usr/local/node/bin", "/usr/bin", "/usr/local/bin"] {
        let d = PathBuf::from(dir);
        if d.join("node").is_file() && d.join("npm").is_file() {
            return Some(d);
        }
    }
    None
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if e.file_type().unwrap().is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

struct Project {
    dir: PathBuf,
    path: String,
    node: PathBuf,
    /// Set for every command (the CI provider's variables of a run).
    env: Vec<(&'static str, &'static str)>,
}

impl Project {
    fn cmd(&self, prog: &Path) -> Command {
        let mut c = Command::new(prog);
        c.current_dir(&self.dir).env("PATH", &self.path);
        // These cases predate the second witness (the default): one run
        // trusts a record. `two_witnesses_by_default` covers the default.
        c.env("QUOD_VITEST_WITNESSES", "1");
        for (k, v) in &self.env {
            c.env(k, v);
        }
        c
    }

    /// Plain vitest's JSON report, then quod's run and its summary.
    fn run(&self, n: u32) -> (serde_json::Value, serde_json::Value, serde_json::Value) {
        let plain = self.dir.join(format!("plain-{n}.json"));
        let st = self
            .cmd(&self.dir.join("node_modules/.bin/vitest"))
            .args(["run", "--reporter=json"])
            .arg(format!("--outputFile={}", plain.display()))
            .status()
            .unwrap();
        assert!(st.success(), "plain vitest failed");
        let report = self.dir.join(format!("quod-{n}.json"));
        let summary = self.dir.join(format!("summary-{n}.json"));
        let out = self
            .cmd(Path::new(env!("CARGO_BIN_EXE_quod")))
            .arg("vitest")
            .arg("--dir")
            .arg(self.dir.join(".quod/vitest"))
            .arg("--node")
            .arg(self.node.join("node"))
            .arg("--summary")
            .arg(&summary)
            .args(["--audits", "0"])
            .args(["--", "run", "--reporter=json"])
            .arg(format!("--outputFile={}", report.display()))
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "quod vitest failed:\n{err}");
        assert!(
            !err.contains("left no record") && !err.contains("could not record"),
            "the recorder failed:\n{err}"
        );
        let read = |p: &Path| -> serde_json::Value {
            serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
        };
        (read(&plain), read(&report), read(&summary))
    }
}

/// Per file: its status and every test's name and status.
fn outcomes(report: &serde_json::Value) -> Vec<(String, String, Vec<String>)> {
    let mut v: Vec<_> = report["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let mut tests: Vec<String> = t["assertionResults"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| format!("{}={}", a["fullName"], a["status"]))
                .collect();
            tests.sort();
            (
                t["name"].as_str().unwrap().to_string(),
                t["status"].as_str().unwrap().to_string(),
                tests,
            )
        })
        .collect();
    v.sort();
    v
}

fn edit(dir: &Path, file: &str, from: &str, to: &str) {
    let p = dir.join(file);
    let s = fs::read_to_string(&p).unwrap();
    assert!(s.contains(from), "{from} not in {file}");
    fs::write(&p, s.replace(from, to)).unwrap();
}

/// The fixture, installed, in a temp dir; None (skipped) without node.
fn project(t: &tempfile::TempDir) -> Option<Project> {
    let Some(node) = node_bin() else {
        // On CI the gate installs node: a skip there would hide the test.
        assert!(
            std::env::var("CI").as_deref() != Ok("true"),
            "no node/npm on CI"
        );
        eprintln!("no node/npm: skipped");
        return None;
    };
    let dir = t.path().join("p");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/vitest-e2e");
    copy_dir(&fixture, &dir);
    let path = format!(
        "{}:{}",
        node.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let p = Project {
        dir,
        path,
        node,
        env: Vec::new(),
    };
    let ci = p
        .cmd(&p.node.join("npm"))
        .args(["ci", "--no-audit", "--no-fund", "--loglevel=error"])
        .status()
        .unwrap();
    assert!(ci.success(), "npm ci failed");
    Some(p)
}

#[test]
fn records_replays_and_narrows_a_real_vitest_project() {
    let t = tempfile::tempdir().unwrap();
    let Some(p) = project(&t) else {
        return;
    };

    // 1. Nothing recorded yet: every file runs and is recorded.
    let (plain, quod, s) = p.run(1);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 3, "{s}");
    assert_eq!(s["recorded_files"], 3, "{s}");

    // 2. Nothing changed: everything replays.
    let (plain, quod, s) = p.run(2);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 0, "{s}");
    assert_eq!(s["replayed"], 3, "{s}");

    // 3. The body of `sub`, which only sub.test.ts executes, changes:
    //    sub.test.ts runs; add.test.ts replays function-level; name.test.ts
    //    (no function-level data: it reads source) runs.
    edit(&p.dir, "src/math.ts", "return a - b", "return a - b + 0");
    let (plain, quod, s) = p.run(3);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 2, "{s}");
    assert_eq!(s["narrowed"], 1, "{s}");

    // 4. The body of `describeAdd`, whose source name.test.ts's test name
    //    quotes, changes: the name changes, and Quod reports the new one.
    edit(&p.dir, "src/math.ts", "`adds ${n}`", "`adds ${n}!`");
    let (plain, quod, s) = p.run(4);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert!(s["ran"].as_u64().unwrap() >= 1, "{s}");
}

#[test]
fn a_branch_variable_counts_only_for_the_test_that_reads_it() {
    // vitest copies the whole environment to its workers; GitHub sets
    // GITHUB_REF, GITHUB_EVENT_NAME, ... per branch and event. A recording
    // made on a push replays on a pull request, but for the test file that
    // reads GITHUB_REF itself: that one runs again.
    let t = tempfile::tempdir().unwrap();
    let Some(mut p) = project(&t) else {
        return;
    };
    fs::write(
        p.dir.join("test/ref.test.ts"),
        "import { expect, test } from 'vitest'\n\
         test('reads the ref', () => {\n  \
         expect(process.env.GITHUB_REF?.startsWith('refs/')).toBe(true)\n})\n",
    )
    .unwrap();
    let pr = vec![
        ("GITHUB_REF", "refs/pull/7/merge"),
        ("GITHUB_REF_NAME", "7/merge"),
        ("GITHUB_EVENT_NAME", "pull_request"),
        ("GITHUB_HEAD_REF", "feature/x"),
        ("GITHUB_BASE_REF", "dev"),
    ];
    p.env = vec![
        ("GITHUB_REF", "refs/heads/dev"),
        ("GITHUB_REF_NAME", "dev"),
        ("GITHUB_EVENT_NAME", "push"),
        ("GITHUB_HEAD_REF", ""),
        ("GITHUB_BASE_REF", ""),
    ];
    let (plain, quod, s) = p.run(1);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["recorded_files"], 4, "{s}");

    p.env = pr.clone();
    let (plain, quod, s) = p.run(2);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 1, "only ref.test.ts reads GITHUB_REF: {s}");
    assert_eq!(s["replayed"], 3, "{s}");

    // Recorded again on the pull request: everything replays there.
    let (plain, quod, s) = p.run(3);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 0, "{s}");
    assert_eq!(s["replayed"], 4, "{s}");

    // Any other variable still counts for every file (CI is vitest's own).
    p.env = pr.into_iter().chain([("CI", "1")]).collect();
    let (plain, quod, s) = p.run(4);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 4, "{s}");
}

#[test]
fn a_hidden_variable_read_through_an_env_copy_quarantines_the_file() {
    // A recording worker lists the environment without the per-run
    // variables (QUOD_VITEST_HIDE). Code that copies the environment and
    // then reads a per-run variable from the copy sees it in the critical
    // path (plain) and not while recording: the shadow's comparison with
    // the critical path's report catches the difference and quarantines the
    // file, so it never replays.
    let t = tempfile::tempdir().unwrap();
    let Some(mut p) = project(&t) else {
        return;
    };
    fs::write(
        p.dir.join("test/copy.test.ts"),
        "import { expect, test } from 'vitest'\n\
         test('the run id, from a copy', () => {\n  \
         const env = { ...process.env }\n  \
         expect(env.GITHUB_RUN_ID).toBe('42')\n})\n",
    )
    .unwrap();
    p.env = vec![("GITHUB_RUN_ID", "42")];
    let quod = |extra: &[&str], n: &str| {
        let summary = p.dir.join(format!("summary-{n}.json"));
        let out = p
            .cmd(Path::new(env!("CARGO_BIN_EXE_quod")))
            .arg("vitest")
            .arg("--dir")
            .arg(p.dir.join(".quod/vitest"))
            .arg("--node")
            .arg(p.node.join("node"))
            .arg("--summary")
            .arg(&summary)
            .args(extra)
            .args(["--", "run"])
            .output()
            .unwrap();
        let s: serde_json::Value = serde_json::from_slice(&fs::read(&summary).unwrap()).unwrap();
        (out.status.success(), s)
    };
    let report = p.dir.join("critical.json");
    let report = report.to_str().unwrap();
    // The critical path, plain: the copy has the run id; the test passes.
    let (ok, _) = quod(&["--no-record", "--report", report], "crit");
    assert!(ok, "the critical path failed");
    // The shadow: recording, the copy has no run id; the test fails there,
    // and the file is quarantined (recording changed its behaviour).
    let (_, s) = quod(&["--audits", "0", "--expect-report", report], "shadow");
    let bad: Vec<String> = s["audit_mismatches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(bad.len(), 1, "{s}");
    // So the next critical path runs it (never a replay of the shadow's
    // result), and the others replay.
    let (ok, s) = quod(&["--no-record", "--report", report], "crit2");
    assert!(ok, "{s}");
    assert_eq!(s["ran"], 1, "{s}");
}

#[test]
fn two_witnesses_by_default_inline() {
    // The check job records inline (the action, no shadow job): a record
    // replays only once a second run of the same inputs agreed. Nothing is
    // hidden from the tests: a copy of the environment has the run id (so
    // that file reads a per-run variable and always runs).
    let t = tempfile::tempdir().unwrap();
    let Some(mut p) = project(&t) else {
        return;
    };
    fs::write(
        p.dir.join("test/copy.test.ts"),
        "import { expect, test } from 'vitest'\n\
         test('the run id, from a copy', () => {\n  \
         const env = { ...process.env }\n  \
         expect(env.GITHUB_RUN_ID).toBeTruthy()\n})\n",
    )
    .unwrap();
    // "" is no number: the default policy.
    p.env = vec![("QUOD_VITEST_WITNESSES", ""), ("GITHUB_RUN_ID", "41")];
    let (plain, quod, s) = p.run(1);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 4, "{s}");
    assert_eq!(s["recorded_files"], 3, "copy.test.ts reads the run id: {s}");
    assert_eq!(s["witnesses_needed"], 2, "{s}");

    // A second run (another run id): every record is provisional, so every
    // file runs again; agreeing makes it trusted.
    p.env = vec![("QUOD_VITEST_WITNESSES", ""), ("GITHUB_RUN_ID", "42")];
    let (plain, quod, s) = p.run(2);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 4, "{s}");
    assert_eq!(s["promoted"], 3, "{s}");

    // Now they replay.
    p.env = vec![("QUOD_VITEST_WITNESSES", ""), ("GITHUB_RUN_ID", "43")];
    let (plain, quod, s) = p.run(3);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 1, "{s}");
    assert_eq!(s["replayed"], 3, "{s}");
}

#[test]
fn a_failure_only_while_recording_does_not_fail_the_job() {
    // A test that the instrumentation itself makes fail (here it sees the
    // recorder; in the wild, a timing check slowed by V8 coverage) passes
    // without Quod: inline, Quod runs it again plain and the job passes,
    // with plain vitest's report.
    let t = tempfile::tempdir().unwrap();
    let Some(mut p) = project(&t) else {
        return;
    };
    fs::write(
        p.dir.join("test/instr.test.ts"),
        "import { expect, test } from 'vitest'\n\
         test('not instrumented', () => {\n  \
         expect((globalThis as any).__quod_vitest__).toBeUndefined()\n})\n",
    )
    .unwrap();
    p.env = vec![("QUOD_VITEST_WITNESSES", "")];
    for n in 1..=2 {
        // run() asserts quod succeeded, as plain vitest did
        let (plain, quod, s) = p.run(n);
        assert_eq!(outcomes(&plain), outcomes(&quod));
        assert_eq!(
            s["recorded_files"], 3,
            "instr.test.ts is never recorded: {s}"
        );
    }
}

#[test]
fn a_config_reading_package_json_straight_is_keyed_by_the_fields_it_read() {
    // The config takes a field of package.json (convex-helpers takes its
    // dependency names): a version bump replays every file, a change to the
    // field it read runs every file.
    let t = tempfile::tempdir().unwrap();
    let Some(p) = project(&t) else {
        return;
    };
    fs::write(
        p.dir.join("vitest.config.mjs"),
        "import { readFileSync } from 'node:fs'\n\
         const pkg = JSON.parse(\n  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),\n)\n\
         export default { test: { env: { PKG_PRIVATE: String(pkg.private) } } }\n",
    )
    .unwrap();
    // (its names count too: a version field from the start)
    edit(
        &p.dir,
        "package.json",
        "\"private\": true,",
        "\"private\": true,\n  \"version\": \"1.0.0\",",
    );
    let (plain, quod, s) = p.run(1);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["recorded_files"], 3, "{s}");

    edit(&p.dir, "package.json", "\"1.0.0\"", "\"1.0.1\"");
    let (plain, quod, s) = p.run(2);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 0, "a version bump: {s}");
    assert_eq!(s["replayed"], 3, "{s}");

    edit(
        &p.dir,
        "package.json",
        "\"private\": true,",
        "\"private\": false,",
    );
    let (plain, quod, s) = p.run(3);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 3, "the field the config read changed: {s}");
}
