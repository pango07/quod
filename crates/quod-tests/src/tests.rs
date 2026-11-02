use std::collections::BTreeMap;

use quod_fs::{ProcInfo, Trace, TraceEvent, Traced};

use super::*;

const MAIN: u32 = 100;

fn file(p: &str) -> Access {
    Access {
        path: p.into(),
        kind: Kind::File,
    }
}

/// Builds a trace and the main process's plugin output together: markers
/// get the plugin's numbers, accesses the next sequence number.
#[derive(Default)]
struct Run {
    trace: Trace,
    out: PluginOut,
    seq: u64,
    stack: Vec<(u64, String, String)>,
}

impl Run {
    fn new() -> Run {
        let mut r = Run {
            out: PluginOut {
                v: 1,
                pid: MAIN,
                role: "main".into(),
                done: true,
                imports_tracked: true,
                exitstatus: Some(0),
                ..PluginOut::default()
            },
            ..Run::default()
        };
        r.proc(MAIN, MAIN, 1);
        r
    }

    fn proc(&mut self, tid: u32, tgid: u32, ppid: u32) {
        self.trace.procs.insert(tid, ProcInfo { tgid, ppid });
    }

    fn push(&mut self, tid: u32, what: Traced) {
        self.seq += 1;
        self.trace.events.push(TraceEvent {
            seq: self.seq,
            tid,
            what,
        });
    }

    fn marker(&mut self, be: &str, kind: &str, id: &str) -> u64 {
        let n = self.out.events.len() as u64 + 1;
        self.out.events.push((n, be.into(), kind.into(), id.into()));
        let pid = self.out.pid;
        self.push(MAIN, Traced::Marker(pid, n));
        n
    }

    fn begin(&mut self, kind: &str, id: &str) -> u64 {
        let n = self.marker("b", kind, id);
        self.stack.push((n, kind.into(), id.into()));
        n
    }

    fn end(&mut self) {
        let (_, kind, id) = self.stack.pop().unwrap();
        self.marker("e", &kind, &id);
    }

    fn read(&mut self, tid: u32, p: &str) {
        self.push(tid, Traced::Access(file(p)));
    }

    fn edge(&mut self, window: u64, m: &str) {
        self.out
            .edges
            .entry(window.to_string())
            .or_default()
            .push(m.into());
    }

    fn test(&mut self, nodeid: &str, module: &str, outcome: &str) {
        self.out.collected.push(nodeid.into());
        self.out.tests.insert(
            nodeid.into(),
            TestOut {
                module: Some(module.into()),
                reports: vec![serde_json::json!({"when": "call", "outcome": outcome})],
                warnings: vec![],
                duration: 0.25,
            },
        );
    }

    fn attribute(&self) -> Result<Attribution, TestsError> {
        attribute(&self.trace, std::slice::from_ref(&self.out))
    }
}

fn paths(s: &BTreeSet<Access>) -> Vec<&str> {
    s.iter().map(|a| a.path.as_str()).collect()
}

