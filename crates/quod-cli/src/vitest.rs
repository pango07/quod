//! `quod vitest`: test-file-level replay for vitest.
//!
//! `quod vitest [--dir D] -- run --project unit` runs vitest through a
//! driver (`vitest/driver.mjs`) that records, for every test file it runs,
//! everything the file's isolated worker read (its module graph, its file,
//! directory and environment reads, the packages it loaded) and what
//! vitest's main process read for all of them (config, plugins, the tools).
//! The ledger (`D/ledger.json`) keys each file's result on those inputs'
//! states. On the next run Quod hashes every recorded input once, in
//! parallel, with a stat cache, and a test file whose inputs and the
//! shared ones are all unchanged replays: vitest still finds every test
//! file itself, runs the others, and reports the replayed files' recorded
//! results to its reporters, so the summary, exit code and JSON/JUnit
//! output are a full run's. A file that failed, could not be recorded, or
//! did anything the recorder cannot follow (a child process, the network,
//! a worker thread, a write to the workspace) always runs.
//!
//! Audits: when the files to run leave workers idle, a random sample of
//! replayable files (no slower than the slowest file that runs anyway)
//! runs instead of replaying, in the same vitest invocation, and its
//! result is compared with the recorded one; a mismatch quarantines the
//! file (it always runs from then on). So audits cost no wall time.
//!
//! See docs/vitest.md for the design and the measurements.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const DRIVER: &str = include_str!("vitest/driver.mjs");
const SETUP: &str = include_str!("vitest/setup.mjs");
const RECORD: &str = include_str!("vitest/record.mjs");
const COVERAGE: &str = include_str!("vitest/coverage.mjs");
const PRELOAD: &str = include_str!("vitest/preload.cjs");
const NARROW: &str = include_str!("vitest/narrow.mjs");

/// Bumped when what a key means or what is recorded changes: a ledger of
/// another version is dropped (every file runs once). v2: function-level
/// coverage and pinned states.
const LEDGER_VERSION: u32 = 2;

/// How old a file's times must be for its hash to be cached by identity
/// (git's racy-index rule, as `quod_cas`'s import stat cache).
const SETTLE_NS: i128 = 2_000_000_000;

/// Variables the GitHub runner sets per step (file-command paths, the
/// step's artifact bookkeeping, systemd's process metadata); not keyed by
/// `envall:`, like `quod_key::unkeyed`'s per-run ones. Without them a
/// critical path and its shadow in different steps never agree on the
/// environment (vite reads all of it), so every file would run.
const STEP_VOLATILE: &[&str] = &[
    "GITHUB_OUTPUT",
    "GITHUB_ENV",
    "GITHUB_PATH",
    "GITHUB_STATE",
    "GITHUB_STEP_SUMMARY",
    "GITHUB_ARTIFACTS",
    "GITHUB_ARTIFACTS_LIST",
    "SYSTEMD_EXEC_PID",
    "JOURNAL_STREAM",
    "INVOCATION_ID",
    "OLDPWD",
    "_",
];

