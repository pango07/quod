//! `quod pytest`: test-file-level replay for pytest, with shadow recording.
//!
//! `quod pytest --no-record [--dir D] -- <pytest args>` is the developer's
//! job (the critical path): it selects against the most recent completed
//! ledger, runs only the affected test files in one plain pytest session
//! (no recording), in their original relative order, and replays the rest
//! through pytest's own hooks, so the summary, `-rA` output, JUnit XML and
//! exit code are a full run's. `quod pytest [--dir D] --expect-report R`
//! (no `--no-record`) is the shadow: a separate, non-blocking job of the
//! same commit that runs the affected files with recording, records a file
//! only when its results match the critical path's (R: its `--summary`),
//! certifies each recorded file order-independent (alone in a fresh
//! process == in the session) and writes the ledger.
//!
//! The logic lives in a pytest plugin (`pytest/quod_pytest.py`) that this
//! command writes into `D/plugin/` and runs with the project's Python.
//! See docs/pytest-shadow.md for the design, the cross-file state rule
//! and the residuals.

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

const PLUGIN: &str = include_str!("pytest/quod_pytest.py");
const RUN: &str = include_str!("pytest/run.py");

/// Variables the GitHub runner sets per step (file-command paths), like
/// `quod_key::unkeyed`'s per-run ones: a test file that reads one is not
/// recorded.
const STEP_VOLATILE: &[&str] = &[
    "GITHUB_OUTPUT",
    "GITHUB_ENV",
    "GITHUB_PATH",
    "GITHUB_STATE",
    "GITHUB_STEP_SUMMARY",
    "OLDPWD",
    "_",
];

#[derive(clap::Args)]
pub struct PytestArgs {
    /// Where the ledger, the recorded results and the plugin live
    /// (default: .quod/pytest under the current directory).
    #[arg(long)]
    dir: Option<PathBuf>,
    /// The critical path: select, run what must run plain (no recording, no
    /// certification, no audits) and replay the rest; the ledger is not
    /// touched. A shadow run of the same commit without this flag records.
    #[arg(long)]
    no_record: bool,
    /// Shadow runs: the critical path's `--summary` of the same commit. A
    /// file that ran here records only if its per-test results match the
    /// summary's; otherwise recording changed its behaviour and it is
    /// quarantined (it always runs).
    #[arg(long, value_name = "REPORT")]
    expect_report: Option<PathBuf>,
    /// Shadow runs: replayable files to run anyway and compare with their
    /// records (a mismatch quarantines the file).
    #[arg(long, default_value_t = 4)]
    audits: usize,
    /// Write this run's numbers (files run, replayed, timings, per-test
    /// outcomes of the tests that ran) as JSON.
    #[arg(long)]
    summary: Option<PathBuf>,
    /// Run every test file (and, in a shadow, record all of them).
    #[arg(long)]
    full: bool,
    /// File-level only: no function-level narrowing.
    #[arg(long)]
    no_narrow: bool,
    /// The project's Python (with pytest installed).
    #[arg(long, default_value = "python")]
    python: String,
    /// pytest's arguments.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

pub fn cli(a: PytestArgs) -> Result<u8> {
    let root = std::env::current_dir()?;
    let abs = |p: &PathBuf| {
        if p.is_absolute() {
            p.clone()
        } else {
            root.join(p)
        }
    };
    let dir = a
        .dir
        .as_ref()
        .map(abs)
        .unwrap_or_else(|| root.join(".quod/pytest"));
    let plugin_dir = dir.join("plugin");
    fs::create_dir_all(&plugin_dir)
        .with_context(|| format!("creating {}", plugin_dir.display()))?;
    // run.py imports the plugin as a module, so Python caches its bytecode.
    for (name, body) in [("quod_pytest.py", PLUGIN), ("run.py", RUN)] {
        let p = plugin_dir.join(name);
        if fs::read_to_string(&p).ok().as_deref() != Some(body) {
            fs::write(&p, body)?;
        }
    }
    let plugin = plugin_dir.join("run.py");
    let volatile: Vec<String> = std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| {
            quod_key::unkeyed(k) || STEP_VOLATILE.contains(&k.as_str()) || k.starts_with("QUOD_")
        })
        .collect();
    let mode = if a.no_record { "critical" } else { "shadow" };
    let opts = serde_json::json!({
        "mode": mode,
        "dir": dir,
        "summary": a.summary.as_ref().map(abs),
        "expect_report": a.expect_report.as_ref().map(abs),
        "audits": if a.no_record { 0 } else { a.audits },
        "full": a.full || std::env::var("QUOD_PYTEST_FULL").is_ok_and(|v| v == "1"),
        "no_narrow": a.no_narrow,
        "volatile": volatile,
        "quod": env!("CARGO_PKG_VERSION"),
    });
    let opts_path = plugin_dir.join(format!("opts-{}.json", std::process::id()));
    fs::write(&opts_path, serde_json::to_vec(&opts)?)?;
    let status = Command::new(&a.python)
        .arg(&plugin)
        .arg(&opts_path)
        .args(&a.args)
        .status()
        .with_context(|| format!("starting {}", a.python))?;
    let _ = fs::remove_file(&opts_path);
    Ok(match (status.code(), status.signal()) {
        (Some(c), _) => c as u8,
        (None, Some(s)) => 128u8.wrapping_add(s as u8),
        _ => 1,
    })
}