/// Two test modules import one source module; the second import is a
/// cache hit that reads nothing, yet both tests depend on what the first
/// load read. A test's own read is its alone; a child process's read is
/// its parent's window's; an unknown process's read and a listing are the
/// session's.
#[test]
fn reads_go_to_their_windows_and_imports_carry_their_loads() {
    let mut r = Run::new();
    r.read(MAIN, "pyproject.toml"); // before any window: session
    let c1 = r.begin("c", "tests/test_a.py");
    r.read(MAIN, "tests/test_a.py");
    r.edge(c1, "calc");
    r.edge(c1, "calc.a");
    let l = r.begin("l", "calc.a");
    r.read(MAIN, "src/calc/a.py");
    r.edge(l, "calc.util");
    let lu = r.begin("l", "calc.util");
    r.read(MAIN, "src/calc/util.py");
    r.end();
    r.end();
    r.end();
    let c2 = r.begin("c", "tests/test_b.py");
    r.read(MAIN, "tests/test_b.py");
    r.edge(c2, "calc.a"); // cached: no read
    r.end();
    let _ = lu;
    r.begin("t", "tests/test_a.py::test_one");
    r.read(MAIN, "data/one.json");
    r.proc(200, 200, MAIN);
    r.read(200, "data/child.txt");
    r.proc(300, 300, 1);
    r.read(300, "data/stranger.txt");
    r.push(
        MAIN,
        Traced::Access(Access {
            path: "data".into(),
            kind: Kind::Dir,
        }),
    );
    r.end();
    r.begin("t", "tests/test_b.py::test_two");
    r.end();
    r.test("tests/test_a.py::test_one", "tests/test_a.py", "passed");
    r.test("tests/test_b.py::test_two", "tests/test_b.py", "passed");
    let a = r.attribute().unwrap();
    assert_eq!(
        paths(&a.session),
        ["data", "data/stranger.txt", "pyproject.toml"]
    );
    assert_eq!(
        paths(&a.executed["tests/test_a.py::test_one"].accesses),
        [
            "data/child.txt",
            "data/one.json",
            "src/calc/a.py",
            "src/calc/util.py",
            "tests/test_a.py"
        ]
    );
    assert_eq!(
        paths(&a.executed["tests/test_b.py::test_two"].accesses),
        ["src/calc/a.py", "src/calc/util.py", "tests/test_b.py"]
    );
    assert_eq!(a.executed["tests/test_a.py::test_one"].duration_ms, 250);
}

/// A module- or session-scoped fixture set up inside a test is the
/// module's or the session's, not the test's.
#[test]
fn scoped_fixtures_belong_to_their_scope() {
    let mut r = Run::new();
    r.begin("c", "tests/test_a.py");
    r.end();
    r.begin("t", "tests/test_a.py::t1");
    r.begin("s", "");
    r.read(MAIN, "db/session.sql");
    r.end();
    r.begin("m", "tests/test_a.py");
    r.read(MAIN, "db/module.sql");
    r.end();
    r.read(MAIN, "own.txt");
    r.end();
    r.test("tests/test_a.py::t1", "tests/test_a.py", "passed");
    let a = r.attribute().unwrap();
    assert_eq!(paths(&a.session), ["db/session.sql"]);
    assert_eq!(
        paths(&a.executed["tests/test_a.py::t1"].accesses),
        ["db/module.sql", "own.txt"]
    );
}

/// An import that ran off the main thread makes its window's reads and
/// imports the session's.
#[test]
fn an_off_thread_import_makes_the_window_the_sessions() {
    let mut r = Run::new();
    r.begin("c", "tests/test_a.py");
    r.end();
    let t = r.begin("t", "tests/test_a.py::t1");
    r.read(MAIN, "own.txt");
    r.edge(t, "late");
    r.end();
    r.out.offthread.push(t);
    r.test("tests/test_a.py::t1", "tests/test_a.py", "passed");
    let a = r.attribute().unwrap();
    assert_eq!(paths(&a.session), ["own.txt"]);
    assert!(a.executed["tests/test_a.py::t1"].accesses.is_empty());
}

#[test]
fn a_lost_marker_or_a_taint_makes_the_run_unusable() {
    let mut r = Run::new();
    r.begin("t", "tests/test_a.py::t1");
    r.end();
    r.test("tests/test_a.py::t1", "tests/test_a.py", "passed");
    assert!(r.attribute().is_ok());
    let mut lost = Run {
        trace: r.trace.clone(),
        out: r.out.clone(),
        ..Run::default()
    };
    lost.trace
        .events
        .retain(|e| e.what != Traced::Marker(MAIN, 2));
    assert!(lost.attribute().is_err());
    let mut tainted = Run {
        trace: r.trace.clone(),
        out: r.out.clone(),
        ..Run::default()
    };
    tainted.out.taint = Some("--maxfail".into());
    assert!(tainted.attribute().is_err());
    let mut unfinished = Run {
        trace: r.trace.clone(),
        out: r.out.clone(),
        ..Run::default()
    };
    unfinished.out.done = false;
    assert!(unfinished.attribute().is_err());
    let mut untracked = Run {
        trace: r.trace.clone(),
        out: r.out.clone(),
        ..Run::default()
    };
    untracked.out.imports_tracked = false;
    assert!(untracked.attribute().is_err());
    // A test both collected and missing from the run.
    let mut missing = Run {
        trace: r.trace.clone(),
        out: r.out.clone(),
        ..Run::default()
    };
    missing.out.collected.push("tests/test_a.py::ghost".into());
    assert!(missing.attribute().is_err());
}