#[derive(clap::Args)]
pub struct VitestArgs {
    /// Where the ledger, the recorded results and the driver live
    /// (default: .quod/vitest under the current directory).
    #[arg(long)]
    dir: Option<PathBuf>,
    /// Run (and record) every test file. Also QUOD_VITEST_FULL=1.
    #[arg(long)]
    full: bool,
    /// Replayable files to audit per run: `auto` (the workers the files to
    /// run leave idle, none when nothing runs) or a number.
    #[arg(long, default_value = "auto")]
    audits: String,
    /// The node binary.
    #[arg(long, default_value = "node")]
    node: String,
    /// Write this run's numbers (files run, replayed, timings) as JSON.
    #[arg(long)]
    summary: Option<PathBuf>,
    /// File-level only: no function-level narrowing (a test file whose
    /// inputs changed always runs).
    #[arg(long)]
    no_narrow: bool,
    /// The critical path: select, run what must run plain (no recording, no
    /// coverage, no audits) and replay the rest; the ledger is not touched.
    /// A shadow run of the same commit without this flag records the files
    /// that ran (and audits), off the job's critical path.
    #[arg(long)]
    no_record: bool,
    /// Before installing dependencies: decide from the checkout alone
    /// (lockfile, package.json files, node, pnpm, the source) whether any
    /// test file must run. When none must and the last recording rendered
    /// a full replay, print it (summary, reports, exit code) and write
    /// `done` to FILE: the job can skip the install. Otherwise write `run`
    /// (install and run `quod vitest --no-record`). Uncertain means `run`.
    #[arg(long, value_name = "FILE")]
    plan: Option<PathBuf>,
    /// Never emit a rendered full replay; always start vitest. Also
    /// QUOD_VITEST_NO_FAST=1.
    #[arg(long)]
    no_fast: bool,
    /// Shadow runs: the critical path's vitest JSON report of the same
    /// commit. A file that ran here records only if its per-test results
    /// match the report's; otherwise recording changed its behaviour and it
    /// is quarantined (it always runs).
    #[arg(long, value_name = "REPORT")]
    expect_report: Option<PathBuf>,
    /// Write every test's name and state, per file, to FILE in the form of
    /// vitest's JSON report (`testResults[].assertionResults[]`), whatever
    /// reporters the run uses: the critical path's report for a shadow's
    /// `--expect-report`, without adding a reporter to the user's run. A
    /// rendered full replay writes it with no files (the shadow then runs
    /// none either).
    #[arg(long, value_name = "FILE")]
    report: Option<PathBuf>,
    /// Instead of running: fold an observer manifest (`quod inspect` of a
    /// `quod run` of the same suite) into the ledger. Every path the
    /// observer saw read that no recorded key accounts for becomes an input
    /// of every test file.
    #[arg(long, value_name = "MANIFEST")]
    absorb: Option<PathBuf>,
    /// Keep the ledger in Quod Cloud instead of a cache: before the run,
    /// fetch the nearest recorded ledger (the job's commit's ancestors,
    /// else the newest of its branch, else the default branch's); after a
    /// record run, save it; after every run, send the run's report. The
    /// URL defaults to QUOD_SERVER. Auth: GitHub OIDC (`permissions:
    /// id-token: write`) or QUOD_SERVER_TOKEN; QUOD_SERVER_KEY pins the key
    /// lookups are signed with. Without this flag nothing is sent anywhere.
    #[arg(long, value_name = "URL", num_args = 0..=1, default_missing_value = "")]
    cloud: Option<String>,
    /// With --cloud: only fetch the ledger into --dir (`restore`) or only
    /// save --dir (`save`), and exit 0 whatever happens. For a workflow
    /// that keeps its own restore and save steps (as with actions/cache).
    #[arg(long, value_name = "restore|save", requires = "cloud")]
    cloud_only: Option<String>,
    /// With --cloud: do not save the ledger after this run (e.g. when an
    /// `--absorb` step follows and saves it).
    #[arg(long, requires = "cloud")]
    cloud_no_save: bool,
    /// With --cloud: the suite's name in Quod Cloud (default: this
    /// directory relative to the repository root, `.` at the root; also
    /// QUOD_VITEST_SUITE).
    #[arg(long, requires = "cloud")]
    suite: Option<String>,
    /// vitest's arguments, e.g. `run --project unit`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct Ledger {
    version: u32,
    /// Dependency keys, interned: `kind:path`, paths under the root relative.
    keys: Vec<String>,
    /// Each key's state when recorded (parallel to `keys`).
    states: Vec<String>,
    /// The stat identity the state was computed from, when cacheable.
    idents: Vec<Option<[i64; 6]>>,
    /// The inputs of every file (vitest's main process).
    global: Vec<u32>,
    /// Node, platform, vitest and recorder versions the ledger was made with.
    facts: BTreeMap<String, String>,
    /// vitest's arguments (a filter or option changes what a file's result is).
    args: Vec<String>,
    /// By vitest's task id of the file (its path and project).
    files: BTreeMap<String, Rec>,
    /// Every test file vitest found at the last record (task ids, sorted).
    #[serde(default)]
    all_files: Vec<String>,
    /// The workspace's paths (not contents) when the last record started;
    /// empty when they could not be summed (a symlink to a directory).
    #[serde(default)]
    tree: String,
    /// What a frozen install depends on (lockfile, package.json files, npm
    /// config, node, pnpm), when the last record ran; empty: unknown.
    #[serde(default)]
    install: String,
    /// A full replay of this ledger as vitest printed and wrote it.
    #[serde(default)]
    render: Option<Render>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Render {
    /// `render_sig` of the ledger it was made from.
    sig: String,
    /// vitest's arguments, exactly.
    args: Vec<String>,
    /// `code_hash`: the quod that made it.
    code: String,
    exit: u8,
    outputs: Vec<RenderOut>,
    /// The render directory as the reporters saw it (their messages name
    /// the files they wrote there: rewritten to where they go).
    #[serde(default)]
    dir: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct RenderOut {
    /// Under `<dir>/render`.
    file: String,
    /// Where the reporter wrote it (relative: under the root).
    to: String,
    append: bool,
    /// The target is this variable's value now (GitHub's step summary).
    env: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Rec {
    project: String,
    file: String,
    state: String,
    duration: f64,
    result: String,
    deps: Vec<u32>,
    #[serde(default)]
    quarantined: bool,
    /// While quarantined: the recordings in a row since the last mismatch
    /// whose results matched (the critical path's, an audit's). At
    /// `reprove()` the file is ready to replay again.
    #[serde(default)]
    good: u32,
    #[serde(default)]
    runs: u32,
    #[serde(default)]
    replays: u32,
    /// Per workspace source file the test file reached as a module: the
    /// functions it executed (0-based line and column of start and end, in
    /// the version it was recorded on).
    #[serde(default)]
    cov: BTreeMap<u32, Vec<[u32; 4]>>,
    /// Inputs that changed since the record without making the file run
    /// (function-level narrowing): the state the record saw.
    #[serde(default)]
    pinned: BTreeMap<u32, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Out {
    facts: BTreeMap<String, String>,
    interrupted: bool,
    global: Vec<String>,
    ran: Vec<Ran>,
    replayed: Vec<String>,
    audit_mismatch: Vec<String>,
    #[serde(default)]
    timings: BTreeMap<String, f64>,
    #[serde(default)]
    all_files: Option<Vec<String>>,
    #[serde(default)]
    render: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Ran {
    id: String,
    project: String,
    file: String,
    state: String,
    duration: f64,
    keys: Option<Vec<String>>,
    unobservable: Vec<String>,
    result: String,
    #[serde(default)]
    cov: Option<BTreeMap<String, Vec<[u32; 4]>>>,
    #[serde(default, rename = "covWhy")]
    cov_why: Option<String>,
}

pub fn cli(a: VitestArgs) -> Result<u8> {
    let t0 = Instant::now();
    let root = std::env::current_dir()?;
    let dir = match &a.dir {
        Some(d) if d.is_absolute() => d.clone(),
        Some(d) => root.join(d),
        None => root.join(".quod/vitest"),
    };
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    // The driver and the setup file go where node loads them natively and
    // vite leaves them alone: the project's node_modules, or in a workspace
    // whose packages have none of their own (npm hoists), the nearest one up.
    let repo = root.ancestors().find(|d| d.join(".git").exists());
    let code = root
        .ancestors()
        .take_while(|d| repo.is_none_or(|r| d.starts_with(r)))
        .take(if repo.is_some() { usize::MAX } else { 1 })
        .map(|d| d.join("node_modules"))
        .find(|d| d.is_dir())
        .map_or_else(|| dir.join("code"), |d| d.join(".quod-vitest"));
    fs::create_dir_all(&code)?;
    for (name, body) in [
        ("driver.mjs", DRIVER),
        ("setup.mjs", SETUP),
        ("record.mjs", RECORD),
        ("coverage.mjs", COVERAGE),
        ("preload.cjs", PRELOAD),
        ("narrow.mjs", NARROW),
    ] {
        let p = code.join(name);
        if fs::read_to_string(&p).ok().as_deref() != Some(body) {
            // Atomically: the packages of a workspace may run at once and
            // share this directory.
            let tmp = code.join(format!("{name}.{}.tmp", std::process::id()));
            fs::write(&tmp, body)?;
            fs::rename(&tmp, &p)?;
        }
    }
    let code_dir = code.clone();
    let cloud = a
        .cloud
        .as_deref()
        .and_then(|u| crate::vitest_cloud::from_args(Some(u), a.suite.as_deref(), &root));
    match a.cloud_only.as_deref() {
        None => {}
        Some("restore") => {
            if let Some(c) = &cloud {
                c.restore(&root, &dir);
            }
            return Ok(0);
        }
        Some("save") => {
            if let Some(c) = &cloud {
                c.save(&dir);
            }
            return Ok(0);
        }
        Some(o) => anyhow::bail!("--cloud-only: restore or save, not {o:?}"),
    }
    if let Some(m) = &a.absorb {
        let code = absorb(&root, &dir, &code, m)?;
        if let Some(c) = &cloud
            && !a.cloud_no_save
        {
            c.save(&dir);
        }
        return Ok(code);
    }
    // vitest before 3.2 has neither the worker module graph nor the reporter
    // hooks quod records and replays with: plain vitest, nothing else.
    if let Some((version, entry)) = vitest_install(&root)
        && !vitest_supported(&version)
    {
        eprintln!(
            "quod vitest: vitest {version} is not supported (3.2 or later); running plain vitest"
        );
        let mut cmd = Command::new(&a.node);
        cmd.arg(entry);
        if a.args.first().map(String::as_str) != Some("run") {
            cmd.arg("run");
        }
        let status = cmd
            .args(&a.args)
            .status()
            .with_context(|| format!("running {}", a.node))?;
        return Ok(status.code().map_or(1, |c| c.clamp(0, 255) as u8));
    }
    let used = cloud.as_ref().map(|c| c.restore(&root, &dir));
    let blobs = dir.join("blobs");
    fs::create_dir_all(&blobs)?;
    let full = a.full || std::env::var("QUOD_VITEST_FULL").is_ok_and(|v| v == "1");
    let ledger_path = dir.join("ledger.json");
    let mut ledger: Ledger = fs::read(&ledger_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|l: &Ledger| l.version == LEDGER_VERSION)
        .unwrap_or_default();
    ledger.version = LEDGER_VERSION;
    let t_load = t0.elapsed();
    if let Some(pf) = &a.plan {
        return plan(&a, pf, &root, &dir, &ledger, full);
    }
    let fast_ok = !a.no_fast && std::env::var("QUOD_VITEST_NO_FAST").map_or(true, |v| v != "1");
    // The paths as this run starts (a record keeps them: the next critical
    // path compares its own before emitting a rendered full replay).
    let tree_before = if a.no_record {
        None
    } else {
        Some(walk(&root, &dir))
    };

    // Every recorded input, hashed once.
    let live = ledger.live();
    let cur = states(&root, &ledger, &live, Some(&blobs));
    let mut now: HashMap<u32, (String, Option<[i64; 6]>)> = HashMap::with_capacity(live.len());
    let mut changed: HashSet<u32> = HashSet::new();
    for (k, s) in live.iter().zip(cur) {
        if ledger.states.get(*k as usize) != Some(&s.0) {
            changed.insert(*k);
        }
        now.insert(*k, s);
    }
    let global_ok = !full
        && !ledger.files.is_empty()
        && ledger.args == keyed_args(&a.args)
        && ledger.global.iter().all(|k| !changed.contains(k));
    let mut replay: BTreeMap<String, String> = BTreeMap::new();
    let mut affected: Vec<String> = Vec::new();
    // Files whose only changed inputs are source files they reached as
    // modules, with the states their records saw: narrowing candidates.
    let mut maybe: Vec<(String, Vec<(u32, String)>)> = Vec::new();
    let mut why_run: Vec<String> = Vec::new();
    // Why each file that must run runs, in plain words (the run report).
    let mut why: HashMap<String, String> = HashMap::new();
    let global_why = if full {
        "a full run was asked for".to_string()
    } else if ledger.files.is_empty() {
        "first run (nothing recorded yet)".to_string()
    } else if ledger.args != keyed_args(&a.args) {
        "the vitest arguments changed".to_string()
    } else {
        match ledger.global.iter().find(|k| changed.contains(k)) {
            Some(k) => format!(
                "a shared input changed: {}",
                plain_key(&ledger.keys[*k as usize])
            ),
            None => String::new(),
        }
    };
    for (id, rec) in &ledger.files {
        if !(global_ok && !rec.quarantined && (rec.state == "pass" || rec.state == "skip")) {
            why.insert(
                id.clone(),
                if !global_ok {
                    global_why.clone()
                } else if rec.quarantined {
                    "always runs: a check found its reused result differed".into()
                } else {
                    format!("its last result was {}", rec.state)
                },
            );
            affected.push(id.clone());
            continue;
        }
        let seen = |d: u32| {
            rec.pinned
                .get(&d)
                .unwrap_or(&ledger.states[d as usize])
                .clone()
        };
        let moved: Vec<(u32, String)> = rec
            .deps
            .iter()
            .filter(|d| now[*d].0 != seen(**d))
            .map(|d| (*d, seen(*d)))
            .collect();
        if moved.is_empty() {
            replay.insert(id.clone(), rec.result.clone());
            continue;
        }
        let narrowable = !a.no_narrow
            && moved.iter().all(|(d, old)| {
                rec.cov.contains_key(d)
                    && ledger.keys[*d as usize].starts_with("f:")
                    && old.len() == 64
                    && now[d].0.len() == 64
                    && blobs.join(old).is_file()
            });
        if narrowable {
            maybe.push((id.clone(), moved));
        } else {
            // Why it runs: the first changed input that rules out narrowing.
            let first = moved.iter().find(|(d, _)| !rec.cov.contains_key(d));
            if let Some((d, _)) = first {
                why_run.push(format!("{} <- {}", rec.file, ledger.keys[*d as usize]));
            }
            let d = first.unwrap_or(&moved[0]).0;
            why.insert(
                id.clone(),
                format!("changed: {}", plain_key(&ledger.keys[d as usize])),
            );
            affected.push(id.clone());
        }
    }
    for w in why_run.iter().take(12) {
        eprintln!("quod vitest:   runs: {w}");
    }
    let mut narrowed: HashMap<String, Vec<(u32, String)>> = HashMap::new();
    let mut narrow_note = serde_json::Value::Null;
    if !maybe.is_empty() {
        let tn = Instant::now();
        let tests: Vec<serde_json::Value> = maybe
            .iter()
            .map(|(id, moved)| {
                let rec = &ledger.files[id];
                let checks: Vec<serde_json::Value> = moved
                    .iter()
                    .map(|(d, old)| {
                        serde_json::json!({
                            "file": root.join(&ledger.keys[*d as usize][2..]),
                            "old": blobs.join(old),
                            "ranges": rec.cov[d],
                        })
                    })
                    .collect();
                let per = dir.join("tests").join(format!("{id}.json"));
                serde_json::json!({ "id": id, "checks": checks, "tests": per.is_file().then_some(per) })
            })
            .collect();
        let req_path = dir.join("narrow-request.json");
        let ans_path = dir.join("narrow-answer.json");
        let _ = fs::remove_file(&ans_path);
        fs::write(
            &req_path,
            serde_json::to_vec(&serde_json::json!({ "root": root, "tests": tests }))?,
        )?;
        let ok = Command::new(&a.node)
            .arg(code.join("narrow.mjs"))
            .arg(&req_path)
            .arg(&ans_path)
            .status()
            .is_ok_and(|s| s.success());
        let ans: Option<serde_json::Value> = if ok {
            fs::read(&ans_path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
        } else {
            None
        };
        let skip: HashSet<String> = ans
            .as_ref()
            .and_then(|v| v["skip"].as_array())
            .map(|v| {
                v.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        for (id, moved) in maybe {
            if skip.contains(&id) {
                replay.insert(id.clone(), ledger.files[&id].result.clone());
                narrowed.insert(id, moved);
            } else {
                why.insert(
                    id.clone(),
                    format!(
                        "changed: {} (code this test runs)",
                        plain_key(&ledger.keys[moved[0].0 as usize])
                    ),
                );
                affected.push(id);
            }
        }
        narrow_note = serde_json::json!({
            "ms": tn.elapsed().as_secs_f64() * 1e3,
            "why": ans.as_ref().map(|v| v["why"].clone()),
            "per_test_would_skip": ans.as_ref().map(|v| v["perTest"].clone()),
        });
        eprintln!(
            "quod vitest: function-level: {} of {} test files with changed source replay ({} ms): {}",
            narrowed.len(),
            narrowed.len() + affected.len(),
            tn.elapsed().as_millis(),
            ans.as_ref()
                .map_or("narrowing failed".into(), |v| v["why"].to_string())
        );
    }

    // Audits on the workers the files to run leave idle.
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    let n_audit = match a.audits.as_str() {
        _ if a.no_record => 0,
        "auto" if affected.is_empty() => 0,
        "auto" => workers.saturating_sub(affected.len()),
        n => n.parse().context("--audits: auto or a number")?,
    };
    let slowest = affected
        .iter()
        .map(|id| ledger.files[id].duration)
        .fold(250.0, f64::max);
    let mut candidates: Vec<&String> = replay
        .keys()
        .filter(|id| ledger.files[*id].duration <= slowest)
        .collect();
    let mut rng = Rng::new();
    let mut audit = Vec::new();
    // Narrowed replays first: they are the newer rule.
    let mut first: Vec<&String> = candidates
        .iter()
        .copied()
        .filter(|id| narrowed.contains_key(*id))
        .collect();
    candidates.retain(|id| !narrowed.contains_key(*id));
    while audit.len() < n_audit && !first.is_empty() {
        let i = rng.below(first.len());
        audit.push(first.swap_remove(i).clone());
    }
    while audit.len() < n_audit && !candidates.is_empty() {
        let i = rng.below(candidates.len());
        audit.push(candidates.swap_remove(i).clone());
    }

    let plan_path = dir.join("plan.json");
    let out_path = dir.join("out.json");
    let _ = fs::remove_file(&out_path);
    let recorded: HashMap<String, (String, f64)> = ledger
        .files
        .iter()
        .map(|(id, r)| (id.clone(), (r.file.clone(), r.duration)))
        .collect();
    let report_base = Reporting {
        cloud: cloud.as_ref(),
        used: used.clone(),
        recorded: &recorded,
        why: &why,
        first_run: ledger.files.is_empty(),
        narrowed: narrowed.len(),
        t0,
        select_ms: 0.0,
    };
    let plan = serde_json::json!({
        "stateDir": dir,
        "replay": replay,
        "audit": audit,
        "record": !a.no_record,
        "workers": if affected.len() > 4 * workers && alone() { serde_json::json!(workers) } else { serde_json::Value::Null },
        "expectReport": a.expect_report.as_ref().map(std::path::absolute).transpose()?,
        "report": a.report.as_ref().map(std::path::absolute).transpose()?,
        "expect": if ledger.facts.is_empty() { serde_json::Value::Null } else { serde_json::to_value(&ledger.facts)? },
    });
    fs::write(&plan_path, serde_json::to_vec(&plan)?)?;
    let t_select = t0.elapsed();
    let shared_changed: Vec<String> = ledger
        .global
        .iter()
        .filter(|k| changed.contains(k))
        .map(|k| ledger.keys[*k as usize].clone())
        .collect();
    for k in shared_changed.iter().take(8) {
        eprintln!("quod vitest:   shared input changed: {k}");
    }
    eprintln!(
        "quod vitest: {} recorded files: {} replay, {} to run{}{}; {} inputs hashed, {} changed; selected in {} ms",
        ledger.files.len(),
        replay.len(),
        affected.len(),
        if global_ok || ledger.files.is_empty() {
            ""
        } else {
            " (shared inputs changed: every file runs)"
        },
        if audit.is_empty() {
            String::new()
        } else {
            format!(", {} audits", audit.len())
        },
        live.len(),
        changed.len(),
        t_select.as_millis()
    );

    if a.no_record
        && fast_ok
        && !ledger.files.is_empty()
        && affected.is_empty()
        && replay.len() == ledger.files.len()
    {
        let w = walk(&root, &dir);
        let emitted = node_version(&a.node)
            .ok_or_else(|| "node --version failed".to_string())
            .and_then(|node| emit_render(&root, &dir, &ledger, &a.args, &node, &w.hash));
        match emitted {
            Ok(code) => {
                let t = t0.elapsed();
                eprintln!(
                    "quod vitest: every file replays; emitted the recorded full replay (no vitest start) in {} ms",
                    t.as_millis()
                );
                if let Some(r) = &a.report {
                    fs::write(r, serde_json::to_vec(&report_from_ledger(&dir, &ledger))?)?;
                }
                let ids: Vec<&String> = ledger.files.keys().collect();
                let n_ids = ids.len();
                fs::write(
                    &out_path,
                    serde_json::to_vec(&serde_json::json!({
                        "facts": ledger.facts, "exitCode": code, "interrupted": false, "global": [],
                        "ran": [], "replayed": ids, "auditMismatch": [], "fast": true,
                    }))?,
                )?;
                report_base.with_select(t_select).send(
                    "critical",
                    &[],
                    &ids,
                    &[],
                    &HashSet::new(),
                    &HashMap::new(),
                    Some(code),
                );
                if let Some(p) = &a.summary {
                    let s = serde_json::json!({
                        "recorded_files": ledger.files.len(), "ran": 0, "replayed": n_ids,
                        "audited": 0, "audit_mismatches": [], "shared_inputs_changed": false,
                        "narrowed": narrowed.len(), "narrow": narrow_note, "inputs_hashed": live.len(),
                        "select_ms": t_select.as_secs_f64() * 1e3,
                        "vitest_ms": (t - t_select).as_secs_f64() * 1e3,
                        "record_ms": 0.0, "fast": true, "exit": code,
                    });
                    fs::write(p, serde_json::to_vec(&s)?)?;
                }
                return Ok(code);
            }
            Err(why) => eprintln!(
                "quod vitest: every file replays, but no rendered replay to emit ({why}); starting vitest"
            ),
        }
    }

    let status = without_credentials(&mut Command::new(&a.node))
        .env("QUOD_VITEST_HIDE", hidden_env())
        .arg(code.join("driver.mjs"))
        .arg(&plan_path)
        .arg(&out_path)
        .args(&a.args)
        .status()
        .with_context(|| format!("running {}", a.node))?;
    let code = status.code().map_or(1, |c| c.clamp(0, 255) as u8);
    let t_run = t0.elapsed();

    let out: Out = match fs::read(&out_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
    {
        Some(o) => o,
        None => {
            eprintln!("quod vitest: the driver left no record; nothing recorded");
            return Ok(code);
        }
    };
    if out.interrupted {
        eprintln!("quod vitest: the run was interrupted; nothing recorded");
        return Ok(code);
    }
    let ran_facts: Vec<(String, String, String, f64)> = out
        .ran
        .iter()
        .map(|r| (r.id.clone(), r.file.clone(), r.state.clone(), r.duration))
        .collect();
    let replayed_ids: Vec<&String> = out.replayed.iter().collect();
    if a.no_record {
        report_base.with_select(t_select).send(
            "critical",
            &ran_facts,
            &replayed_ids,
            &[],
            &HashSet::new(),
            &HashMap::new(),
            Some(code),
        );
        eprintln!(
            "quod vitest: ran {} files plain, replayed {} ({} function-level); quod {} ms (select); not recorded (the shadow run records)",
            out.ran.len(),
            out.replayed.len(),
            narrowed.len(),
            t_select.as_millis()
        );
        if let Some(p) = &a.summary {
            let s = serde_json::json!({
                "recorded_files": ledger.files.len(),
                "ran": out.ran.len(),
                "replayed": out.replayed.len(),
                "audited": 0,
                "audit_mismatches": [],
                "shared_inputs_changed": !global_ok,
                "shared_changed_keys": shared_changed.iter().take(20).collect::<Vec<_>>(),
                "narrowed": narrowed.len(),
                "narrow": narrow_note,
                "inputs_hashed": live.len(),
                "select_ms": t_select.as_secs_f64() * 1e3,
                "vitest_ms": (t_run - t_select).as_secs_f64() * 1e3,
                "record_ms": 0.0,
                "driver": out.timings,
                "exit": code,
            });
            fs::write(p, serde_json::to_vec(&s)?)?;
        }
        return Ok(code);
    }

    // Ingest: the files that ran replace their records.
    let mut intern: HashMap<String, u32> = ledger
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (k.clone(), i as u32))
        .collect();
    let mut key = |ledger: &mut Ledger, k: &str| -> u32 {
        let k = relativize(&root, k);
        if let Some(&i) = intern.get(&k) {
            return i;
        }
        let i = ledger.keys.len() as u32;
        ledger.keys.push(k.clone());
        ledger.states.push(String::new());
        ledger.idents.push(None);
        intern.insert(k, i);
        i
    };
    let mut global: Vec<u32> = out.global.iter().map(|k| key(&mut ledger, k)).collect();
    if !out.replayed.is_empty() {
        global.extend(ledger.global.iter().copied());
    }
    global.sort_unstable();
    global.dedup();
    ledger.global = global;
    for id in &affected {
        ledger.files.remove(id);
    }
    let mismatched: HashSet<&String> = out.audit_mismatch.iter().collect();
    let mut unobservable: BTreeMap<String, usize> = BTreeMap::new();
    let mut no_fn_level: BTreeMap<String, usize> = BTreeMap::new();
    for r in &out.ran {
        if let Some(w) = &r.cov_why {
            *no_fn_level.entry(w.clone()).or_default() += 1;
        }
    }
    let (mut recorded, mut failed) = (0usize, 0usize);
    // Why each file that ran was not recorded (for the run report).
    let mut unrec: HashMap<String, String> = HashMap::new();
    for r in &out.ran {
        let old = ledger.files.remove(&r.id);
        // Quarantine is re-proved, not permanent: a quarantined file runs
        // every time, and once `reprove()` recordings in a row matched (a
        // one-off difference: a timeout under load), it may replay again.
        // A mismatch starts the count over.
        let was = old.as_ref().is_some_and(|o| o.quarantined);
        let bad = mismatched.contains(&r.id);
        let good = match (bad, was) {
            (true, _) => 0,
            (false, true) => old.as_ref().map_or(0, |o| o.good) + 1,
            (false, false) => 0,
        };
        let quarantined = bad || (was && good < reprove());
        let good = if quarantined { good } else { 0 };
        if was && !quarantined {
            eprintln!(
                "quod vitest: {} matched {good_runs} recordings in a row: ready to replay again",
                r.file,
                good_runs = reprove()
            );
        }
        for u in &r.unobservable {
            *unobservable.entry(u.clone()).or_default() += 1;
        }
        let Some(keys) = &r.keys else {
            unrec.insert(
                r.id.clone(),
                "not recorded: it is not in the check's report".into(),
            );
            continue;
        };
        if let Some(u) = r.unobservable.first() {
            unrec.insert(r.id.clone(), u.clone());
            continue;
        }
        // A per-run value (the commit, the run id) makes a result
        // unrepeatable. The runner's per-step bookkeeping (STEP_VOLATILE:
        // file-command paths, artifact lists) and Quod's own variables do
        // not: a test environment
        // that copies the whole environment into its context (edge-runtime)
        // reads them all, and that kept 88 of class-management's backend
        // files from ever recording.
        if let Some(v) = keys.iter().find_map(|k| {
            k.strip_prefix("env:").filter(|n| {
                volatile_env(n) && !STEP_VOLATILE.contains(n) && !n.starts_with("QUOD_")
            })
        }) {
            *unobservable
                .entry(format!("reads the per-run variable {v}"))
                .or_default() += 1;
            unrec.insert(r.id.clone(), format!("reads the per-run variable {v}"));
            continue;
        }
        if r.state != "pass" && r.state != "skip" {
            failed += 1;
            continue;
        }
        let mut deps: Vec<u32> = keys.iter().map(|k| key(&mut ledger, k)).collect();
        deps.sort_unstable();
        deps.dedup();
        let mut cov = BTreeMap::new();
        for (f, fns) in r.cov.iter().flatten() {
            let k = key(&mut ledger, &format!("f:{f}"));
            if deps.binary_search(&k).is_ok() {
                cov.insert(k, fns.clone());
            }
        }
        recorded += 1;
        ledger.files.insert(
            r.id.clone(),
            Rec {
                project: r.project.clone(),
                file: relativize(&root, &format!("f:{}", r.file))[2..].to_string(),
                state: r.state.clone(),
                duration: r.duration,
                result: r.result.clone(),
                deps,
                quarantined,
                good,
                runs: old.as_ref().map_or(0, |o| o.runs) + 1,
                replays: old.as_ref().map_or(0, |o| o.replays),
                cov,
                pinned: BTreeMap::new(),
            },
        );
    }
    for id in &out.replayed {
        if let Some(r) = ledger.files.get_mut(id) {
            r.replays += 1;
            for (d, old) in narrowed.get(id).into_iter().flatten() {
                r.pinned.entry(*d).or_insert_with(|| old.clone());
            }
        }
    }
    ledger.facts = out.facts;
    ledger.args = keyed_args(&a.args);
    let mut all_files = out.all_files.clone().unwrap_or_default();
    all_files.sort();
    ledger.all_files = all_files;
    let tree = tree_before.unwrap_or_else(|| walk(&root, &dir));
    ledger.tree = tree.hash.clone();
    ledger.install = ledger
        .facts
        .get("node")
        .and_then(|n| install_key(&root, &tree, n))
        .unwrap_or_default();
    ledger.render = None;
    let mut render_note = serde_json::Value::Null;
    if fast_ok && everything_recorded(&ledger) {
        let tr = Instant::now();
        match render(&a, &root, &dir, &code_dir, &ledger) {
            Ok(r) => {
                render_note = serde_json::json!({ "ms": tr.elapsed().as_secs_f64() * 1e3 });
                ledger.render = Some(r);
            }
            Err(why) => {
                eprintln!("quod vitest: no rendered full replay ({why})");
                render_note =
                    serde_json::json!({ "ms": tr.elapsed().as_secs_f64() * 1e3, "why": why });
            }
        }
    }

    // States: the selection's for keys it hashed, new keys now.
    let live = ledger.live();
    let missing: Vec<u32> = live
        .iter()
        .copied()
        .filter(|k| !now.contains_key(k))
        .collect();
    let fresh = states(&root, &ledger, &missing, Some(&blobs));
    for (k, s) in missing.into_iter().zip(fresh) {
        now.insert(k, s);
    }
    for k in &live {
        let (s, id) = &now[k];
        ledger.states[*k as usize] = s.clone();
        ledger.idents[*k as usize] = *id;
    }
    ledger.compact();
    let tmp = dir.join("ledger.json.tmp");
    fs::write(&tmp, serde_json::to_vec(&ledger)?)?;
    fs::rename(&tmp, &ledger_path)?;
    let t_done = t0.elapsed();

    let quod_ms = (t_select + (t_done - t_run)).as_millis();
    // Failed as vitest counts it (any file whose state is not a pass or a
    // skip, recordable or not); not recorded: everything else that ran and
    // has no record.
    let _ = failed;
    let failed_all = out
        .ran
        .iter()
        .filter(|r| r.state != "pass" && r.state != "skip")
        .count();
    eprintln!(
        "quod vitest: ran {} files ({} recorded, {} failed, {} not recordable), replayed {}, audited {}{}; quod {} ms (select {} ms, record {} ms)",
        out.ran.len(),
        recorded,
        failed_all,
        unrec.len(),
        out.replayed.len(),
        audit.len(),
        if mismatched.is_empty() {
            String::new()
        } else {
            format!(", {} AUDIT MISMATCHES (quarantined)", mismatched.len())
        },
        quod_ms,
        t_select.as_millis(),
        (t_done - t_run).as_millis(),
    );
    for (u, n) in &unobservable {
        eprintln!("quod vitest:   {n} files always run: {u}");
    }
    for (u, n) in &no_fn_level {
        eprintln!("quod vitest:   {n} files without function-level data: {u}");
    }
    let mism: HashSet<String> = out.audit_mismatch.iter().cloned().collect();
    report_base.with_select(t_select).send(
        "shadow",
        &ran_facts,
        &replayed_ids,
        &audit,
        &mism,
        &unrec,
        Some(code),
    );
    if let Some(c) = &cloud
        && !a.cloud_no_save
    {
        c.save(&dir);
    }
    if let Some(p) = &a.summary {
        let s = serde_json::json!({
            "recorded_files": ledger.files.len(),
            "ran": out.ran.len(),
            "replayed": out.replayed.len(),
            "audited": audit.len(),
            "audit_mismatches": out.audit_mismatch,
            "failed": failed,
            "unrecordable": unobservable,
            "no_function_level": no_fn_level,
            "shared_inputs_changed": !global_ok,
            "narrowed": narrowed.len(),
            "narrow": narrow_note,
            "shared_changed_keys": shared_changed.iter().take(20).collect::<Vec<_>>(),
            "inputs_hashed": live.len(),
            "load_ms": t_load.as_secs_f64() * 1e3,
            "select_ms": t_select.as_secs_f64() * 1e3,
            "vitest_ms": (t_run - t_select).as_secs_f64() * 1e3,
            "record_ms": (t_done - t_run).as_secs_f64() * 1e3,
            "driver": out.timings,
            "render": render_note,
            "exit": code,
        });
        fs::write(p, serde_json::to_vec(&s)?)?;
    }
    Ok(code)
}

/// The observer backstop. The observer sees every process's reads (native
/// code, child processes, worker threads, whatever the hooks miss) but
/// cannot tell which test file caused one, so what the hooks did not
/// account for goes to every file. Covered: a path some key names, or one
/// inside a recorded package. Directory listings are not folded in: the
/// only listing the hooks leave out on purpose is vitest's glob for test
/// files, whose result Quod takes from vitest itself.
fn absorb(root: &Path, dir: &Path, code: &Path, manifest: &Path) -> Result<u8> {
    let ledger_path = dir.join("ledger.json");
    let mut ledger: Ledger =
        serde_json::from_slice(&fs::read(&ledger_path).context("no ledger to absorb into")?)?;
    let text = fs::read_to_string(manifest)?;
    let json = &text[text.find('{').context("not a manifest")?..];
    let m: serde_json::Value = serde_json::from_str(json)?;
    let entries = m["entries"]
        .as_array()
        .context("manifest without entries")?;
    let abs = |p: &str| -> String {
        if p.starts_with('/') {
            p.to_string()
        } else {
            root.join(p).display().to_string()
        }
    };
    let mut covered: HashSet<String> = HashSet::new();
    let mut packages: Vec<String> = Vec::new();
    for k in &ledger.keys {
        if let Some((kind, p)) = k.split_once(':') {
            let a = abs(p);
            if kind == "p" {
                packages.push(format!("{a}/"));
            }
            covered.insert(a);
        }
    }
    packages.sort();
    let in_package = |a: &str| {
        let i = packages.partition_point(|p| p.as_str() <= a);
        i > 0 && a.starts_with(packages[i - 1].as_str())
    };
    let (dir_s, code_s) = (
        format!("{}/", dir.display()),
        format!("{}/", code.display()),
    );
    let mut add: BTreeMap<String, &str> = BTreeMap::new();
    let (mut seen, mut skipped_dirs) = (0usize, 0usize);
    for e in entries {
        let (Some(p), Some(k)) = (e["p"].as_str(), e["k"].as_str()) else {
            continue;
        };
        seen += 1;
        let a = abs(p);
        if a.starts_with(&dir_s)
            || a.starts_with(&code_s)
            || ["/proc/", "/sys/", "/dev/"]
                .iter()
                .any(|x| a.starts_with(x))
        {
            continue;
        }
        if k == "d" {
            skipped_dirs += 1;
            continue;
        }
        if covered.contains(&a) || in_package(&a) {
            continue;
        }
        if a.contains("/node_modules/.vite/") || a.contains("/node_modules/.vitest/") {
            continue;
        }
        // Under node_modules, an installed package counts (whole) when a
        // file of it was read. A lookup that found nothing, or a link the
        // run followed, is keyed by existence only (`e:`: absent, or the
        // link's target): an optional package that appears later, or a
        // link that moves, re-runs; content churn of packages it never
        // read does not (keying those whole made any reinstall a full run).
        if a.contains("/node_modules/") && k != "f" {
            add.insert(relativize(root, &format!("e:{a}")), k);
            continue;
        }
        let key = if let Some(i) = a.rfind("/node_modules/") {
            let rest = &a[i + 14..];
            let mut seg = rest.split('/');
            let first = seg.next().unwrap_or("");
            let name = if first.starts_with('@') {
                format!("{first}/{}", seg.next().unwrap_or(""))
            } else {
                first.to_string()
            };
            if name.is_empty() || name.starts_with('.') {
                // the package manager's or a tool's (record.mjs), not a package
                continue;
            } else {
                format!("p:{}{name}", &a[..i + 14])
            }
        } else {
            format!("{}:{a}", if k == "f" { "f" } else { "e" })
        };
        add.insert(relativize(root, &key), k);
    }
    let mut intern: HashMap<String, u32> = ledger
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (k.clone(), i as u32))
        .collect();
    let mut new = Vec::new();
    for k in add.keys() {
        let i = *intern.entry(k.clone()).or_insert_with(|| {
            ledger.keys.push(k.clone());
            ledger.states.push(String::new());
            ledger.idents.push(None);
            (ledger.keys.len() - 1) as u32
        });
        if !ledger.global.contains(&i) {
            ledger.global.push(i);
            new.push(i);
        }
    }
    let st = states(root, &ledger, &new, None);
    for (k, (s, id)) in new.iter().zip(st) {
        ledger.states[*k as usize] = s;
        ledger.idents[*k as usize] = id;
    }
    ledger.global.sort_unstable();
    let tmp = dir.join("ledger.json.tmp");
    fs::write(&tmp, serde_json::to_vec(&ledger)?)?;
    fs::rename(&tmp, &ledger_path)?;
    let by_kind = add
        .keys()
        .fold(BTreeMap::<&str, usize>::new(), |mut acc, k| {
            *acc.entry(k.split_once(':').map_or("", |x| x.0))
                .or_default() += 1;
            acc
        });
    eprintln!(
        "quod vitest: absorbed the observer's manifest: {seen} accesses, {} not accounted for by the hooks became shared inputs {by_kind:?} ({skipped_dirs} directory listings left out)",
        new.len()
    );
    for k in add
        .iter()
        .filter(|(_, k)| **k == "f")
        .map(|(p, _)| p)
        .take(40)
    {
        eprintln!("quod vitest:   + {k}");
    }
    Ok(0)
}

/// `quod vitest --plan FILE`, before the dependencies are installed: the
/// selection with every input under the workspace's node_modules taken as
/// recorded, which holds only when what a frozen install depends on is
/// what it was at the record (`install_key`). Writes `done` (and emits the
/// rendered full replay) or `run` to FILE.
fn plan(
    a: &VitestArgs,
    pf: &Path,
    root: &Path,
    dir: &Path,
    ledger: &Ledger,
    full: bool,
) -> Result<u8> {
    let t0 = Instant::now();
    let verdict = |why: String| -> Result<u8> {
        fs::write(pf, "run\n")?;
        eprintln!(
            "quod vitest --plan: install and run ({why}; {} ms)",
            t0.elapsed().as_millis()
        );
        Ok(0)
    };
    if full {
        return verdict("--full".into());
    }
    if a.no_fast || std::env::var("QUOD_VITEST_NO_FAST").is_ok_and(|v| v == "1") {
        return verdict("--no-fast".into());
    }
    if ledger.files.is_empty() || ledger.render.is_none() || ledger.install.is_empty() {
        return verdict("no recording with a rendered full replay and install inputs".into());
    }
    let Some(node) = node_version(&a.node) else {
        return verdict("node --version failed".into());
    };
    let w = walk(root, dir);
    match install_key(root, &w, &node) {
        None => {
            return verdict("install inputs unknown (a pnpm lockfile and pnpm are needed)".into());
        }
        Some(k) if k != ledger.install => return verdict("install inputs changed".into()),
        Some(_) => {}
    }
    let live = ledger.live();
    let check: Vec<u32> = live
        .iter()
        .copied()
        .filter(|k| !installed(&ledger.keys[*k as usize]))
        .collect();
    let cur = states(root, ledger, &check, None);
    let now: HashMap<u32, String> = check
        .iter()
        .copied()
        .zip(cur.into_iter().map(|s| s.0))
        .collect();
    let state_now = |k: u32| -> &str {
        now.get(&k)
            .map_or(ledger.states[k as usize].as_str(), |s| s.as_str())
    };
    if ledger.args != keyed_args(&a.args) {
        return verdict("other vitest arguments".into());
    }
    if let Some(k) = ledger
        .global
        .iter()
        .find(|k| state_now(**k) != ledger.states[**k as usize])
    {
        return verdict(format!(
            "shared input changed: {}",
            ledger.keys[*k as usize]
        ));
    }
    for (id, rec) in &ledger.files {
        if rec.quarantined || !(rec.state == "pass" || rec.state == "skip") {
            return verdict(format!("{id} must run"));
        }
        let moved = rec.deps.iter().find(|d| {
            let seen = rec.pinned.get(d).unwrap_or(&ledger.states[**d as usize]);
            state_now(**d) != seen.as_str()
        });
        if let Some(d) = moved {
            return verdict(format!(
                "{} changed ({})",
                ledger.keys[*d as usize], rec.file
            ));
        }
    }
    match emit_render(root, dir, ledger, &a.args, &node, &w.hash) {
        Ok(code) => {
            fs::write(pf, "done\n")?;
            eprintln!(
                "quod vitest --plan: every test file replays without installing; emitted the recorded full replay ({} inputs checked, {} ms)",
                check.len(),
                t0.elapsed().as_millis()
            );
            if let Some(p) = &a.summary {
                let s = serde_json::json!({
                    "recorded_files": ledger.files.len(), "ran": 0, "replayed": ledger.files.len(),
                    "plan": "done", "fast": true, "inputs_hashed": check.len(),
                    "select_ms": t0.elapsed().as_secs_f64() * 1e3, "vitest_ms": 0.0, "exit": code,
                });
                fs::write(p, serde_json::to_vec(&s)?)?;
            }
            Ok(code)
        }
        Err(why) => verdict(why),
    }
}

fn replace_bytes(hay: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(hay.len());
    let mut i = 0;
    while i < hay.len() {
        if !from.is_empty() && hay[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

/// A key the install determines: a path under the workspace's node_modules.
fn installed(key: &str) -> bool {
    let Some((_, p)) = key.split_once(':') else {
        return false;
    };
    !p.starts_with('/')
        && (p == "node_modules"
            || p.starts_with("node_modules/")
            || p.contains("/node_modules/")
            || p.ends_with("/node_modules"))
}

/// Every file is recorded and replayable: a full replay can be rendered.
fn everything_recorded(l: &Ledger) -> bool {
    !l.all_files.is_empty()
        && l.all_files.len() == l.files.len()
        && l.all_files.iter().all(|id| {
            l.files
                .get(id)
                .is_some_and(|r| !r.quarantined && (r.state == "pass" || r.state == "skip"))
        })
}

/// The ledger's recorded results, as a render depends on them.
fn render_sig(l: &Ledger) -> String {
    let mut h = blake3::Hasher::new();
    for (id, r) in &l.files {
        let runs = r.runs.to_string();
        for x in [
            id.as_str(),
            r.project.as_str(),
            r.result.as_str(),
            r.state.as_str(),
            runs.as_str(),
        ] {
            h.update(x.as_bytes());
            h.update(b"\0");
        }
        h.update(b"\n");
    }
    for id in &l.all_files {
        h.update(id.as_bytes());
        h.update(b"\n");
    }
    h.finalize().to_hex().to_string()
}

/// This quod's driver code and version.
fn code_hash() -> String {
    let mut h = blake3::Hasher::new();
    for s in [
        DRIVER,
        SETUP,
        RECORD,
        COVERAGE,
        NARROW,
        PRELOAD,
        env!("CARGO_PKG_VERSION"),
    ] {
        h.update(s.as_bytes());
        h.update(b"\0");
    }
    h.finalize().to_hex()[..16].to_string()
}

/// `process.platform-process.arch` as node names them.
fn platform() -> Option<String> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    Some(format!("{os}-{arch}"))
}

fn node_version(node: &str) -> Option<String> {
    let o = Command::new(node).arg("--version").output().ok()?;
    let v = String::from_utf8(o.stdout).ok()?.trim().to_string();
    (o.status.success() && v.starts_with('v')).then_some(v)
}

/// The installed pnpm's version: the `pnpm` on PATH, resolved to its
/// package (a shim of another package manager is not pnpm: unknown).
fn pnpm_version() -> Option<String> {
    let path = std::env::var_os("PATH")?;
    let bin = std::env::split_paths(&path)
        .map(|d| d.join("pnpm"))
        .find(|p| p.is_file())?;
    let mut d = fs::canonicalize(bin).ok()?;
    for _ in 0..6 {
        d = d.parent()?.to_path_buf();
        if let Ok(b) = fs::read(d.join("package.json")) {
            let v: serde_json::Value = serde_json::from_slice(&b).ok()?;
            return (v["name"] == "pnpm").then(|| v["version"].as_str().map(String::from))?;
        }
    }
    None
}

/// The workspace's paths: every name under the root but node_modules,
/// .git and the ledger's own directory, with its type (a link with its
/// target). What vitest's glob for test files can see, so the same sum
/// means the same test files. A symlink to a directory leaves it unsummed.
struct Walk {
    hash: String,
    /// The files a package manager's install depends on.
    install: Vec<PathBuf>,
}

const INSTALL_FILES: &[&str] = &[
    "package.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
    ".npmrc",
    ".pnpmfile.cjs",
    ".pnpmfile.mjs",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    ".yarnrc",
    ".yarnrc.yml",
    "bun.lock",
    "bun.lockb",
];

fn walk(root: &Path, dir: &Path) -> Walk {
    let mut names = Vec::new();
    let mut install = Vec::new();
    let mut ok = true;
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(rd) = fs::read_dir(root.join(&rel)) else {
            ok = false;
            break;
        };
        for e in rd.flatten() {
            let name = e.file_name();
            let r = rel.join(&name);
            if e.path() == dir {
                continue;
            }
            let Ok(t) = e.file_type() else {
                ok = false;
                continue;
            };
            if t.is_dir() {
                if name == "node_modules" || name == ".git" {
                    continue;
                }
                names.push(format!("{}\0d", r.display()));
                stack.push(r);
            } else if t.is_symlink() {
                if e.path().is_dir() {
                    ok = false;
                }
                let target =
                    fs::read_link(e.path()).map_or(String::new(), |t| t.display().to_string());
                names.push(format!("{}\0l\0{target}", r.display()));
            } else {
                if name.to_str().is_some_and(|n| INSTALL_FILES.contains(&n)) {
                    install.push(r.clone());
                }
                names.push(format!(
                    "{}\0{}",
                    r.display(),
                    if t.is_file() { 'f' } else { 'o' }
                ));
            }
        }
    }
    names.sort_unstable();
    install.sort();
    Walk {
        hash: if ok {
            blake3::hash(names.join("\n").as_bytes())
                .to_hex()
                .to_string()
        } else {
            String::new()
        },
        install,
    }
}

/// What a frozen pnpm install produces depends on: the lockfile and every
/// install file in the workspace (whole), the npm/pnpm configuration in the
/// environment and the user's .npmrc, node, the platform, pnpm, and this
/// quod. None when it cannot be known (not pnpm, pnpm not found).
fn install_key(root: &Path, w: &Walk, node: &str) -> Option<String> {
    if w.hash.is_empty() || !root.join("pnpm-lock.yaml").is_file() {
        return None;
    }
    let pnpm = pnpm_version()?;
    let mut h = blake3::Hasher::new();
    for x in [node.to_string(), platform()?, pnpm, code_hash()] {
        h.update(x.as_bytes());
        h.update(b"\0");
    }
    for rel in &w.install {
        h.update(rel.display().to_string().as_bytes());
        h.update(b"\0");
        h.update(blake3::hash(&fs::read(root.join(rel)).ok()?).as_bytes());
    }
    if let Some(home) = std::env::var_os("HOME") {
        h.update(b"~/.npmrc\0");
        h.update(
            blake3::hash(&fs::read(Path::new(&home).join(".npmrc")).unwrap_or_default()).as_bytes(),
        );
    }
    let mut vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            let l = k.to_ascii_lowercase();
            l.starts_with("npm_config_") || l.starts_with("pnpm_") || l.starts_with("corepack_")
        })
        .collect();
    vars.sort();
    for (k, v) in vars {
        h.update(format!("{k}={v}\0").as_bytes());
    }
    Some(h.finalize().to_hex().to_string())
}

/// Shadow side: replay every file through vitest once more (no tests run)
/// and keep what its reporters printed and wrote.
fn render(
    a: &VitestArgs,
    root: &Path,
    dir: &Path,
    code_dir: &Path,
    ledger: &Ledger,
) -> std::result::Result<Render, String> {
    let rd = dir.join("render");
    let _ = fs::remove_dir_all(&rd);
    fs::create_dir_all(&rd).map_err(|e| e.to_string())?;
    let replay: BTreeMap<&String, &String> =
        ledger.files.iter().map(|(id, r)| (id, &r.result)).collect();
    let plan = serde_json::json!({
        "stateDir": dir, "replay": replay, "audit": [], "record": false,
        "render": rd, "expect": ledger.facts,
    });
    let (pp, op) = (rd.join("plan.json"), rd.join("out.json"));
    fs::write(&pp, serde_json::to_vec(&plan).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let file = |n: &str| fs::File::create(rd.join(n)).map_err(|e| e.to_string());
    let status = without_credentials(&mut Command::new(&a.node))
        .arg(code_dir.join("driver.mjs"))
        .arg(&pp)
        .arg(&op)
        .args(&a.args)
        .env("QUOD_VITEST_QUIET", "1")
        .stdin(std::process::Stdio::null())
        .stdout(file("stdout")?)
        .stderr(file("stderr")?)
        .status()
        .map_err(|e| e.to_string())?;
    let out: Out = fs::read(&op)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("the render left no record")?;
    let r = out.render.unwrap_or_default();
    if r["ok"] != true {
        return Err(format!(
            "render: {}",
            r["reason"].as_str().unwrap_or("not supported")
        ));
    }
    if out.interrupted || !out.ran.is_empty() || out.replayed.len() != ledger.files.len() {
        return Err(format!(
            "the render ran {} files and replayed {} of {}",
            out.ran.len(),
            out.replayed.len(),
            ledger.files.len()
        ));
    }
    let mut all = out.all_files.unwrap_or_default();
    all.sort();
    if all != ledger.all_files {
        return Err("the render saw other test files".into());
    }
    let outputs = r["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|o| {
            let to = o["to"].as_str().unwrap_or_default();
            RenderOut {
                file: o["file"].as_str().unwrap_or_default().to_string(),
                to: Path::new(to)
                    .strip_prefix(root)
                    .map_or(to.to_string(), |p| p.display().to_string()),
                append: o["append"] == true,
                env: o["env"].as_str().map(String::from),
            }
        })
        .collect();
    Ok(Render {
        sig: render_sig(ledger),
        args: a.args.clone(),
        code: code_hash(),
        exit: status.code().map_or(1, |c| c.clamp(0, 255) as u8),
        outputs,
        dir: rd.display().to_string(),
    })
}

/// Critical side: when every file replays, emit the render of exactly this
/// ledger instead of starting vitest, if it still says what vitest would
/// print: same results, arguments, quod, node, platform and test files.
fn emit_render(
    root: &Path,
    dir: &Path,
    ledger: &Ledger,
    args: &[String],
    node: &str,
    tree: &str,
) -> std::result::Result<u8, String> {
    let r = ledger.render.as_ref().ok_or("the recording has none")?;
    if r.sig != render_sig(ledger) {
        return Err("it is of other results".into());
    }
    if r.args != args {
        return Err("other vitest arguments".into());
    }
    if r.code != code_hash() {
        return Err("another quod made it".into());
    }
    if ledger.facts.get("node").map(String::as_str) != Some(node) {
        return Err("another node".into());
    }
    if ledger.facts.get("platform") != platform().as_ref() {
        return Err("another platform".into());
    }
    if tree.is_empty() || ledger.tree != tree {
        return Err("paths were added, removed or renamed".into());
    }
    if !everything_recorded(ledger) {
        return Err("not every test file is recorded".into());
    }
    let rd = dir.join("render");
    let read = |n: &str| fs::read(rd.join(n)).map_err(|e| format!("{n}: {e}"));
    let (mut stdout, mut stderr) = (read("stdout")?, read("stderr")?);
    let mut outs = Vec::new();
    for o in &r.outputs {
        let to = match &o.env {
            Some(e) => match std::env::var_os(e) {
                Some(v) if !v.is_empty() => PathBuf::from(v),
                _ => continue,
            },
            None => root.join(&o.to),
        };
        // "JSON report written to <render dir>/out-0": the path it goes to
        if !r.dir.is_empty() {
            let from = format!("{}/{}", r.dir, o.file);
            let to = to.display().to_string();
            stdout = replace_bytes(&stdout, from.as_bytes(), to.as_bytes());
            stderr = replace_bytes(&stderr, from.as_bytes(), to.as_bytes());
        }
        outs.push((to, o.append, read(&o.file)?));
    }
    use std::io::Write;
    for (to, append, bytes) in outs {
        if let Some(p) = to.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let res = if append {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&to)
                .and_then(|mut f| f.write_all(&bytes))
        } else {
            fs::write(&to, &bytes)
        };
        res.map_err(|e| format!("{}: {e}", to.display()))?;
    }
    // Printed last: nothing can fail after the first byte (a failure before
    // it falls back to vitest, which writes every output again).
    let _ = std::io::stdout().write_all(&stdout);
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().write_all(&stderr);
    Ok(r.exit)
}

impl Ledger {
    /// Every key the global inputs or a record refer to, once.
    fn live(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.global.clone();
        for r in self.files.values() {
            v.extend_from_slice(&r.deps);
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Drop keys nothing refers to.
    fn compact(&mut self) {
        let live = self.live();
        let mut map = vec![u32::MAX; self.keys.len()];
        let (mut keys, mut states, mut idents) = (Vec::new(), Vec::new(), Vec::new());
        for k in live {
            map[k as usize] = keys.len() as u32;
            keys.push(std::mem::take(&mut self.keys[k as usize]));
            states.push(std::mem::take(&mut self.states[k as usize]));
            idents.push(self.idents[k as usize]);
        }
        (self.keys, self.states, self.idents) = (keys, states, idents);
        for g in &mut self.global {
            *g = map[*g as usize];
        }
        for r in self.files.values_mut() {
            for d in &mut r.deps {
                *d = map[*d as usize];
            }
            r.cov = std::mem::take(&mut r.cov)
                .into_iter()
                .map(|(k, v)| (map[k as usize], v))
                .collect();
            r.pinned = std::mem::take(&mut r.pinned)
                .into_iter()
                .map(|(k, v)| (map[k as usize], v))
                .collect();
        }
    }
}

/// `kind:/root/x` becomes `kind:x`; other keys are kept.
fn relativize(root: &Path, k: &str) -> String {
    let Some((kind, p)) = k.split_once(':') else {
        return k.to_string();
    };
    match Path::new(p).strip_prefix(root) {
        Ok(rel) if p.starts_with('/') => format!("{kind}:{}", rel.display()),
        _ => k.to_string(),
    }
}

/// The current state of each key, in parallel.
fn states(
    root: &Path,
    ledger: &Ledger,
    keys: &[u32],
    blobs: Option<&Path>,
) -> Vec<(String, Option<[i64; 6]>)> {
    let started = wall_ns();
    let envall = std::sync::OnceLock::new();
    par_map(keys, |k| {
        let i = *k as usize;
        let cached = ledger
            .idents
            .get(i)
            .copied()
            .flatten()
            .map(|id| (id, ledger.states[i].as_str()));
        state(root, &ledger.keys[i], cached, started, &envall, blobs)
    })
}

fn state(
    root: &Path,
    key: &str,
    cached: Option<([i64; 6], &str)>,
    started: i128,
    envall: &std::sync::OnceLock<String>,
    blobs: Option<&Path>,
) -> (String, Option<[i64; 6]>) {
    let (kind, p) = key.split_once(':').unwrap_or((key, ""));
    let path = || -> PathBuf {
        if p.starts_with('/') {
            PathBuf::from(p)
        } else {
            root.join(p)
        }
    };
    match kind {
        // The tools enumerate the environment (vitest copies it to its
        // workers), reading every name: a per-run name is not an input of the
        // main process. A test file that reads one is not recorded (ingest).
        "env" if volatile_env(p) => ("volatile".into(), None),
        "env" => (
            std::env::var_os(p).map_or("unset".into(), |v| {
                let v = yarn_stable(&v.to_string_lossy());
                blake3::hash(v.as_bytes()).to_hex()[..32].to_string()
            }),
            None,
        ),
        "envall" => (envall.get_or_init(|| env_all(false)).clone(), None),
        "envtool" => (env_all(true), None),
        "e" => (exists(&path()), None),
        "d" => (listing(&path(), false), None),
        "rj" => (listing_of(&path(), true, true), None),
        "pjf" => {
            let (fp, fields) = p.split_once('\0').unwrap_or((p, "[]"));
            let fp = if fp.starts_with('/') {
                PathBuf::from(fp)
            } else {
                root.join(fp)
            };
            (pj_fields(&fp, fields), None)
        }
        "r" => (listing(&path(), true), None),
        "rc" => (content_tree(&path()), None),
        "f" | "pj" | "p" => {
            let mut path = path();
            // npm's layout names no version in a package's path: what the
            // lockfile says was installed there (version, source, integrity)
            // joins its package.json, so a git or tarball dependency that
            // moved without a version bump counts too.
            let lock = if kind == "p" { npm_locked(&path) } else { None };
            if kind == "p" {
                path.push("package.json");
            }
            let Ok(m) = fs::metadata(&path) else {
                return ("absent".into(), None);
            };
            if m.is_dir() {
                return ("dir".into(), None);
            }
            if !m.is_file() {
                return ("other".into(), None);
            }
            let id = ident(&m);
            if let Some((cid, cs)) = cached
                && cid == id
                && !cs.is_empty()
                && cs.split_once('+').map(|x| x.1) == lock.as_deref()
            {
                return (cs.to_string(), Some(id));
            }
            let Ok(bytes) = fs::read(&path) else {
                return ("unreadable".into(), None);
            };
            let h = if kind == "pj" {
                pj_hash(&bytes)
            } else {
                blake3::hash(&bytes).to_hex().to_string()
            };
            let h = match &lock {
                Some(l) => format!("{h}+{l}"),
                None => h,
            };
            // Workspace source is kept by content, for function-level
            // narrowing to diff a record's version against the current one.
            if let Some(b) = blobs
                && kind == "f"
                && !p.starts_with('/')
                && bytes.len() < (2 << 20)
                && is_source(p)
            {
                keep_blob(b, &h, &bytes);
            }
            // Keep the identity only for a file settled before the read and
            // unchanged by it.
            let settled = started - (id[3].max(id[4]) as i128) > SETTLE_NS;
            let same = fs::metadata(&path).is_ok_and(|m2| ident(&m2) == id);
            (h, (settled && same).then_some(id))
        }
        _ => (format!("unknown kind {kind}"), None),
    }
}

/// The per-run variables present (the commit, the run id, the event file,
/// the runner's per-step paths, Quod's own): a recording worker lists the
/// environment without them (record.mjs).
fn hidden_env() -> String {
    let mut v: Vec<String> = std::env::vars_os()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .filter(|k| volatile_env(k))
        .collect();
    v.push("QUOD_VITEST_HIDE".into());
    v.sort();
    v.join(",")
}

/// The vitest the project resolves from `root` (the nearest
/// node_modules/vitest up from it): its version and its CLI entry.
fn vitest_install(root: &Path) -> Option<(String, PathBuf)> {
    let d = root
        .ancestors()
        .map(|d| d.join("node_modules").join("vitest"))
        .find(|d| d.join("package.json").is_file())?;
    let pj: serde_json::Value =
        serde_json::from_slice(&fs::read(d.join("package.json")).ok()?).ok()?;
    let entry = d.join("vitest.mjs");
    let version = pj["version"].as_str()?.to_string();
    entry.is_file().then_some((version, entry))
}

/// vitest 3.2 or later.
fn vitest_supported(version: &str) -> bool {
    let mut n = version.split(['.', '-']).map(|x| x.parse::<u32>().ok());
    match (n.next().flatten(), n.next().flatten()) {
        (Some(major), Some(minor)) => major > 3 || (major == 3 && minor >= 2),
        _ => true,
    }
}

/// `--report` for a rendered full replay: every file's recorded tests, in
/// the shape the driver writes (vitest's JSON report: per file, each test's
/// full name and status).
fn report_from_ledger(dir: &Path, ledger: &Ledger) -> serde_json::Value {
    fn walk(t: &serde_json::Value, names: &mut Vec<String>, out: &mut Vec<serde_json::Value>) {
        names.push(t["name"].as_str().unwrap_or("").to_string());
        if t["type"] == "test" {
            let st = t["result"]["state"]
                .as_str()
                .or(t["mode"].as_str())
                .unwrap_or("");
            let status = match st {
                "pass" => "passed",
                "fail" => "failed",
                "skip" | "todo" => "skipped",
                _ => "pending",
            };
            out.push(serde_json::json!({
                "ancestorTitles": [], "title": names.join(" > "), "status": status,
            }));
        }
        for c in t["tasks"].as_array().into_iter().flatten() {
            walk(c, names, out);
        }
        names.pop();
    }
    let mut results = Vec::new();
    for rec in ledger.files.values() {
        let file: Option<serde_json::Value> = fs::read(dir.join(&rec.result))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        if let Some(file) = file {
            let mut tests = Vec::new();
            for c in file["tasks"].as_array().into_iter().flatten() {
                walk(c, &mut Vec::new(), &mut tests);
            }
            results.push(serde_json::json!({
                "name": file["filepath"], "project": rec.project, "assertionResults": tests,
            }));
        }
    }
    serde_json::json!({ "testResults": results })
}

/// Recordings in a row that must match before a quarantined file may
/// replay again (QUOD_VITEST_REPROVE, default 3: the team's "good runs
/// needed again").
fn reprove() -> u32 {
    std::env::var("QUOD_VITEST_REPROVE")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(3)
}

/// Credentials quod itself may hold (Quod Cloud's: GitHub's OIDC request
/// token, a server token; the runner's own). vitest, the tests and so the
/// recorded results never see them.
const CREDENTIALS: &[&str] = &[
    "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
    "ACTIONS_ID_TOKEN_REQUEST_URL",
    "ACTIONS_RUNTIME_TOKEN",
    "ACTIONS_RESULTS_URL",
    "QUOD_SERVER_TOKEN",
];

fn without_credentials(c: &mut Command) -> &mut Command {
    for k in CREDENTIALS {
        c.env_remove(k);
    }
    c
}

/// A credential's value found in a file under `dir` (a ledger about to
/// leave the machine): its name. Values shorter than 16 bytes are not
/// searched (no real credential is that short; a short one would match
/// by chance).
pub(crate) fn credential_in(dir: &Path) -> Option<&'static str> {
    let secrets: Vec<(&'static str, Vec<u8>)> = CREDENTIALS
        .iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (*k, v.into_encoded_bytes())))
        .filter(|(_, v)| v.len() >= 16)
        .collect();
    if secrets.is_empty() {
        return None;
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(p);
            } else if let Ok(b) = fs::read(&p) {
                for (k, v) in &secrets {
                    if b.windows(v.len()).any(|w| w == v.as_slice()) {
                        return Some(k);
                    }
                }
            }
        }
    }
    None
}

/// May a large run take every CPU (vitest's default leaves one)? Not when a
/// task runner (turbo, nx, lerna) may be running other packages' suites at
/// the same time: on a monorepo's CI that oversubscribed the runner and a
/// test with a 5 s timeout timed out (class-management, turbo with five
/// vitest suites at once). Not when QUOD_VITEST_ALL_CPUS=0 either.
fn alone() -> bool {
    const RUNNERS: [&str; 4] = [
        "TURBO_HASH",
        "NX_TASK_HASH",
        "NX_TASK_TARGET_PROJECT",
        "LERNA_PACKAGE_NAME",
    ];
    std::env::var("QUOD_VITEST_ALL_CPUS").map_or(true, |v| v != "0")
        && RUNNERS.iter().all(|k| std::env::var_os(k).is_none())
}

/// An installed package's entry in the npm lockfile of the workspace it is
/// in (the nearest package-lock.json or npm-shrinkwrap.json up from it, by
/// its path from there): a digest of its version, `resolved` and
/// `integrity`, or `unlisted`. None when no npm lockfile is found (pnpm and
/// yarn name versions in the path or need none).
fn npm_locked(pkg: &Path) -> Option<String> {
    type Locks = std::sync::Mutex<HashMap<PathBuf, Option<std::sync::Arc<serde_json::Value>>>>;
    static LOCKS: std::sync::OnceLock<Locks> = std::sync::OnceLock::new();
    let mut found = None;
    for d in pkg.ancestors().skip(1) {
        if let Some(f) = ["npm-shrinkwrap.json", "package-lock.json"]
            .iter()
            .map(|n| d.join(n))
            .find(|f| f.is_file())
        {
            found = Some((d, f));
            break;
        }
        if d.join(".git").exists() {
            break;
        }
    }
    let (base, file) = found?;
    let rel = pkg.strip_prefix(base).ok()?.to_str()?.to_string();
    let lock = {
        let mut m = LOCKS.get_or_init(Default::default).lock().ok()?;
        m.entry(file.clone())
            .or_insert_with(|| {
                fs::read(&file)
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .map(std::sync::Arc::new)
            })
            .clone()
    };
    let Some(lock) = lock else {
        return Some("unreadable".into());
    };
    let e = &lock["packages"][rel.as_str()];
    if e.is_null() {
        return Some("unlisted".into());
    }
    let mut h = blake3::Hasher::new();
    for f in ["version", "resolved", "integrity"] {
        h.update(e[f].as_str().unwrap_or("").as_bytes());
        h.update(b"\0");
    }
    Some(h.finalize().to_hex()[..16].to_string())
}

fn is_source(p: &str) -> bool {
    [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"]
        .iter()
        .any(|e| p.ends_with(e))
}

fn keep_blob(dir: &Path, hash: &str, bytes: &[u8]) {
    let p = dir.join(hash);
    if p.exists() {
        return;
    }
    let tmp = dir.join(format!("{hash}.{}.tmp", std::process::id()));
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::rename(&tmp, &p);
    }
}

fn ident(m: &fs::Metadata) -> [i64; 6] {
    [
        m.dev() as i64,
        m.ino() as i64,
        m.size() as i64,
        m.mtime() * 1_000_000_000 + m.mtime_nsec(),
        m.ctime() * 1_000_000_000 + m.ctime_nsec(),
        i64::from(m.mode()),
    ]
}

fn wall_ns() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i128)
}

fn exists(p: &Path) -> String {
    match fs::symlink_metadata(p) {
        Err(_) => "absent".into(),
        Ok(m) if m.file_type().is_symlink() => format!(
            "link:{}",
            fs::read_link(p).map_or(String::new(), |t| t.display().to_string())
        ),
        Ok(m) if m.is_dir() => "dir".into(),
        Ok(m) if m.is_file() => "file".into(),
        Ok(_) => "other".into(),
    }
}

/// Everything under a path: names, types, link targets and file contents
/// (a watched path; `fs.watch` sees any of it change).
fn content_tree(p: &Path) -> String {
    let Ok(m) = fs::symlink_metadata(p) else {
        return "absent".into();
    };
    if !m.is_dir() {
        return match fs::read(p) {
            Ok(b) => blake3::hash(&b).to_hex().to_string(),
            Err(_) => exists(p),
        };
    }
    let mut h = blake3::Hasher::new();
    let mut stack = vec![PathBuf::new()];
    let mut entries = Vec::new();
    while let Some(rel) = stack.pop() {
        let Ok(rd) = fs::read_dir(p.join(&rel)) else {
            continue;
        };
        for e in rd.flatten() {
            let r = rel.join(e.file_name());
            match e.file_type() {
                Ok(t) if t.is_dir() => {
                    entries.push((r.clone(), "d".to_string()));
                    stack.push(r);
                }
                Ok(t) if t.is_symlink() => entries.push((r.clone(), exists(&p.join(&r)))),
                Ok(t) if t.is_file() => {
                    let c = fs::read(p.join(&r)).map_or("unreadable".into(), |b| {
                        blake3::hash(&b).to_hex().to_string()
                    });
                    entries.push((r, c));
                }
                _ => entries.push((r, "other".into())),
            }
        }
    }
    entries.sort();
    for (r, c) in entries {
        h.update(r.display().to_string().as_bytes());
        h.update(b"\0");
        h.update(c.as_bytes());
        h.update(b"\n");
    }
    h.finalize().to_hex().to_string()
}

/// A directory's names and types (recursively: every path under it).
fn listing(p: &Path, recursive: bool) -> String {
    listing_of(p, recursive, false)
}

/// A directory's listing; `json_only`: only its `.json` files, not under
/// node_modules or .git (`rj:`, what tsconfck's walk for tsconfig files
/// finds: it descends into directories but those, not through links, and
/// keeps files by name).
fn listing_of(p: &Path, recursive: bool, json_only: bool) -> String {
    if !p.is_dir() {
        return if p.exists() {
            "notdir".into()
        } else {
            "absent".into()
        };
    }
    let mut names = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(rd) = fs::read_dir(p.join(&rel)) else {
            names.push(format!("{}\0unreadable", rel.display()));
            continue;
        };
        for e in rd.flatten() {
            let t = e.file_type().ok();
            let kind = match t {
                Some(t) if t.is_dir() => 'd',
                Some(t) if t.is_symlink() => 'l',
                Some(t) if t.is_file() => 'f',
                _ => 'o',
            };
            let r = rel.join(e.file_name());
            if json_only {
                let name = e.file_name();
                if kind == 'd' && name != "node_modules" && name != ".git" {
                    stack.push(r);
                } else if kind == 'f' && name.to_string_lossy().ends_with(".json") {
                    names.push(format!("{}\0f", r.display()));
                }
                continue;
            }
            names.push(format!("{}\0{kind}", r.display()));
            if recursive && kind == 'd' {
                stack.push(r);
            }
        }
    }
    names.sort_unstable();
    blake3::hash(names.join("\n").as_bytes())
        .to_hex()
        .to_string()
}

