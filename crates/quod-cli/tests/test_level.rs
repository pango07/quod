//! Test-level replay through `quod run` (docs/test-level.md) on the
//! fixture project fixtures/test-level: a change to one source file re-runs
//! only its tests; a conftest change re-runs all; a data file re-runs the
//! test that read it; an order-dependent test is caught; a failing test
//! stays failing; parametrized cases are separate tests; xdist; the opt-out;
//! and the mutation suite (every file of the tree broken in turn: no test
//! that a plain run fails is ever replayed). Needs Linux, FUSE, bwrap, the
//! Python toolchain and uv, and the network once per test (step A installs
//! pytest); gated on QUOD_INTEGRATION=1.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Env, Out, enabled, git, last_event, toolchain_installed};
use quod_key::Toolchain;

const STEP: &str = "python -m pytest -rA -p no:cacheprovider";
const XDIST: &str = "python -m pytest -rA -p no:cacheprovider -n 2 --dist loadfile";

fn ready() -> bool {
    enabled()
        && toolchain_installed(Toolchain::Python)
        && Path::new("/usr/local/uv/bin/uv").is_file()
}

/// The fixture as a fresh checkout with its dependencies installed (step
/// A, with the network), under a policy where a test replays from its
/// second matching run and nothing is audited: the cases below are about
/// attribution; the trust suite covers the schedule.
fn fixture() -> Env {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/test-level");
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let st = Command::new("cp")
        .arg("-a")
        .arg(&src)
        .arg(&repo)
        .status()
        .unwrap();
    assert!(st.success());
    git(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join(".git/info/exclude"), ".quod/\n").unwrap();
    let env = Env {
        tmp,
        repo,
        toolchain: Some(Toolchain::Python),
        run_defaults: ["--mode", "replay", "--shadow", "0", "--audit", "0"]
            .map(String::from)
            .to_vec(),
    };
    env.commit("init");
    let a = env.run_step("uv sync --frozen", &["--allow-net"]);
    assert_eq!(a.code, 0, "step A: {}", a.stderr);
    env
}

fn stdout(o: &Out) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// Per test, from `-rA`'s short summary of the last pytest session in the
/// output: (outcome word, replayed).
fn outcomes(o: &Out) -> BTreeMap<String, (String, bool)> {
    let text = stdout(o);
    let last = text
        .rfind("short test summary info")
        .map_or(&text[..], |i| &text[i..]);
    parse_outcomes(last)
}

