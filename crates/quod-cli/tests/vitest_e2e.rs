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

/// Two runs with the same arguments (a render is for the same command
/// line), every output outside the project (a new path in it is a new tree,
/// which a render refuses): the first records and renders, the second
/// replays every file. Returns both summaries and the second's stderr; each
/// run's outcomes are plain vitest's.
fn render_runs(p: &Project, t: &Path) -> (serde_json::Value, serde_json::Value, String) {
    let report = t.join("quod.json");
    let ledger = t.join("ledger");
    let mut runs = Vec::new();
    for n in 1..=2 {
        let plain = t.join(format!("plain-{n}.json"));
        let st = p
            .cmd(&p.dir.join("node_modules/.bin/vitest"))
            .args(["run", "--reporter=json"])
            .arg(format!("--outputFile={}", plain.display()))
            .status()
            .unwrap();
        assert!(st.success(), "plain vitest failed");
        let _ = fs::remove_file(&report);
        let summary = t.join(format!("summary-{n}.json"));
        let out = p
            .cmd(Path::new(env!("CARGO_BIN_EXE_quod")))
            .arg("vitest")
            .arg("--dir")
            .arg(&ledger)
            .arg("--node")
            .arg(p.node.join("node"))
            .arg("--summary")
            .arg(&summary)
            .args(["--audits", "0", "--", "run", "--reporter=json"])
            .arg(format!("--outputFile={}", report.display()))
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(out.status.success(), "quod vitest failed:\n{err}");
        let read = |p: &Path| -> serde_json::Value {
            serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
        };
        assert_eq!(outcomes(&read(&plain)), outcomes(&read(&report)), "run {n}:\n{err}");
        runs.push((read(&summary), err));
    }
    let (s2, err) = runs.pop().unwrap();
    (runs.pop().unwrap().0, s2, err)
}

#[test]
fn a_full_hit_emits_the_render_with_a_relative_test_dir_or_root() {
    // vitest keeps `test.dir` as written (`test`), relative to the root;
    // the render compared it with the absolute cwd and declined ("test dir
    // test is outside ..."), so a full hit started vitest (zustand: 1.7 s
    // for 13 replayed files). Resolved, the second run emits.
    let configs = [
        ("dir", "export default { test: { dir: 'test' } }\n"),
        ("root", "export default { root: '.', test: { dir: './test' } }\n"),
    ];
    for (what, config) in configs {
        let t = tempfile::tempdir().unwrap();
        let Some(p) = project(&t) else {
            return;
        };
        fs::write(p.dir.join("vitest.config.mjs"), config).unwrap();
        let (s1, s2, err) = render_runs(&p, t.path());
        assert_eq!(s1["ran"], 3, "{what}: {s1}");
        assert!(
            s1["render"]["ms"].is_number() && s1["render"]["why"].is_null(),
            "{what}: the recording run did not render: {s1}"
        );
        assert_eq!(s2["replayed"], 3, "{what}: {s2}");
        assert_eq!(s2["fast"], true, "{what}: a full hit started vitest: {s2}\n{err}");
        assert!(err.contains("emitted the recorded full replay"), "{err}");
    }
}

/// The coverage outputs a CI job depends on, as written: coverage-final.json
/// (parsed: istanbul's per-file statement, branch and function counters),
/// lcov.info and coverage-summary.json (bytes). None when absent.
fn coverage_outputs(dir: &Path) -> Vec<(String, Option<String>)> {
    ["coverage-final.json", "lcov.info", "coverage-summary.json"]
        .iter()
        .map(|f| {
            let s = fs::read_to_string(dir.join("coverage").join(f)).ok();
            let s = match (*f, s) {
                ("coverage-final.json", Some(s)) => Some(
                    serde_json::from_str::<serde_json::Value>(&s)
                        .unwrap()
                        .to_string(),
                ),
                (_, s) => s,
            };
            (f.to_string(), s)
        })
        .collect()
}