/// The package.json fields module resolution and the tools read (not
/// `packageManager` or `engines`: neither vite nor vitest acts on them, and
/// a bot bumps them often).
const PJ_FIELDS: &[&str] = &[
    "name",
    "type",
    "exports",
    "imports",
    "main",
    "module",
    "browser",
    "jsnext:main",
    "jsnext",
    "workspaces",
    "vite",
    "vitest",
];

/// A workspace package.json as vite's and vitest's own resolvers read it
/// (`pj:` keys): the resolution fields. The dependency lists are not keyed:
/// what is installed is, by the `p:` keys of the packages a run loads. A release's `version` bump or a
/// script change counts for nobody. What reads package.json itself keeps
/// it whole (`f:`): a test's read or import, and the config (read while it
/// loads, or bundled into it).
fn pj_hash(bytes: &[u8]) -> String {
    let Ok(serde_json::Value::Object(v)) = serde_json::from_slice::<serde_json::Value>(bytes)
    else {
        return blake3::hash(bytes).to_hex().to_string();
    };
    let mut kept = serde_json::Map::new();
    for (k, val) in v {
        if PJ_FIELDS.contains(&k.as_str()) {
            kept.insert(k, val);
        }
    }
    let canon = serde_json::to_vec(&kept).unwrap_or_default();
    format!("pj:{}", blake3::hash(&canon).to_hex())
}