/// Every test replayed in any session of the output: a run that is run
/// again in full (a failure beside replays) printed its replays first.
fn replayed_anywhere(o: &Out) -> Vec<String> {
    let mut v: Vec<String> = stdout(o)
        .lines()
        .filter_map(|l| l.strip_prefix("PASSED (replayed) "))
        .map(|id| id.trim().to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}

fn parse_outcomes(text: &str) -> BTreeMap<String, (String, bool)> {
    let mut m = BTreeMap::new();
    for l in text.lines() {
        let (word, rest) = match l.split_once(' ') {
            Some(x) => x,
            None => continue,
        };
        if !["PASSED", "FAILED", "ERROR", "XFAIL", "XPASS"].contains(&word) {
            continue;
        }
        let (replayed, rest) = match rest.strip_prefix("(replayed) ") {
            Some(r) => (true, r),
            None => (false, rest),
        };
        let id = rest.split(" - ").next().unwrap().trim().to_string();
        m.insert(id, (word.to_string(), replayed));
    }
    m
}

fn replayed(o: &Out) -> Vec<String> {
    outcomes(o)
        .into_iter()
        .filter(|(_, (_, r))| *r)
        .map(|(id, _)| id)
        .collect()
}

/// The tests that ran (and passed, failed or errored). An xfail's short
/// summary line does not say whether it replayed; it is left out.
fn ran(o: &Out) -> Vec<String> {
    outcomes(o)
        .into_iter()
        .filter(|(_, (w, r))| !*r && w != "XFAIL")
        .map(|(id, _)| id)
        .collect()
}

/// pytest's summary line (`10 passed, 1 skipped, ... in 0.04s`) without
/// its duration.
fn summary(o: &Out) -> String {
    let text = stdout(o);
    let l = text
        .lines()
        .rev()
        .find(|l| l.contains(" passed") || l.contains(" failed"))
        .unwrap_or_default()
        .to_string();
    let l = l.trim_matches(|c| c == '=' || c == ' ');
    l.rsplit_once(" in ").map_or(l, |(a, _)| a).to_string()
}

fn tests_event(env: &Env) -> serde_json::Value {
    last_event(&env.state(), "tests").expect("a tests event")
}

/// Two runs on the same tree: the first records every test, the second
/// (the whole step's inputs matching) runs every test again and leaves
/// each test with a proof; the next miss of the step replays them.
fn warm(env: &Env, step: &str) -> String {
    let a = env.run_step(step, &[]);
    assert_eq!(a.code, 0, "{}", a.stderr);
    assert!(
        a.stderr.contains("test level: 12 tests recorded"),
        "{}",
        a.stderr
    );
    summary(&a)
}

#[test]
fn one_source_change_reruns_only_its_tests() {
    if !ready() {
        return;
    }
    let env = fixture();
    let full = warm(&env, STEP);
    env.write("src/calc/add.py", "def add(a, b):\n    return b + a\n");
    env.commit("add");
    let o = env.run_step(STEP, &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_eq!(
        ran(&o),
        [
            "tests/test_add.py::test_add",
            "tests/test_add.py::test_add_base"
        ]
    );
    assert_eq!(replayed(&o).len(), 8, "{}", stdout(&o));
    // What a full run reports: the same counts and exit code.
    assert_eq!(summary(&o), full);
    assert!(
        o.stderr.contains("QUOD  EXECUTED AT TEST LEVEL  "),
        "{}",
        o.stderr
    );
    // The xfail test runs too: formatting its traceback stats every file
    // linecache holds (src/calc/add.py among them), so it depends on them.
    // Over-recorded, never under.
    let e = tests_event(&env);
    assert_eq!(
        (e["replayed"].as_u64(), e["executed"].as_u64()),
        (Some(9), Some(3)),
        "{e}"
    );
}

#[test]
fn a_data_file_reruns_the_test_that_read_it_and_parametrized_cases_are_tests() {
    if !ready() {
        return;
    }
    let env = fixture();
    warm(&env, STEP);
    env.write(
        "data/table.json",
        "{\"two\": 2, \"three\": 3, \"four\": 4}\n",
    );
    env.commit("table");
    let o = env.run_step(STEP, &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    // Read inside the test, not at import: its neighbour replays.
    assert_eq!(ran(&o), ["tests/test_table.py::test_two"]);
    // A module two imports away: every case of the parametrized test.
    env.write(
        "src/calc/helpers.py",
        "def repeat(a, n):\n    return sum(a for _ in range(n))\n",
    );
    env.commit("helpers");
    let o = env.run_step(STEP, &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_eq!(
        ran(&o),
        [
            "tests/test_mul.py::test_mul[1-7-7]",
            "tests/test_mul.py::test_mul[2-3-6]",
            "tests/test_mul.py::test_mul[4-0-0]"
        ]
    );
}

#[test]
fn a_conftest_change_reruns_every_test() {
    if !ready() {
        return;
    }
    let env = fixture();
    warm(&env, STEP);
    let c = fs::read_to_string(env.repo.join("tests/conftest.py")).unwrap();
    env.write("tests/conftest.py", &format!("{c}\n# edited\n"));
    env.commit("conftest");
    let o = env.run_step(STEP, &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(replayed(&o).is_empty(), "{}", stdout(&o));
    assert!(
        o.stderr.contains("every test ran (the session's inputs"),
        "{}",
        o.stderr
    );
}

/// test_b passes only after test_a ran in the same process. Changing what
/// test_b alone reads replays test_a: test_b fails, so the step runs every
/// test again (the step passes, as a plain run does), test_b is compared
/// with that run and quarantined, and its module is coupled: next time its
/// tests run together.
#[test]
fn an_order_dependent_test_is_caught_and_its_module_coupled() {
    if !ready() {
        return;
    }
    let env = fixture();
    let full = warm(&env, STEP);
    env.write("data/b.txt", "c\n");
    env.commit("b");
    let o = env.run_step(STEP, &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stderr.contains("running every test"), "{}", o.stderr);
    assert!(
        o.stderr
            .contains("1 diverged and are quarantined: tests/test_order.py::test_b_needs_state"),
        "{}",
        o.stderr
    );
    assert_eq!(summary(&o), full);
    // The next run executes every test (the divergence asked for it).
    env.write("data/b.txt", "d\n");
    env.commit("b2");
    let o = env.run_step(STEP, &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    // Then the module runs whole whenever one of its tests runs.
    env.write("data/b.txt", "e\n");
    env.commit("b3");
    let o = env.run_step(STEP, &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(!o.stderr.contains("running every test"), "{}", o.stderr);
    let r = ran(&o);
    assert!(
        r.contains(&"tests/test_order.py::test_a_sets_state".to_string())
            && r.contains(&"tests/test_order.py::test_b_needs_state".to_string()),
        "{r:?}"
    );
}

#[test]
fn a_failing_test_stays_failing() {
    if !ready() {
        return;
    }
    let env = fixture();
    warm(&env, STEP);
    env.write("src/calc/add.py", "def add(a, b):\n    return a - b\n");
    env.commit("break add");
    let o = env.run_step(STEP, &[]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    assert!(summary(&o).starts_with("2 failed"), "{}", summary(&o));
    // Unrelated change: the failures are never replayed as passes.
    env.write("data/table.json", "{\"two\": 2}\n");
    env.commit("table");
    let o = env.run_step(STEP, &[]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    let out = outcomes(&o);
    for t in [
        "tests/test_add.py::test_add",
        "tests/test_add.py::test_add_base",
    ] {
        assert_eq!(out[t].0, "FAILED", "{t}: {}", stdout(&o));
    }
}

#[test]
fn xdist_workers_attribute_their_own_tests() {
    if !ready() {
        return;
    }
    let env = fixture();
    let full = warm(&env, XDIST);
    env.write("src/calc/add.py", "def add(a, b):\n    return b + a\n");
    env.commit("add");
    let o = env.run_step(XDIST, &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_eq!(
        ran(&o),
        [
            "tests/test_add.py::test_add",
            "tests/test_add.py::test_add_base"
        ]
    );
    assert_eq!(summary(&o), full);
}

#[test]
fn quod_test_level_0_runs_the_plain_step() {
    if !ready() {
        return;
    }
    let env = fixture();
    let repo = env.repo.to_str().unwrap();
    let o = env.quod_in(
        &env.state(),
        &[
            "run",
            "--repo",
            repo,
            "--toolchain",
            "python",
            "--mode",
            "replay",
            "--step",
            STEP,
        ],
        &[("QUOD_TEST_LEVEL", "0")],
    );
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(last_event(&env.state(), "tests").is_none());
    assert!(!o.stderr.contains("test level"), "{}", o.stderr);
}

/// The A7 of test level: every file of the fixture's sources, tests and
/// data broken in turn (a Python file made to raise when imported, any
/// other file emptied of its content), each on a copy of a warm state. A
/// test Quod replays must pass in a plain run of the same broken tree: 0
/// false replays. Also checks something replayed at all, so the suite is
/// not vacuous.
#[test]
fn mutation_suite_never_replays_a_test_a_plain_run_fails() {
    if !ready() {
        return;
    }
    let env = fixture();
    warm(&env, STEP);
    let warm_state = env.tmp.path().join("warm-state");
    cp(&env.state(), &warm_state);
    // The plain runs need the installed dependencies, which live in the
    // state dir: a copy of the warm one (another step key: plain).
    let plain_state = env.tmp.path().join("plain-state");
    cp(&warm_state, &plain_state);
    let mut files = Vec::new();
    for top in ["src", "tests", "data"] {
        walk(&env.repo.join(top), &mut files);
    }
    let (mut replays, mut checked) = (0, 0);
    for f in files {
        let rel = f
            .strip_prefix(&env.repo)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let orig = fs::read(&f).unwrap();
        let broken = if rel.ends_with(".py") {
            [b"raise RuntimeError('quod mutation')\n".as_slice(), &orig].concat()
        } else {
            b"mutated\n".to_vec()
        };
        fs::write(&f, &broken).unwrap();
        env.commit(&format!("break {rel}"));
        let state = env.tmp.path().join("m-state");
        let _ = fs::remove_dir_all(&state);
        cp(&warm_state, &state);
        let repo = env.repo.to_str().unwrap();
        let q = env.quod_in(
            &state,
            &[
                "run",
                "--repo",
                repo,
                "--toolchain",
                "python",
                "--mode",
                "replay",
                "--shadow",
                "0",
                "--audit",
                "0",
                "--step",
                STEP,
            ],
            &[],
        );
        let plain = env.quod_in(
            &plain_state,
            &[
                "run",
                "--repo",
                repo,
                "--toolchain",
                "python",
                "--step",
                STEP,
            ],
            &[("QUOD_TEST_LEVEL", "0")],
        );
        let truth = outcomes(&plain);
        for t in replayed_anywhere(&q) {
            replays += 1;
            let p = truth.get(&t).map(|(w, _)| w.as_str());
            assert!(
                matches!(p, Some("PASSED") | Some("XFAIL")),
                "{rel}: {t} replayed, but a plain run says {p:?}\n{}",
                stdout(&plain)
            );
        }
        assert_eq!(q.code, plain.code, "{rel}: exit codes differ");
        checked += 1;
        fs::write(&f, &orig).unwrap();
        env.commit(&format!("restore {rel}"));
    }
    eprintln!("mutation suite: {checked} files broken, {replays} replays checked, 0 false");
    assert!(checked >= 10 && replays > 0, "{checked} {replays}");
}

fn cp(src: &Path, dst: &Path) {
    let st = Command::new("cp")
        .arg("-a")
        .arg(src)
        .arg(dst)
        .status()
        .unwrap();
    assert!(st.success());
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            if !p.ends_with("__pycache__") {
                walk(&p, out);
            }
        } else {
            out.push(p);
        }
    }
    out.sort();
}