#[test]
fn coverage_reports_and_thresholds_match_plain_with_replays() {
    // `vitest --coverage`, v8 and istanbul: a replayed file's recorded
    // coverage goes into the report, so every coverage output and the
    // threshold's exit code are plain vitest's, run after run.
    for provider in ["v8", "istanbul"] {
        let t = tempfile::tempdir().unwrap();
        let Some(p) = project(&t) else {
            return;
        };
        fs::write(
            p.dir.join("src/other.ts"),
            "export function twice(n: number): number {\n  if (n > 100) {\n    return n\n  }\n  return n * 2\n}\n",
        )
        .unwrap();
        fs::write(
            p.dir.join("src/untested.ts"),
            "export const never = (x: number) => x + 1\n",
        )
        .unwrap();
        fs::write(
            p.dir.join("test/other.test.ts"),
            "import { expect, test } from 'vitest'\nimport { twice } from '../src/other'\n\ntest('twice', () => {\n  expect(twice(2)).toBe(4)\n})\n",
        )
        .unwrap();
        let args = |extra: &[&str]| -> Vec<String> {
            let mut v: Vec<String> = [
                "run",
                "--coverage",
                "--coverage.include=src/**",
                "--coverage.reporter=json",
                "--coverage.reporter=lcov",
                "--coverage.reporter=json-summary",
                "--coverage.reporter=text",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            v.push(format!("--coverage.provider={provider}"));
            v.extend(extra.iter().map(|s| s.to_string()));
            v
        };
        // `witnesses`: QUOD_VITEST_WITNESSES ("" is the default policy, a
        // second witness before a record replays).
        let run = |n: u32, extra: &[&str], witnesses: &str| -> (bool, bool, serde_json::Value) {
            let plain = p
                .cmd(&p.dir.join("node_modules/.bin/vitest"))
                .args(args(extra))
                .output()
                .unwrap();
            let want = coverage_outputs(&p.dir);
            assert!(
                want.iter().all(|(_, s)| s.is_some()),
                "{provider}: plain wrote no coverage:\n{}",
                String::from_utf8_lossy(&plain.stderr)
            );
            fs::remove_dir_all(p.dir.join("coverage")).unwrap();
            let summary = p.dir.join(format!("summary-{provider}-{n}.json"));
            let out = p
                .cmd(Path::new(env!("CARGO_BIN_EXE_quod")))
                .env("QUOD_VITEST_WITNESSES", witnesses)
                .arg("vitest")
                .arg("--dir")
                .arg(p.dir.join(".quod/vitest"))
                .arg("--node")
                .arg(p.node.join("node"))
                .arg("--summary")
                .arg(&summary)
                .args(["--audits", "0", "--"])
                .args(args(extra))
                .output()
                .unwrap();
            let err = String::from_utf8_lossy(&out.stderr);
            let got = coverage_outputs(&p.dir);
            for ((f, a), (_, b)) in want.iter().zip(&got) {
                assert_eq!(a, b, "{provider} run {n}: {f} differs from plain\n{err}");
            }
            let s: serde_json::Value =
                serde_json::from_slice(&fs::read(&summary).unwrap()).unwrap();
            (plain.status.success(), out.status.success(), s)
        };

        // 1. Nothing recorded: every file runs (and records its coverage).
        let (pl, q, s) = run(1, &[], "1");
        assert!(pl && q, "{provider}: {s}");
        assert_eq!(s["ran"], 4, "{provider}: {s}");
        // 2. Nothing changed: every file replays; the report is the same.
        let (pl, q, s) = run(2, &[], "1");
        assert!(pl && q, "{provider}: {s}");
        assert_eq!(s["replayed"], 4, "{provider}: {s}");
        // Never the rendered full replay: coverage is made fresh each run.
        assert_ne!(s["fast"], true, "{provider}: coverage emitted a render: {s}");
        // 3. math.ts changes: its three test files run (coverage: no
        //    function-level narrowing), other.test.ts replays.
        edit(&p.dir, "src/math.ts", "return a - b", "return a - b + 0");
        let (pl, q, s) = run(3, &[], "1");
        assert!(pl && q, "{provider}: {s}");
        assert_eq!(s["ran"], 3, "{provider}: {s}");
        assert_eq!(s["replayed"], 1, "{provider}: {s}");
        // 4. A threshold the untested file breaks: the configuration is part
        //    of the key (every file runs), and the job fails as plain's does.
        let th = ["--coverage.thresholds.functions=100"];
        let (pl, q, s) = run(4, &th, "1");
        assert!(!pl && !q, "{provider}: the threshold must fail both: {s}");
        assert_eq!(s["ran"], 4, "{provider}: {s}");
        // 5. Unchanged under the threshold: replays, and still fails.
        let (pl, q, s) = run(5, &th, "1");
        assert!(!pl && !q, "{provider}: {s}");
        assert_eq!(s["replayed"], 4, "{provider}: {s}");
        // 6-8. The default policy (inline recording, a second witness): a
        //    change to math.ts records its three files provisionally (other.test.ts too if
        //    its one-witness record counts as provisional), the
        //    next run witnesses them again, the one after replays them;
        //    every run's coverage is plain's (the threshold still fails
        //    both).
        edit(&p.dir, "src/math.ts", "return a + b", "return a + b + 0");
        let (pl, q, s) = run(6, &th, "");
        assert!(!pl && !q, "{provider}: {s}");
        assert!(s["ran"].as_u64().unwrap() >= 3, "{provider}: {s}");
        let (pl, q, s) = run(7, &th, "");
        assert!(!pl && !q, "{provider}: {s}");
        assert!(s["ran"].as_u64().unwrap() >= 3, "{provider}: {s}");
        assert!(s["promoted"].as_u64().unwrap() >= 3, "{provider}: {s}");
        let (pl, q, s) = run(8, &th, "");
        assert!(!pl && !q, "{provider}: {s}");
        assert_eq!(s["replayed"], 4, "{provider}: {s}");
    }
}