/// A package.json the config parsed (`pjf:<path>\0<fields>`, record.mjs):
/// its top-level names in order, and the raw text of each field it read
/// (absent, or the file whole when it is not a JSON object).
fn pj_fields(path: &Path, fields: &str) -> String {
    let Ok(bytes) = fs::read(path) else {
        return if path.exists() {
            "unreadable".into()
        } else {
            "absent".into()
        };
    };
    let wanted: Vec<String> = serde_json::from_str(fields).unwrap_or_default();
    let Some(top) = top_fields(&bytes) else {
        return blake3::hash(&bytes).to_hex().to_string();
    };
    let mut h = blake3::Hasher::new();
    for (k, _) in &top {
        h.update(k.as_bytes());
        h.update(b"\0");
    }
    h.update(b"\n");
    for w in &wanted {
        h.update(w.as_bytes());
        // the last of a repeated name, as JSON.parse keeps
        match top.iter().rev().find(|(k, _)| k == w) {
            Some((_, v)) => {
                h.update(b"=");
                h.update(v);
            }
            None => {
                h.update(b"!absent");
            }
        }
        h.update(b"\0");
    }
    format!("pjf:{}", h.finalize().to_hex())
}

/// A JSON object's top-level names and the raw text of their values, in
/// order; None when the text is not one.
fn top_fields(b: &[u8]) -> Option<Vec<(String, &[u8])>> {
    let ws = |i: &mut usize| {
        while *i < b.len() && matches!(b[*i], b' ' | b'\t' | b'\n' | b'\r') {
            *i += 1;
        }
    };
    // the index just past a string starting at i (a quote)
    let string_end = |mut i: usize| -> Option<usize> {
        i += 1;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 2,
                b'"' => return Some(i + 1),
                _ => i += 1,
            }
        }
        None
    };
    let value_end = |mut i: usize| -> Option<usize> {
        match *b.get(i)? {
            b'"' => string_end(i),
            b'{' | b'[' => {
                let mut depth = 0usize;
                while i < b.len() {
                    match b[i] {
                        b'"' => {
                            i = string_end(i)?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth -= 1;
                            if depth == 0 {
                                return Some(i + 1);
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                None
            }
            _ => {
                while i < b.len()
                    && !matches!(b[i], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
                {
                    i += 1;
                }
                Some(i)
            }
        }
    };
    let mut i = 0;
    ws(&mut i);
    if b.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    let mut out = Vec::new();
    loop {
        ws(&mut i);
        match b.get(i)? {
            b'}' => break,
            b'"' => {}
            _ => return None,
        }
        let ke = string_end(i)?;
        let key: String = serde_json::from_slice(&b[i..ke]).ok()?;
        i = ke;
        ws(&mut i);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i += 1;
        ws(&mut i);
        let ve = value_end(i)?;
        if ve == i {
            return None;
        }
        out.push((key, &b[i..ve]));
        i = ve;
        ws(&mut i);
        match b.get(i)? {
            b',' => i += 1,
            b'}' => break,
            _ => return None,
        }
    }
    i += 1;
    ws(&mut i);
    (i == b.len()).then_some(out)
}

/// vitest's arguments that change only what is printed or written, not a
/// test's result: not part of what a record is keyed on (replays render
/// in whatever reporters this run asks for).
fn keyed_args(args: &[String]) -> Vec<String> {
    const VALUED: &[&str] = &["--reporter", "--outputFile"];
    const FLAGS: &[&str] = &[
        "--silent",
        "--color",
        "--no-color",
        "--hideSkippedTests",
        "--logHeapUsage",
        "--printConsoleTrace",
        "--no-printConsoleTrace",
    ];
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let name = a.split_once('=').map_or(a.as_str(), |x| x.0);
        let base = name.split_once('.').map_or(name, |x| x.0);
        if VALUED.contains(&base) {
            if !a.contains('=') && args.get(i + 1).is_some_and(|n| !n.starts_with('-')) {
                i += 1;
            }
        } else if !FLAGS.contains(&base) {
            out.push(a.clone());
        }
        i += 1;
    }
    out
}

/// An environment value with yarn's per-run directory names made constant.
/// Yarn runs scripts with a directory of its own first on PATH (berry:
/// `<tmp>/xfs-<8 hex>`, also BERRY_BIN_FOLDER and npm_node_execpath; classic:
/// `<tmp>/yarn--<ms>-<random>`), a new one every run, holding wrappers that
/// run yarn's node and yarn itself (each keyed where it is: the node
/// version, the yarn release). The name is not an input.
fn yarn_stable(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    loop {
        let xfs = rest.find("/xfs-");
        let classic = rest.find("/yarn--");
        let (at, skip, ok): (usize, usize, fn(char) -> bool) = match (xfs, classic) {
            (Some(a), Some(b)) if b < a => (b, 7, |c| c.is_ascii_digit() || c == '-' || c == '.'),
            (Some(a), _) => (a, 5, |c| c.is_ascii_hexdigit()),
            (None, Some(b)) => (b, 7, |c| c.is_ascii_digit() || c == '-' || c == '.'),
            (None, None) => break,
        };
        out.push_str(&rest[..at + skip]);
        let tail = &rest[at + skip..];
        let n = tail.find(|c: char| !ok(c)).unwrap_or(tail.len());
        // Only a whole path component made of the generated part.
        if n > 0
            && tail[n..]
                .chars()
                .next()
                .is_none_or(|c| c == '/' || c == ':')
        {
            out.push('*');
        } else {
            out.push_str(&tail[..n]);
        }
        rest = &tail[n..];
    }
    out.push_str(rest);
    out
}

/// Every environment variable but the per-run and per-step ones; for the
/// tools' own enumeration (`envtool:`), those of them `tool_env` names.
fn env_all(tools: bool) -> String {
    let mut vars: Vec<(String, String)> = std::env::vars_os()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                yarn_stable(&v.to_string_lossy()),
            )
        })
        .filter(|(k, _)| !volatile_env(k) && (!tools || tool_env(k)))
        .collect();
    vars.sort();
    let mut h = blake3::Hasher::new();
    for (k, v) in vars {
        h.update(k.as_bytes());
        h.update(b"=");
        h.update(v.as_bytes());
        h.update(b"\0");
    }
    h.finalize().to_hex().to_string()
}