/// Under xdist the master's reads are the session's and each worker's
/// tests are attributed by the worker's own windows.
#[test]
fn xdist_workers_attribute_their_own_tests() {
    let mut w = Run::new();
    w.out.role = "worker".into();
    w.out.workerid = Some("gw0".into());
    w.begin("t", "t.py::a");
    w.read(MAIN, "a.txt");
    w.end();
    w.test("t.py::a", "t.py", "passed");
    w.proc(50, 50, 1);
    w.read(50, "master.txt");
    let master = PluginOut {
        v: 1,
        pid: 50,
        role: "master".into(),
        done: true,
        exitstatus: Some(0),
        ..PluginOut::default()
    };
    let a = attribute(&w.trace, &[master, w.out.clone()]).unwrap();
    assert_eq!(paths(&a.session), ["master.txt"]);
    assert_eq!(paths(&a.executed["t.py::a"].accesses), ["a.txt"]);
}

#[test]
fn two_owning_sessions_are_refused() {
    let mut r = Run::new();
    r.begin("t", "t.py::a");
    r.end();
    r.test("t.py::a", "t.py", "passed");
    let mut other = r.out.clone();
    other.pid = 101;
    other.events.clear();
    assert!(attribute(&r.trace, &[r.out.clone(), other]).is_err());
}

#[test]
fn records_round_trip_and_report_failures() {
    let r = Record {
        reports: vec![
            serde_json::json!({"when": "setup", "outcome": "passed"}),
            serde_json::json!({"when": "call", "outcome": "failed", "longrepr": "boom"}),
        ],
        warnings: vec![],
    };
    assert!(r.failed());
    assert_eq!(Record::from_blob(&r.to_blob()).unwrap(), r);
    // Keys sorted: the same report always gives the same blob.
    let a = Record {
        reports: vec![serde_json::json!({"b": 1, "a": 2})],
        warnings: vec![],
    };
    let mut m = BTreeMap::new();
    m.insert("a", 2);
    m.insert("b", 1);
    let b = Record {
        reports: vec![serde_json::to_value(m).unwrap()],
        warnings: vec![],
    };
    assert_eq!(a.to_blob(), b.to_blob());
}

#[test]
fn keys_and_detection() {
    let s = Hash::of(b"step");
    assert_ne!(test_key(&s, "t.py::a"), test_key(&s, "t.py::b"));
    assert_ne!(test_key(&s, "t.py::a[1]"), test_key(&s, "t.py::a[2]"));
    assert_ne!(
        test_key(&s, "t.py::a"),
        test_key(&Hash::of(b"other"), "t.py::a")
    );
    assert_ne!(session_key(&s), test_key(&s, ""));
    for yes in [
        "pytest",
        "python -m pytest",
        "python -m pytest -q tests/",
        "uv run pytest -n 4",
        ".venv/bin/pytest tests",
        "cd x && py.test",
    ] {
        assert!(runs_pytest(yes), "{yes}");
    }
    for no in [
        "go test ./...",
        "npm test",
        "python -m unittest",
        "pytest-cov --help-me",
    ] {
        assert!(!runs_pytest(no), "{no}");
    }
}

/// The vendored plugin is the one this crate documents (a smoke check
/// that the file is not empty or truncated).
#[test]
fn the_plugin_is_vendored() {
    assert!(PLUGIN_SOURCE.contains("def pytest_configure(config):"));
    assert!(PLUGIN_SOURCE.contains("PLUGIN_VERSION = 1"));
}
