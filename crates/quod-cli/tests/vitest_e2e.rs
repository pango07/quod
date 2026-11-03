//! `quod vitest` end to end on a real vitest 4 project
//! (fixtures/vitest-e2e, installed with `npm ci`): a recording run records
//! every file with function-level data; an unchanged re-run replays them
//! all; a change inside a function only one test file executed re-runs that
//! file and replays the others (function-level); a change to a function
//! whose source a test name quotes re-runs that test; a new file in a
//! directory one test's import.meta.glob lists re-runs that test only (the
//! listing is that module's input, not every file's). Every run's JSON
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
}

impl Project {
    fn cmd(&self, prog: &Path) -> Command {
        let mut c = Command::new(prog);
        c.current_dir(&self.dir).env("PATH", &self.path);
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

#[test]
fn records_replays_and_narrows_a_real_vitest_project() {
    let Some(node) = node_bin() else {
        // On CI the gate installs node: a skip there would hide the test.
        assert!(
            std::env::var("CI").as_deref() != Ok("true"),
            "no node/npm on CI"
        );
        eprintln!("no node/npm: skipped");
        return;
    };
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("p");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/vitest-e2e");
    copy_dir(&fixture, &dir);
    let path = format!(
        "{}:{}",
        node.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let p = Project { dir, path, node };
    let ci = p
        .cmd(&p.node.join("npm"))
        .args(["ci", "--no-audit", "--no-fund", "--loglevel=error"])
        .status()
        .unwrap();
    assert!(ci.success(), "npm ci failed");

    // 1. Nothing recorded yet: every file runs and is recorded.
    let (plain, quod, s) = p.run(1);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 4, "{s}");
    assert_eq!(s["recorded_files"], 4, "{s}");

    // 2. Nothing changed: everything replays.
    let (plain, quod, s) = p.run(2);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["ran"], 0, "{s}");
    assert_eq!(s["replayed"], 4, "{s}");

    // 3. The body of `sub`, which only sub.test.ts executes, changes:
    //    sub.test.ts runs; add.test.ts replays function-level; name.test.ts
    //    (no function-level data: it reads source) runs; glob.test.ts
    //    replays (it never loads math.ts).
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

    // 5. A new file in src/lib, which only glob.test.ts's import.meta.glob
    //    lists: that listing changed what vite wrote into glob.test.ts, so it
    //    runs; it is not a shared input, so the other files replay.
    fs::write(p.dir.join("src/lib/three.ts"), "export const three = 3\n").unwrap();
    let (plain, quod, s) = p.run(5);
    assert_eq!(outcomes(&plain), outcomes(&quod));
    assert_eq!(s["shared_inputs_changed"], false, "{s}");
    assert_eq!(s["ran"], 1, "{s}");
    assert_eq!(s["replayed"], 3, "{s}");
}