/// The variables node, vite and vitest change their own behaviour on, the
/// part of the environment the tools' copy to the workers (`envtool:`) is
/// keyed on. The rest of a runner's environment (the CI provider's, the
/// image's tool paths and versions: another value on another VM) counts only
/// where some code other than the tools reads it: then it is an `env:` key.
fn tool_env(name: &str) -> bool {
    ["NODE_", "VITE", "LC_", "DEBUG", "UV_"]
        .iter()
        .any(|p| name.starts_with(p))
        || [
            "CI",
            "TEST",
            "TZ",
            "LANG",
            "LANGUAGE",
            "FORCE_COLOR",
            "NO_COLOR",
            "TERM",
            "TMPDIR",
            "HOME",
            "PATH",
        ]
        .contains(&name)
}

/// A per-run or per-step CI variable, or Quod's own.
fn volatile_env(name: &str) -> bool {
    quod_key::unkeyed(name)
        || STEP_VOLATILE.contains(&name)
        || TASK_VOLATILE.contains(&name)
        || name.starts_with("QUOD_")
}

/// What Nx sets per task run: the task's hash (another for any change to
/// its inputs), the run's analytics session, the task's output log path.
/// Not keyed by `envall:`; a test file that reads one by name is not
/// recorded, as for the CI's per-run variables.
const TASK_VOLATILE: &[&str] = &[
    "NX_TASK_HASH",
    "NX_ANALYTICS_SESSION_ID",
    "NX_TERMINAL_OUTPUT_PATH",
];

fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 16);
    if items.len() < 32 || n == 1 {
        return items.iter().map(&f).collect();
    }
    let next = AtomicUsize::new(0);
    let f = &f;
    let next = &next;
    let parts: Vec<Vec<(usize, R)>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..n)
            .map(|_| {
                s.spawn(move || {
                    let mut v = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= items.len() {
                            break;
                        }
                        v.push((i, f(&items[i])));
                    }
                    v
                })
            })
            .collect();
        hs.into_iter()
            .map(|h| h.join().expect("hashing thread panicked"))
            .collect()
    });
    let mut out: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
    for (i, r) in parts.into_iter().flatten() {
        out[i] = Some(r);
    }
    out.into_iter()
        .map(|r| r.expect("every item mapped"))
        .collect()
}

struct Rng(u64);
impl Rng {
    fn new() -> Rng {
        Rng((wall_ns() as u64) | 1)
    }
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// A dependency key in plain words: `f:src/a.ts` is `src/a.ts`, a package
/// is `the package <name>`, a variable `the variable X`.
fn plain_key(k: &str) -> String {
    match k.split_once(':') {
        Some(("f", p)) => p.to_string(),
        Some(("p", p)) => {
            let name = p.rsplit_once("node_modules/").map_or(p, |(_, n)| n);
            format!("the package {name}")
        }
        Some(("env" | "envall", v)) => format!("the variable {v}"),
        Some(("d", p)) => format!("the directory listing of {p}"),
        Some(("rj", p)) => format!("the .json files under {p}"),
        Some(("pjf", p)) => {
            let (f, fields) = p.split_once('\0').unwrap_or((p, ""));
            format!("{f} (its names and {fields})")
        }
        Some(("e", p)) => format!("whether {p} exists"),
        _ => k.to_string(),
    }
}

/// What a run report needs from the selection, gathered before the record
/// changes the ledger.
#[derive(Clone)]
struct Reporting<'a> {
    cloud: Option<&'a crate::vitest_cloud::Cloud>,
    used: Option<quod_wire::testreplay::LedgerUsed>,
    /// Every recorded file: its path and recorded duration (ms).
    recorded: &'a HashMap<String, (String, f64)>,
    why: &'a HashMap<String, String>,
    first_run: bool,
    narrowed: usize,
    t0: Instant,
    select_ms: f64,
}

impl Reporting<'_> {
    fn with_select(&self, t: std::time::Duration) -> Self {
        let mut r = self.clone();
        r.select_ms = t.as_secs_f64() * 1e3;
        r
    }

    /// Sends the report, if --cloud: `ran` is (id, file, state, duration)
    /// per file that ran, `replayed` the ids reused, `audit` the ids the
    /// shadow checked and `mismatched` those whose fresh result differed.
    #[allow(clippy::too_many_arguments)]
    fn send(
        &self,
        kind: &str,
        ran: &[(String, String, String, f64)],
        replayed: &[&String],
        audit: &[String],
        mismatched: &HashSet<String>,
        unrecordable: &HashMap<String, String>,
        exit: Option<u8>,
    ) {
        let Some(c) = self.cloud else { return };
        let saved: f64 = replayed
            .iter()
            .filter_map(|id| self.recorded.get(*id).map(|r| r.1))
            .sum();
        let ran_ms: f64 = ran.iter().map(|r| r.3).sum();
        let file_of = |id: &str| {
            self.recorded
                .get(id)
                .map_or_else(|| id.to_string(), |r| r.0.clone())
        };
        let ran_files = ran
            .iter()
            .map(|(id, file, state, d)| quod_wire::testreplay::RanFile {
                file: relativize_file(file),
                project: String::new(),
                why: self.why.get(id).cloned().unwrap_or_else(|| {
                    if audit.contains(id) {
                        "a check: run again to compare with its reused result".into()
                    } else if self.first_run {
                        "first run (nothing recorded yet)".into()
                    } else if self.recorded.contains_key(id) {
                        "changed".into()
                    } else {
                        "new test file (not recorded yet)".into()
                    }
                }),
                duration_ms: Some(*d),
                state: state.clone(),
                unrecordable: unrecordable.get(id).cloned(),
            })
            .collect();
        // Files the shadow ran to record (not to audit) whose result
        // differed from the check's: quarantined at record time.
        let record_differs = mismatched
            .iter()
            .filter(|id| !audit.iter().any(|a| a == *id))
            .map(|id| {
                ran.iter()
                    .find(|r| &r.0 == id)
                    .map_or_else(|| file_of(id), |r| relativize_file(&r.1))
            })
            .collect();
        let replayed_files = replayed
            .iter()
            .map(|id| quod_wire::testreplay::ReplayedFile {
                file: file_of(id),
                duration_ms: self.recorded.get(*id).map(|r| r.1),
            })
            .collect();
        let audit_files = audit
            .iter()
            .map(|id| quod_wire::testreplay::AuditFile {
                file: file_of(id),
                ok: !mismatched.contains(id),
            })
            .collect();
        let r = quod_wire::testreplay::TestReport {
            tool: "vitest".into(),
            kind: kind.into(),
            total: (ran.len() + replayed.len()) as u64,
            ran: ran.len() as u64,
            replayed: replayed.len() as u64,
            narrowed: self.narrowed.min(replayed.len()) as u64,
            wall_ms: Some(self.t0.elapsed().as_secs_f64() * 1e3),
            select_ms: Some(self.select_ms),
            est_plain_ms: Some(saved + ran_ms),
            saved_ms: Some(saved),
            exit: exit.map(i32::from),
            audited: audit.len() as u64,
            audit_files,
            ran_files,
            ledger: self.used.clone(),
            replayed_files,
            record_differs,
        };
        c.report(&r);
    }
}

/// A test file's path for the report: relative to the working directory
/// when under it.
fn relativize_file(f: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    Path::new(f)
        .strip_prefix(&cwd)
        .map_or_else(|_| f.to_string(), |p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pj_fields_keys_names_and_the_fields_read() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("package.json");
        let st = |s: &str, fields: &str| {
            fs::write(&p, s).unwrap();
            pj_fields(&p, fields)
        };
        let deps = r#"["dependencies"]"#;
        let a = st(
            r#"{"name":"x","version":"1","dependencies":{"a":"1","b":"2"}}"#,
            deps,
        );
        // another version, other whitespace outside the field: the same
        assert_eq!(
            a,
            st(
                "{ \"name\": \"x\", \"version\": \"2\",\n \"dependencies\":{\"a\":\"1\",\"b\":\"2\"} }",
                deps
            )
        );
        // a field read changes, its order changes, a name is added: not
        assert_ne!(
            a,
            st(
                r#"{"name":"x","version":"1","dependencies":{"a":"1","b":"3"}}"#,
                deps
            )
        );
        assert_ne!(
            a,
            st(
                r#"{"name":"x","version":"1","dependencies":{"b":"2","a":"1"}}"#,
                deps
            )
        );
        assert_ne!(
            a,
            st(
                r#"{"name":"x","version":"1","dependencies":{"a":"1","b":"2"},"c":1}"#,
                deps
            )
        );
        // not an object: the bytes
        assert_ne!(a, st("[1]", deps));
        assert_eq!(
            st("{ broken", deps),
            blake3::hash(b"{ broken").to_hex().to_string()
        );
        fs::remove_file(&p).unwrap();
        assert_eq!(pj_fields(&p, deps), "absent");
        assert_eq!(
            top_fields(br#" {"a" : [1, {"}": "]"}], "b\"c":null,"d":-1.5e3} "#)
                .unwrap()
                .iter()
                .map(|(k, v)| format!("{k}={}", String::from_utf8_lossy(v)))
                .collect::<Vec<_>>(),
            vec![r#"a=[1, {"}": "]"}]"#, r#"b"c=null"#, "d=-1.5e3"]
        );
    }

    #[test]
    fn a_json_tree_sees_only_json_files_outside_node_modules() {
        let d = tempfile::tempdir().unwrap();
        let rj = |p: &Path| listing_of(p, true, true);
        let a = rj(d.path());
        // other files, a new directory (a build's dist/), node_modules: no
        fs::write(d.path().join("CHANGELOG.md"), "x").unwrap();
        fs::create_dir_all(d.path().join("dist/esm")).unwrap();
        fs::write(d.path().join("dist/esm/index.js"), "x").unwrap();
        fs::create_dir_all(d.path().join("node_modules/x")).unwrap();
        fs::write(d.path().join("node_modules/x/tsconfig.json"), "{}").unwrap();
        assert_eq!(a, rj(d.path()));
        // a tsconfig, at any depth: yes
        fs::create_dir_all(d.path().join("packages/p")).unwrap();
        fs::write(d.path().join("packages/p/tsconfig.json"), "{}").unwrap();
        let b = rj(d.path());
        assert_ne!(a, b);
        fs::rename(
            d.path().join("packages/p/tsconfig.json"),
            d.path().join("packages/p/jsconfig.json"),
        )
        .unwrap();
        assert_ne!(b, rj(d.path()));
    }

    #[test]
    fn nx_task_variables_are_per_run() {
        assert!(volatile_env("NX_TASK_HASH"));
        assert!(volatile_env("NX_ANALYTICS_SESSION_ID"));
        assert!(!volatile_env("NX_WORKSPACE_ROOT"));
    }

    #[test]
    fn relativize_keeps_outside_paths_and_env() {
        let root = Path::new("/w/repo");
        assert_eq!(relativize(root, "f:/w/repo/src/a.ts"), "f:src/a.ts");
        assert_eq!(relativize(root, "f:/tmp/x"), "f:/tmp/x");
        assert_eq!(relativize(root, "env:HOME"), "env:HOME");
        assert_eq!(relativize(root, "envall:"), "envall:");
    }

    #[test]
    fn yarn_run_directories_are_not_inputs() {
        assert_eq!(
            yarn_stable("/tmp/xfs-d5f5e454:/home/r/_temp/quod/bin:/usr/bin"),
            "/tmp/xfs-*:/home/r/_temp/quod/bin:/usr/bin"
        );
        assert_eq!(yarn_stable("/tmp/xfs-0a1b2c3d/node"), "/tmp/xfs-*/node");
        assert_eq!(
            yarn_stable("/tmp/yarn--1730000000000-0.4512/node:/a"),
            "/tmp/yarn--*/node:/a"
        );
        // not yarn's: kept as is
        assert_eq!(yarn_stable("/opt/xfs-tools/bin"), "/opt/xfs-tools/bin");
        assert_eq!(yarn_stable("/a/yarn--x/b"), "/a/yarn--x/b");
        assert_eq!(yarn_stable("plain"), "plain");
    }

    #[test]
    fn vitest_before_3_2_is_not_supported() {
        for v in ["3.2.0", "3.2.4", "4.0.0", "4.1.12", "5.0.0-beta.1", "weird"] {
            assert!(vitest_supported(v), "{v}");
        }
        for v in ["3.1.4", "3.0.9", "2.1.9", "1.6.0"] {
            assert!(!vitest_supported(v), "{v}");
        }
    }

    #[test]
    fn absorb_counts_a_resolver_package_json_as_covered() {
        // The tools read package.json field-level (`pj:`); the observer
        // sees the same file read: not a new whole-file shared input.
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let dir = root.join(".quod/vitest");
        fs::create_dir_all(&dir).unwrap();
        fs::write(root.join("package.json"), r#"{"name":"x","version":"1"}"#).unwrap();
        fs::write(root.join("other.json"), "{}").unwrap();
        let l = Ledger {
            version: LEDGER_VERSION,
            keys: vec!["pj:package.json".into()],
            states: vec![String::new()],
            idents: vec![None],
            global: vec![0],
            ..Ledger::default()
        };
        fs::write(dir.join("ledger.json"), serde_json::to_vec(&l).unwrap()).unwrap();
        let m = dir.join("m.json");
        fs::write(
            &m,
            r#"manifest b3:x
{"entries":[{"p":"package.json","k":"f"},{"p":"other.json","k":"f"},
{"p":"node_modules/optional-pkg","k":"a"},
{"p":"node_modules/linked","k":"l"},
{"p":"node_modules/.pnpm/vite@8/node_modules/vite/index.js","k":"f"}]}"#,
        )
        .unwrap();
        absorb(root, &dir, &dir.join("code"), &m).unwrap();
        let l: Ledger =
            serde_json::from_slice(&fs::read(dir.join("ledger.json")).unwrap()).unwrap();
        let keys: Vec<&str> = l
            .global
            .iter()
            .map(|k| l.keys[*k as usize].as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "pj:package.json",
                "e:node_modules/linked",
                "e:node_modules/optional-pkg",
                "f:other.json",
                "p:node_modules/.pnpm/vite@8/node_modules/vite"
            ],
            "absent and linked paths by existence, read packages whole, eslint (never seen) not at all"
        );
        // The optional package appears: its key changes, so its readers run.
        let i = l
            .keys
            .iter()
            .position(|k| k == "e:node_modules/optional-pkg")
            .unwrap();
        assert_eq!(l.states[i], "absent");
        fs::create_dir_all(root.join("node_modules/optional-pkg")).unwrap();
        let env = std::sync::OnceLock::new();
        let now = state(root, &l.keys[i], None, wall_ns(), &env, None).0;
        assert_ne!(now, l.states[i]);
    }

    #[test]
    fn a_watched_tree_is_keyed_by_content() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("w");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub/a"), "1").unwrap();
        let before = content_tree(&d);
        fs::write(d.join("sub/a"), "2").unwrap();
        assert_ne!(content_tree(&d), before);
        assert_eq!(content_tree(&t.path().join("none")), "absent");
    }

    #[test]
    fn output_only_arguments_are_not_keyed() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            keyed_args(&v(&[
                "run",
                "--project",
                "unit",
                "--reporter",
                "json",
                "--outputFile.json=a.json",
                "--silent",
                "-t",
                "x"
            ])),
            v(&["run", "--project", "unit", "-t", "x"])
        );
    }

    #[test]
    fn package_json_versions_do_not_count_but_the_rest_does() {
        let v1 = pj_hash(br#"{"name":"x","version":"1.0.0","packageManager":"pnpm@10.1.0"}"#);
        let v2 = pj_hash(br#"{"name":"x","version":"1.0.1","packageManager":"pnpm@10.2.0"}"#);
        assert_eq!(
            v1, v2,
            "a release bump or a packageManager bump counts for nobody"
        );
        let a = pj_hash(br#"{"name":"x","type":"module","devDependencies":{"eslint":"^9.1.0"}}"#);
        let b = pj_hash(br#"{"name":"x","type":"module","devDependencies":{"eslint":"^9.2.0"}}"#);
        let c = pj_hash(br#"{"name":"x","type":"commonjs","devDependencies":{"eslint":"^9.2.0"}}"#);
        let d = pj_hash(
            br#"{"name":"x","type":"module","devDependencies":{"eslint":"^9.2.0","vite":"1"}}"#,
        );
        assert_eq!(a, b);
        assert_ne!(b, c);
        assert_eq!(b, d, "dependencies count through the packages loaded");
    }

    #[test]
    fn states_see_content_listing_and_links() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        fs::write(root.join("a"), "1").unwrap();
        fs::create_dir(root.join("d")).unwrap();
        std::os::unix::fs::symlink("a", root.join("l")).unwrap();
        let env = std::sync::OnceLock::new();
        let st = |k: &str| state(root, k, None, wall_ns(), &env, None).0;
        let (fa, dl, el) = (st("f:a"), st("d:d"), st("e:l"));
        assert_eq!(st("f:missing"), "absent");
        assert_eq!(st("f:d"), "dir");
        assert_eq!(el, "link:a");
        fs::write(root.join("a"), "2").unwrap();
        fs::write(root.join("d/x"), "").unwrap();
        assert_ne!(st("f:a"), fa);
        assert_ne!(st("d:d"), dl);
        assert_eq!(st("e:l"), el, "existence does not see content");
        assert_ne!(st("r:."), st("d:."), "recursive listing sees d/x");
    }

    #[test]
    fn a_cached_identity_is_trusted_only_when_it_matches() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        fs::write(root.join("a"), "1").unwrap();
        let env = std::sync::OnceLock::new();
        let m = fs::metadata(root.join("a")).unwrap();
        let id = ident(&m);
        let (s, _) = state(root, "f:a", Some((id, "cached")), wall_ns(), &env, None);
        assert_eq!(s, "cached");
        let mut other = id;
        other[2] += 1;
        let (s, _) = state(root, "f:a", Some((other, "cached")), wall_ns(), &env, None);
        assert_ne!(s, "cached");
    }

    #[test]
    fn compact_remaps_and_drops_dead_keys() {
        let mut l = Ledger {
            keys: vec!["f:a".into(), "f:dead".into(), "f:b".into()],
            states: vec!["1".into(), "2".into(), "3".into()],
            idents: vec![None, None, None],
            global: vec![2],
            ..Ledger::default()
        };
        l.files.insert(
            "x".into(),
            Rec {
                project: String::new(),
                file: "t.spec.ts".into(),
                state: "pass".into(),
                duration: 1.0,
                result: "results/x.json".into(),
                deps: vec![0, 2],
                quarantined: false,
                good: 0,
                runs: 1,
                replays: 0,
                cov: BTreeMap::from([(2, vec![[1, 0, 3, 0]])]),
                pinned: BTreeMap::from([(0, "old".to_string())]),
            },
        );
        l.compact();
        assert_eq!(l.keys, vec!["f:a".to_string(), "f:b".to_string()]);
        assert_eq!(l.global, vec![1]);
        assert_eq!(l.files["x"].deps, vec![0, 1]);
        assert_eq!(
            l.files["x"].cov.keys().copied().collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(
            l.files["x"].pinned.keys().copied().collect::<Vec<_>>(),
            vec![0]
        );
        assert_eq!(l.states, vec!["1".to_string(), "3".to_string()]);
    }

    #[test]
    fn replace_bytes_rewrites_every_occurrence() {
        assert_eq!(
            replace_bytes(b"a /r/out-0 b /r/out-0", b"/r/out-0", b"/x.json"),
            b"a /x.json b /x.json".to_vec()
        );
        assert_eq!(replace_bytes(b"none", b"/r", b"/x"), b"none".to_vec());
    }

    #[test]
    fn installed_keys_are_under_the_workspace_node_modules() {
        assert!(installed(
            "p:node_modules/.pnpm/vitest@4.1.10/node_modules/vitest"
        ));
        assert!(installed("e:packages/a/node_modules/b"));
        assert!(installed("f:node_modules"));
        assert!(installed("d:packages/a/node_modules"));
        assert!(!installed(
            "f:/usr/local/node/lib/node_modules/npm/index.js"
        ));
        assert!(!installed("f:src/index.ts"));
        assert!(!installed("envall"));
    }

    #[test]
    fn an_npm_package_counts_its_lockfile_entry() {
        // npm hoists: node_modules/<name> names no version, and a git or
        // tarball dependency can change without its package.json changing.
        // The lockfile's entry (by the path from the workspace root, nested
        // ones included) joins the state; same package.json, other source:
        // another state. (A fresh root each time: lockfiles are parsed once
        // per process.)
        let lock = |resolved: &str| {
            format!(
                r#"{{"packages":{{"node_modules/g":{{"version":"1.0.0","resolved":"{resolved}"}},
"packages/a/node_modules/g":{{"version":"2.0.0"}}}}}}"#
            )
        };
        let st = |resolved: &str, key: &str| {
            let t = tempfile::tempdir().unwrap();
            let r = t.path();
            fs::create_dir(r.join(".git")).unwrap();
            fs::write(r.join("package-lock.json"), lock(resolved)).unwrap();
            let pj = r#"{"name":"g","version":"1.0.0"}"#;
            for d in ["node_modules/g", "packages/a/node_modules/g"] {
                fs::create_dir_all(r.join(d)).unwrap();
                fs::write(r.join(d).join("package.json"), pj).unwrap();
            }
            fs::create_dir_all(r.join("packages/a/node_modules/h")).unwrap();
            fs::write(r.join("packages/a/node_modules/h/package.json"), pj).unwrap();
            let pkg_root = r.join("packages/a");
            let key = if key.starts_with("p:/") {
                key.replace("p:/", &format!("p:{}/", r.display()))
            } else {
                key.to_string()
            };
            state(&pkg_root, &key, None, 0, &std::sync::OnceLock::new(), None).0
        };
        let a = st("git+ssh://x/g.git#aaaa", "p:/node_modules/g");
        let b = st("git+ssh://x/g.git#bbbb", "p:/node_modules/g");
        assert!(a.contains('+'), "{a}");
        assert_ne!(a, b);
        assert_eq!(a.split('+').next(), b.split('+').next());
        // nested, relative to the package that runs vitest
        let n1 = st("x", "p:node_modules/g");
        let n2 = st("y", "p:node_modules/g");
        assert_eq!(n1, n2);
        assert!(n1.contains('+'));
        assert!(st("x", "p:node_modules/h").ends_with("+unlisted"));
        // no npm lockfile (pnpm): the package.json alone, as before
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join(".git")).unwrap();
        fs::create_dir_all(t.path().join("node_modules/g")).unwrap();
        fs::write(t.path().join("node_modules/g/package.json"), "{}").unwrap();
        let none = std::sync::OnceLock::new();
        let s = state(t.path(), "p:node_modules/g", None, 0, &none, None).0;
        assert_eq!(s, blake3::hash(b"{}").to_hex().to_string());
    }

    #[test]
    fn walk_sums_paths_and_finds_install_files() {
        let d = std::env::temp_dir().join(format!("quod-walk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("src")).unwrap();
        fs::create_dir_all(d.join("node_modules/x")).unwrap();
        fs::write(d.join("package.json"), "{}").unwrap();
        fs::write(d.join("src/a.ts"), "1").unwrap();
        let ledger_dir = d.join(".quod");
        let a = walk(&d, &ledger_dir);
        assert_ne!(a.hash, "");
        assert_eq!(a.install, vec![PathBuf::from("package.json")]);
        // contents and node_modules do not count; a new path does
        fs::write(d.join("src/a.ts"), "2").unwrap();
        fs::write(d.join("node_modules/x/y"), "").unwrap();
        fs::create_dir_all(ledger_dir.join("render")).unwrap();
        assert_eq!(walk(&d, &ledger_dir).hash, a.hash);
        fs::write(d.join("src/b.spec.ts"), "").unwrap();
        assert_ne!(walk(&d, &ledger_dir).hash, a.hash);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn par_map_keeps_order() {
        let v: Vec<u32> = (0..1000).collect();
        assert_eq!(
            par_map(&v, |x| x * 2),
            v.iter().map(|x| x * 2).collect::<Vec<_>>()
        );
    }
}
