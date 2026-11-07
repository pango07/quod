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
//! Second witness (the default; `QUOD_VITEST_WITNESSES=1` trusts a record at
//! once, as before, and a higher number asks for more): a record is
//! provisional until a second, independent run of the file produces the same
//! key (the same states of all its inputs, the same vitest arguments and
//! facts, shared inputs unchanged in between) with the same per-test
//! results; only then does it replay. A disagreement quarantines the file:
//! it runs until it re-proves (one more agreeing witness per recent
//! disagreement, as the trust ledger's proofs). With it the job that runs
//! the tests records inline and no shadow job is needed.
//!
//! The second witness runs plain (`QUOD_VITEST_PLAIN_WITNESS=0`: recorded,
//! as before): when a provisional record's key still holds at selection
//! (every input in the state it recorded, the shared inputs unchanged), the
//! file runs as without Quod (no setup file, no hooks, no coverage), and its
//! per-test results promote the record when they agree. Its durations are
//! plain ones, and only they decide the near-timeout margin
//! (`QUOD_VITEST_NEAR_TIMEOUT`, a fraction of each test's timeout, 0.5 by
//! default): a duration measured under the recorder never does.
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
const CHILD: &str = include_str!("vitest/child.mjs");
const DENY: &str = include_str!("vitest/deny.mjs");
const NAMES_PROOF: &str = include_str!("vitest/names-proof.mjs");
const BOOLEAN_PROOF: &str = include_str!("vitest/boolean-proof.mjs");
const ORIGIN_PRELOAD: &str = include_str!("vitest/origin-preload.cjs");
const STAT_ORIGIN: &str = include_str!("vitest/stat-origin.cjs");
const ACORN: &str = include_str!("vitest/acorn.mjs");
const ACORN_LICENSE: &str = include_str!("vitest/acorn-LICENSE");

/// The driver's files, as embedded; for development, a file of the same
/// name in `QUOD_VITEST_JS_DIR` stands in for one (the bench iterates on
/// them without a build). Either way `code_hash` is of what runs, so a
/// ledger never pairs with other code.
fn js() -> &'static [(&'static str, String)] {
    static JS: std::sync::OnceLock<Vec<(&'static str, String)>> = std::sync::OnceLock::new();
    JS.get_or_init(|| {
        let over = std::env::var_os("QUOD_VITEST_JS_DIR").map(PathBuf::from);
        [
            ("driver.mjs", DRIVER),
            ("setup.mjs", SETUP),
            ("record.mjs", RECORD),
            ("coverage.mjs", COVERAGE),
            ("preload.cjs", PRELOAD),
            ("narrow.mjs", NARROW),
            ("child.mjs", CHILD),
            ("deny.mjs", DENY),
            ("names-proof.mjs", NAMES_PROOF),
            ("boolean-proof.mjs", BOOLEAN_PROOF),
            ("origin-preload.cjs", ORIGIN_PRELOAD),
            ("stat-origin.cjs", STAT_ORIGIN),
            ("acorn.mjs", ACORN),
            ("acorn-LICENSE", ACORN_LICENSE),
        ]
        .into_iter()
        .map(|(name, body)| {
            let o = over
                .as_ref()
                .and_then(|d| fs::read_to_string(d.join(name)).ok());
            (name, o.unwrap_or_else(|| body.to_string()))
        })
        .collect()
    })
}

/// Bumped when what a key means or what is recorded changes: a ledger of
/// another version is dropped (every file runs once). v2: function-level
/// coverage and pinned states.
const LEDGER_VERSION: u32 = 3;

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

/// Per-run in general (quod_key's VOLATILE and DENYLIST), but a fixed path
/// on a hosted runner: a read is keyed by its value, as any variable's (a
/// runner where it differs is a miss, never a wrong replay). npm reads
/// GITHUB_EVENT_PATH in every npx a test starts (convex-helpers'
/// openApiSpec.test.ts, `npx redocly lint`); a file's content, if read, is
/// keyed as a file.
const VALUE_KEYED: &[&str] = &[
    "GITHUB_EVENT_PATH",
    "RUNNER_TEMP",
    "GITHUB_ACTION_PATH",
    "QUOD_VITEST_NAMES_PROOF",
    "QUOD_VITEST_BOOLEAN_PROOF",
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

#[derive(Serialize, Deserialize, Default, Clone)]
struct Ledger {
    version: u32,
    /// Recorder and key semantics that observed these dependencies. Older
    /// records cannot supply witnesses for code that may observe more inputs.
    #[serde(default)]
    code: String,
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
    /// Recording admission is only an economic hint. It never changes any
    /// input, witness, quarantine or replay decision.
    #[serde(default)]
    recording_admission: RecordingAdmission,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct RecordingAdmission {
    shared: String,
    misses: u32,
    skipped: u32,
}

impl RecordingAdmission {
    fn skip(&self, shared: &str, eligible: bool) -> bool {
        eligible
            && !shared.is_empty()
            && !self.shared.is_empty()
            && shared != self.shared
            && self.misses >= 2
            && self.skipped < 7
    }

    fn observe(&mut self, shared: String, unstable: bool, skipped: bool) {
        self.shared = shared;
        self.misses = if unstable {
            self.misses.saturating_add(1)
        } else {
            0
        };
        self.skipped = if skipped {
            self.skipped.saturating_add(1)
        } else {
            0
        };
    }
}

/// A performance hint, deliberately separate from the ledger's proof states.
/// Include key names so changing the dependency set cannot look stable.
fn shared_fingerprint(ledger: &Ledger, mut state: impl FnMut(u32) -> Option<String>) -> String {
    if ledger.global.is_empty() {
        return String::new();
    }
    let mut inputs = Vec::new();
    for id in &ledger.global {
        let (Some(key), Some(value)) = (ledger.keys.get(*id as usize), state(*id)) else {
            return String::new();
        };
        inputs.push((key, value));
    }
    inputs.sort();
    let mut h = blake3::Hasher::new();
    for (key, value) in inputs {
        h.update(&(key.len() as u64).to_le_bytes());
        h.update(key.as_bytes());
        h.update(&(value.len() as u64).to_le_bytes());
        h.update(value.as_bytes());
    }
    h.finalize().to_hex().to_string()
}

fn save_ledger(path: &Path, ledger: &Ledger) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec(ledger)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn compatible_ledger(bytes: &[u8], code: &str) -> Option<Ledger> {
    serde_json::from_slice(bytes)
        .ok()
        .filter(|l: &Ledger| l.version == LEDGER_VERSION && !code.is_empty() && l.code == code)
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

#[derive(Serialize, Deserialize, Clone, Default)]
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
    /// The second-witness policy's state of this record.
    #[serde(default)]
    w: Witness,
    /// A test of this run took more than half its timeout: a pass that a
    /// loaded runner can turn into a timeout. Recorded, never replayed,
    /// until a run with every test within half its timeout.
    #[serde(default)]
    near_timeout: bool,
    /// Plain witnesses (no recorder) that agreed with this record's key.
    #[serde(default)]
    plain_n: u32,
    /// Pending plain audit of an E1 promotion. Kept separate from the
    /// original stamp so one bad audit revokes even already-audited peers.
    #[serde(default)]
    lineage: Option<String>,
    #[serde(default)]
    promotion_stamp: Option<String>,
    /// The largest fraction of its timeout any test took in a plain witness.
    #[serde(default)]
    margin: f64,
    /// The recording used the network: only a plain witness with no network
    /// (deny.mjs) can vouch for it.
    #[serde(default)]
    net: bool,
}

/// Second witness: who has vouched for a record.
#[derive(Serialize, Deserialize, Clone, Default)]
struct Witness {
    /// Independent runs that produced this key with these results, in a row.
    #[serde(default)]
    n: u32,
    /// The key: every input's state (and the arguments and facts) when recorded.
    #[serde(default)]
    key: String,
    /// The per-test results (names and states).
    #[serde(default)]
    tests: String,
    /// Disagreements seen (each one raises the bar to re-prove, up to 5).
    #[serde(default)]
    disagreements: u32,
}

/// Witnesses a record needs before it replays: 2 (the default, the
/// second-witness policy); `QUOD_VITEST_WITNESSES=1` trusts a record at once.
fn witnesses_needed() -> u32 {
    std::env::var("QUOD_VITEST_WITNESSES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2)
        .max(1)
}

/// The witnesses a record needs (more after disagreements).
fn bar(need: u32, w: &Witness) -> u32 {
    need + w.disagreements.min(5)
}

/// May this record replay under the witness policy? Never while its
/// last run had a test near its timeout.
fn witnessed(need: u32, r: &Rec) -> bool {
    !r.near_timeout
        && (r.lineage.is_none() || lineage_enabled())
        && (need <= 1 || r.w.n >= bar(need, &r.w))
}

fn lineage_enabled() -> bool {
    std::env::var("QUOD_VITEST_LINEAGE").is_ok_and(|v| v == "1")
}

/// A changed input is still executed and recorded. Only the extra plain
/// confirmation can be inherited, from a predecessor confirmed plain.
fn promote_lineage(need: u32, p: &Rec, q: &mut Rec, stamp: &str) -> bool {
    if need != 2
        || p.w.n < bar(need, &p.w)
        || p.plain_n == 0
        || p.quarantined
        || q.quarantined
        || q.near_timeout
        || q.net
        || !matches!(q.state.as_str(), "pass" | "skip")
        || p.deps != q.deps
        || p.w.tests.is_empty()
        || p.w.tests != q.w.tests
        || p.w.key.is_empty()
        || q.w.key.is_empty()
        || p.w.disagreements != 0
        || !q.pinned.is_empty()
    {
        return false;
    }
    q.w.n = 2;
    q.plain_n = 0;
    q.lineage = Some(stamp.to_string());
    q.promotion_stamp = q.lineage.clone();
    true
}

/// Stable sample, rounded up. Each agreeing audit leaves the pending set,
/// so subsequent jobs progress through the rest rather than repeat it.
fn lineage_sample<'a>(records: impl Iterator<Item = (&'a String, &'a Rec)>) -> Vec<String> {
    let mut pending: Vec<_> = records
        .filter_map(|(id, r)| {
            r.lineage.as_ref().map(|stamp| {
                (
                    blake3::hash(format!("{id}\0{stamp}").as_bytes())
                        .to_hex()
                        .to_string(),
                    id.clone(),
                )
            })
        })
        .collect();
    pending.sort();
    let n = pending.len().div_ceil(8);
    pending.into_iter().take(n).map(|(_, id)| id).collect()
}

fn revoke_lineage(files: &mut BTreeMap<String, Rec>, stamps: &HashSet<String>) -> usize {
    let mut revoked = 0;
    for r in files.values_mut() {
        if r.promotion_stamp
            .as_ref()
            .is_some_and(|s| stamps.contains(s))
        {
            r.w.n = 1;
            r.plain_n = 0;
            r.lineage = None;
            r.promotion_stamp = None;
            revoked += 1;
        }
    }
    revoked
}

/// Timing-sensitive replay (`QUOD_VITEST_TIMING=replay`, off by default):
/// a record whose plain runs came near a test's timeout replays anyway once
/// its two witnesses of its unchanged key agree on a pass, at least one of
/// them plain (the one that measured the margin), labelled with that
/// margin (through-the-wall A1 (b)). The timeout is then the machine's, not
/// the code's: the inputs are the ones the two witnesses passed on. A plain
/// run after that buys nothing but the runner's load of the day.
fn timing_replay() -> bool {
    std::env::var("QUOD_VITEST_TIMING").is_ok_and(|v| v == "replay")
}

/// With timing-sensitive replay: every Nth replay of such a file runs it
/// plain instead (`QUOD_VITEST_TIMING_AUDIT=N`, 0 or unset: never); a
/// failure there (a timeout) quarantines it.
fn timing_audit() -> u32 {
    std::env::var("QUOD_VITEST_TIMING_AUDIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// A near-timeout record that timing-sensitive replay lets replay.
fn timing_ok(need: u32, r: &Rec) -> bool {
    timing_ok_if(timing_replay(), need, r)
}

fn timing_ok_if(on: bool, need: u32, r: &Rec) -> bool {
    on && r.near_timeout
        && !r.quarantined
        && r.plain_n >= 1
        && (need <= 1 || r.w.n >= bar(need, &r.w))
}

/// May this record replay, under the witness and timing policies?
fn replayable(need: u32, r: &Rec) -> bool {
    witnessed(need, r) || timing_ok(need, r)
}

/// The second witness runs plain (the default); `QUOD_VITEST_PLAIN_WITNESS=0`
/// records it, as before.
fn plain_witness() -> bool {
    std::env::var("QUOD_VITEST_PLAIN_WITNESS").map_or(true, |v| v != "0")
}

/// The near-timeout margin: a test whose plain duration is past this
/// fraction of its timeout keeps its record from replaying (0.5 by default).
fn near_timeout_factor() -> f64 {
    std::env::var("QUOD_VITEST_NEAR_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|f| *f > 0.0 && *f <= 1.0)
        .unwrap_or(0.5)
}

#[derive(Deserialize, Default)]
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
    /// The main process recorded (false: every file that ran was a plain
    /// second witness, so `global` is empty and the ledger's stands).
    #[serde(default, rename = "mainRecorded")]
    main_recorded: Option<bool>,
}

#[derive(Deserialize, Default)]
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
    /// Per-test names and states (the second witness compares these).
    #[serde(default)]
    tests: Option<String>,
    /// Tests that took more than half their timeout ("name: 1200 of 2000 ms").
    #[serde(default, rename = "nearTimeout")]
    near_timeout: Vec<String>,
    /// Ran as a plain second witness (nothing recorded: no keys).
    #[serde(default)]
    plain: bool,
    /// The largest fraction of its timeout a test took.
    #[serde(default)]
    ratio: f64,
    /// Its recording used the network.
    #[serde(default)]
    net: bool,
    /// Ran with the network denied (a plain witness of a networked record).
    #[serde(default)]
    denied: bool,
}

fn test_signature(r: &Ran) -> String {
    r.tests
        .as_ref()
        .map(|tests| {
            blake3::hash(format!("{}\n{tests}", r.state).as_bytes())
                .to_hex()
                .to_string()
        })
        .unwrap_or_default()
}

/// Determine faulty cohorts before accepting any of their current replays.
/// Plain audits compare the same signature as witness ingestion below.
fn lineage_faults(
    ledger: &Ledger,
    out: &Out,
    plain_ids: &HashSet<String>,
) -> (HashSet<String>, HashSet<String>) {
    let mut stamps = HashSet::new();
    let mut files = HashSet::new();
    for r in &out.ran {
        let Some(p) = ledger.files.get(&r.id) else {
            continue;
        };
        let Some(stamp) = &p.promotion_stamp else {
            continue;
        };
        let plain_bad = out.facts == ledger.facts
            && r.plain
            && plain_ids.contains(&r.id)
            && (!p.net || r.denied)
            && (!matches!(r.state.as_str(), "pass" | "skip") || test_signature(r) != p.w.tests);
        if plain_bad || out.audit_mismatch.contains(&r.id) {
            stamps.insert(stamp.clone());
            files.insert(r.id.clone());
        }
    }
    (stamps, files)
}

pub fn cli(mut a: VitestArgs) -> Result<u8> {
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
    for (name, body) in js() {
        let p = code.join(name);
        if fs::read_to_string(&p).ok().as_deref() != Some(body.as_str()) {
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
    let recorder_code = code_hash();
    let mut ledger = fs::read(&ledger_path)
        .ok()
        .and_then(|b| compatible_ledger(&b, &recorder_code))
        .unwrap_or_else(|| Ledger {
            version: LEDGER_VERSION,
            code: recorder_code,
            ..Ledger::default()
        });
    let t_load = t0.elapsed();
    if let Some(pf) = &a.plan {
        return plan(&a, pf, &root, &dir, &ledger, full);
    }
    let fast_ok = !a.no_fast && std::env::var("QUOD_VITEST_NO_FAST").map_or(true, |v| v != "1");
    // The paths as this run starts (a record keeps them: the next critical
    // path compares its own before emitting a rendered full replay).
    let tw = Instant::now();
    let tree_before = if a.no_record {
        None
    } else {
        Some(walk(&root, &dir))
    };
    let walk_ms = tw.elapsed().as_secs_f64() * 1e3;

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
    let adaptive_recording = std::env::var("QUOD_VITEST_RECORDING").is_ok_and(|v| v == "adaptive");
    // Back off only when the existing selector already requires EVERY file
    // to execute. Full/explicit recording and pending E1 audits take priority.
    // A skipped recording supplies no witness and updates no proof state.
    let admission_eligible = !a.no_record
        && !full
        && a.expect_report.is_none()
        && matches!(a.audits.as_str(), "auto" | "0")
        && !global_ok
        && !ledger.files.is_empty()
        && ledger.args == keyed_args(&a.args)
        && ledger.files.values().all(|r| r.lineage.is_none());
    let shared_now = if adaptive_recording {
        shared_fingerprint(&ledger, |id| now.get(&id).map(|s| s.0.clone()))
    } else {
        String::new()
    };
    let unstable_shared = admission_eligible
        && !shared_now.is_empty()
        && !ledger.recording_admission.shared.is_empty()
        && shared_now != ledger.recording_admission.shared;
    let adaptive_plain = adaptive_recording
        && ledger
            .recording_admission
            .skip(&shared_now, admission_eligible);
    if adaptive_plain {
        a.no_record = true;
        eprintln!(
            "quod vitest: shared inputs keep changing; every test runs plain, recording retries after at most seven skipped recordings"
        );
    }
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
    let need = witnesses_needed();
    let mut provisional = 0usize;
    // The files this run can witness again (a record of these very inputs
    // exists): the only ones whose V8 coverage (function-level narrowing)
    // a trusted record can come to hold. A first witness never replays, so
    // its coverage is never used: not taken (setup.mjs), the costliest part
    // of recording (doc 4.2).
    let unchanged = |rec: &Rec| {
        let same = |d: &u32| now[d].0 == ledger.states[*d as usize];
        global_ok && rec.deps.iter().all(same)
    };
    let mut witness_files: Vec<PathBuf> = Vec::new();
    // Plain second witnesses: a provisional record (or one whose last plain
    // run was near a timeout) whose key holds now runs as without Quod.
    let plain_mode = plain_witness() && need > 1 && !a.no_record && a.expect_report.is_none();
    let can_plain = |rec: &Rec| {
        plain_mode
            && !rec.quarantined
            && (rec.state == "pass" || rec.state == "skip")
            && !rec.w.key.is_empty()
            && !rec.w.tests.is_empty()
            && rec.w.n >= 1
            && unchanged(rec)
    };
    let mut plain_files: Vec<PathBuf> = Vec::new();
    let mut plain_ids: HashSet<String> = HashSet::new();
    let mut deny_files: Vec<PathBuf> = Vec::new();
    let audit_n = timing_audit();
    let mut timing_note: Vec<String> = Vec::new();
    for (id, rec) in &ledger.files {
        if !(global_ok && !rec.quarantined && (rec.state == "pass" || rec.state == "skip")) {
            if rec.quarantined && unchanged(rec) {
                witness_files.push(root.join(&rec.file));
            }
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
        let timed = timing_ok(need, rec);
        if timed && audit_n > 0 && (rec.replays + 1) % audit_n == 0 && can_plain(rec) {
            // The timing audit: this replay runs plain instead.
            why.insert(id.clone(), "timing-sensitive: its audit run".into());
            plain_files.push(root.join(&rec.file));
            plain_ids.insert(id.clone());
            affected.push(id.clone());
            continue;
        }
        if rec.near_timeout && !timed {
            why.insert(
                id.clone(),
                "a test took more than half its timeout last run".into(),
            );
            if can_plain(rec) {
                plain_files.push(root.join(&rec.file));
                plain_ids.insert(id.clone());
            }
            affected.push(id.clone());
            continue;
        }
        if !replayable(need, rec) {
            // One witness so far: it runs again, and agreeing makes it two.
            provisional += 1;
            if can_plain(rec) {
                plain_files.push(root.join(&rec.file));
                plain_ids.insert(id.clone());
            } else if unchanged(rec) {
                witness_files.push(root.join(&rec.file));
            }
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
            if timed {
                timing_note.push(format!(
                    "{} (passed at {:.2} of its timeout when recorded)",
                    rec.file, rec.margin
                ));
            }
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
    let lineage_audits: HashSet<String> = if plain_mode {
        lineage_sample(replay.keys().filter_map(|id| {
            let r = &ledger.files[id];
            can_plain(r).then_some((id, r))
        }))
        .into_iter()
        .collect()
    } else {
        HashSet::new()
    };
    for id in &lineage_audits {
        replay.remove(id);
        plain_ids.insert(id.clone());
        plain_files.push(root.join(&ledger.files[id].file));
        affected.push(id.clone());
        why.insert(id.clone(), "lineage witness: sampled plain audit".into());
    }
    for w in why_run.iter().take(12) {
        eprintln!("quod vitest:   runs: {w}");
    }
    for t in &timing_note {
        eprintln!("quod vitest: timing-sensitive: {t} replays");
    }
    for id in &plain_ids {
        let rec = &ledger.files[id];
        if rec.net {
            deny_files.push(root.join(&rec.file));
        }
    }
    if !deny_files.is_empty() {
        eprintln!(
            "quod vitest: {} files that used the network run their second witness with none",
            deny_files.len()
        );
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
        // E1 records await their dedicated plain audit. Recording them
        // as opportunistic audits must not consume that pending obligation.
        .filter(|id| ledger.files[*id].lineage.is_none() && ledger.files[*id].duration <= slowest)
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
    let mut report_base = Reporting {
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
        // Where workers take V8 coverage: every file (null) when one witness
        // trusts a record, else the witness runs and the audits.
        "cov": if need > 1 {
            serde_json::json!(witness_files
                .iter()
                .cloned()
                .chain(audit.iter().map(|id| root.join(&ledger.files[id].file)))
                .collect::<Vec<_>>())
        } else {
            serde_json::Value::Null
        },
        // Every CPU only in a shadow (`--expect-report`, diagnostics only):
        // the critical path keeps vitest's own worker count, the one the
        // project's timeouts were set under (convex-helpers: a 25 s
        // CPU-bound test took 29-30 s with one more worker and timed out
        // at 30 s, the job red where plain vitest was green). With inline
        // recording (no shadow) the critical path records, so recording
        // alone does not lift the count.
        // Plain second witnesses, and whether anything records at all (no:
        // every file that runs is one, so vitest runs entirely plain).
        "plain": plain_files,
        "deny": deny_files,
        "netWitness": plain_mode,
        "hooks": a.no_record || !(audit.is_empty() && !affected.is_empty() && affected.iter().all(|id| plain_ids.contains(id))),
        "nearTimeout": near_timeout_factor(),
        "workers": if !a.no_record && a.expect_report.is_some() && affected.len() > 4 * workers && alone() { serde_json::json!(workers) } else { serde_json::Value::Null },
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
    if need > 1 {
        let all_plain = plan["hooks"] == serde_json::json!(false);
        let plain_note = match (plain_ids.len(), all_plain) {
            (0, _) => String::new(),
            (n, false) => format!("; {n} of them plain (their keys hold)"),
            (n, true) => format!("; {n} of them plain: nothing records, vitest runs plain"),
        };
        eprintln!(
            "quod vitest: second witness: {provisional} provisional records run again ({need} witnesses needed){plain_note}"
        );
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

    // Every file replays (none to run, none to audit): emit the recorded
    // full replay instead of starting vitest. Inline too: a run that runs
    // nothing records nothing.
    if (a.no_record || audit.is_empty())
        && fast_ok
        && !ledger.files.is_empty()
        && affected.is_empty()
        && replay.len() == ledger.files.len()
    {
        let tree = match &tree_before {
            Some(w) => w.hash.clone(),
            None => walk(&root, &dir).hash,
        };
        let emitted = node_version(&a.node)
            .ok_or_else(|| "node --version failed".to_string())
            .and_then(|node| emit_render(&root, &dir, &ledger, &a.args, &node, &tree));
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

    // The render (for a later run that replays every file) loads vitest
    // while this run's tests run; it renders only if this run leaves every
    // file recorded, else it is stopped.
    let no_render =
        std::env::var("QUOD_VITEST_DISABLE").is_ok_and(|v| v.split(',').any(|p| p == "render"));
    let standby = if !a.no_record && fast_ok && !no_render {
        Standby::start(&a, &dir, &code_dir)
            .map_err(|e| eprintln!("quod vitest: no render ({e})"))
            .ok()
    } else {
        None
    };
    // A run that is all plain witnesses, some of which must run with no
    // network: in a network namespace of its own (loopback only), so the
    // denial holds for native code too. Else the denial is in-process
    // (deny.mjs) for those files and their node children.
    let netns = plan["hooks"] == serde_json::json!(false)
        && plan["deny"].as_array().is_some_and(|d| !d.is_empty())
        && bwrap_net_ok();
    let mut cmd = if netns {
        eprintln!("quod vitest: the plain witnesses run in a network namespace (no network)");
        let mut c = Command::new("bwrap");
        c.args([
            "--dev-bind",
            "/",
            "/",
            "--unshare-net",
            "--die-with-parent",
            "--",
        ])
        .arg(&a.node);
        c
    } else {
        Command::new(&a.node)
    };
    if plan["record"] != serde_json::json!(false)
        && plan["hooks"] != serde_json::json!(false)
        && ["QUOD_VITEST_NAMES_PROOF", "QUOD_VITEST_BOOLEAN_PROOF"]
            .iter()
            .any(|name| std::env::var(name).is_ok_and(|value| value == "1"))
    {
        // Only the Node child sees the prefix. The planner keys the original
        // environment; the preload restores it before any user code runs.
        if let Some(options) = origin_node_options(
            &code.join("origin-preload.cjs"),
            std::env::var_os("NODE_OPTIONS").as_deref(),
        ) {
            cmd.env("NODE_OPTIONS", options);
        }
    }
    let status = without_credentials(&mut cmd)
        .env("QUOD_VITEST_HIDE", hidden_env(&a))
        .arg(code.join("driver.mjs"))
        .arg(&plan_path)
        .arg(&out_path)
        .args(&a.args)
        .status()
        .with_context(|| format!("running {}", a.node))?;
    let code = status.code().map_or(1, |c| c.clamp(0, 255) as u8);
    let mut out: Out = match fs::read(&out_path)
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
    let (faulty_stamps, faulty_files) = lineage_faults(&ledger, &out, &plain_ids);
    let invalidated_replays: HashSet<String> = out
        .replayed
        .iter()
        .filter(|id| {
            ledger
                .files
                .get(*id)
                .and_then(|r| r.promotion_stamp.as_ref())
                .is_some_and(|stamp| faulty_stamps.contains(stamp))
        })
        .cloned()
        .collect();
    let mut recovery = None;
    let mut recovery_note = serde_json::Value::Null;
    let code = if !invalidated_replays.is_empty() && !a.no_record {
        // Persist revocation before starting another child. A failed or
        // interrupted recovery must not leave known-bad trust on disk.
        let mut rejected = ledger.clone();
        for id in &faulty_files {
            if let Some(r) = rejected.files.get_mut(id) {
                r.quarantined = true;
                r.w.disagreements += 1;
            }
        }
        revoke_lineage(&mut rejected.files, &faulty_stamps);
        rejected.render = None;
        save_ledger(&ledger_path, &rejected)?;
        let started = Instant::now();
        match recover_lineage(
            &a,
            &code_dir,
            &plan,
            &replay,
            &out,
            &dir,
            &invalidated_replays,
        ) {
            Ok((exit, recovered)) => {
                recovery_note = serde_json::json!({
                    "invalidated_replays": invalidated_replays.len(),
                    "executed": recovered.ran.len(),
                    "ms": started.elapsed().as_secs_f64() * 1e3,
                    "driver": recovered.timings,
                });
                recovery = Some(recovered);
                exit
            }
            Err(error) => {
                if let Some(report) = &a.report {
                    let _ = fs::remove_file(report);
                }
                if let Some(summary) = &a.summary {
                    fs::write(
                        summary,
                        serde_json::to_vec(&serde_json::json!({
                            "exit": 1, "lineage_recovery_failed": true,
                            "invalidated_replays": invalidated_replays.len(),
                        }))?,
                    )?;
                }
                if let Some(c) = &cloud
                    && !a.cloud_no_save
                {
                    c.save(&dir);
                }
                eprintln!("quod vitest: lineage recovery failed: {error:#}; faulty cohort revoked, combined report discarded");
                return Ok(1);
            }
        }
    } else {
        code
    };
    // Inline recording never turns a green job red: the instrumentation (V8
    // coverage, the hooks) can slow a timing-sensitive test into a failure
    // (reselect's cache-hit performance check). The files that failed while
    // recording run once more, plain, as without Quod, and every other file
    // reports this run's own result; that run's outcome, outputs and report
    // are the job's. Failed files are not recorded either way.
    let code = if recovery.is_none() && !a.no_record && a.expect_report.is_none() && code != 0 {
        confirm_failures(&a, &code_dir, &plan, &replay, &out, &dir).unwrap_or(code)
    } else {
        code
    };
    let t_run = t0.elapsed();
    if a.no_record {
        let ran_facts: Vec<_> = out
            .ran
            .iter()
            .map(|r| (r.id.clone(), r.file.clone(), r.state.clone(), r.duration))
            .collect();
        let replayed_ids: Vec<_> = out.replayed.iter().collect();
        if adaptive_plain {
            anyhow::ensure!(
                out.replayed.is_empty(),
                "adaptive plain execution unexpectedly replayed a file"
            );
            // No input, result, witness or audit field is refreshed by this
            // plain run. Only the admission hint travels to the next job.
            ledger
                .recording_admission
                .observe(shared_now, unstable_shared, true);
            save_ledger(&ledger_path, &ledger)?;
            if let Some(c) = &cloud
                && !a.cloud_no_save
            {
                c.save(&dir);
            }
        }
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
            "quod vitest: ran {} files plain, replayed {} ({} function-level); quod {} ms (select); not recorded ({})",
            out.ran.len(),
            out.replayed.len(),
            narrowed.len(),
            t_select.as_millis(),
            if adaptive_plain {
                "adaptive recording backoff"
            } else {
                "the shadow run records"
            },
        );
        if let Some(p) = &a.summary {
            let s = serde_json::json!({
                "recorded_files": ledger.files.len(),
                "recording_policy": if adaptive_plain { "adaptive-plain" } else { "no-record" },
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
                "provisional": provisional,
                "exit": code,
            });
            fs::write(p, serde_json::to_vec(&s)?)?;
        }
        return Ok(code);
    }

    // Ingest: the files that ran replace their records.
    let mut phase = Instant::now();
    let mut phases: BTreeMap<&str, f64> = BTreeMap::new();
    let mut lap = |name: &'static str, phases: &mut BTreeMap<&str, f64>| {
        phases.insert(name, phase.elapsed().as_secs_f64() * 1e3);
        phase = Instant::now();
    };
    phases.insert("walk", walk_ms);
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
    // A plain witness writes no record of its own module graph, so the
    // main process's reads on its behalf (the sources vite transformed for
    // it, the packages it resolved) look like the main process's own. They
    // are that file's inputs, keyed in its record already: not shared.
    let plain_keys: HashSet<String> = plain_ids
        .iter()
        .filter_map(|id| ledger.files.get(id))
        .flat_map(|r| r.deps.iter().map(|d| ledger.keys[*d as usize].clone()))
        .filter(|k| k.starts_with("f:") || k.starts_with("p:"))
        .collect();
    let mut global: Vec<u32> = out
        .global
        .iter()
        .filter(|k| !plain_keys.contains(&relativize(&root, k)))
        .map(|k| key(&mut ledger, k))
        .collect();
    if !out.replayed.is_empty() || out.main_recorded == Some(false) {
        global.extend(ledger.global.iter().copied());
    }
    global.sort_unstable();
    global.dedup();
    // E1 compares the file's own read set. Shared inputs already forced
    // this fresh recorded execution; their tool bookkeeping can differ
    // (temporary transform paths) without changing the file's reads.
    let lineage_context = ledger.args == keyed_args(&a.args);
    ledger.global = global;
    // Under the witness policy the previous record of a file that runs is
    // what this run's result is witnessed against.
    let mut prev: HashMap<String, Rec> = HashMap::new();
    for id in &affected {
        if let Some(r) = ledger.files.remove(id)
            && need > 1
        {
            prev.insert(id.clone(), r);
        }
    }
    // (file, its previous witness state, an audit mismatched, the previous
    // run had a test near its timeout)
    let mut witness_todo: Vec<(String, Option<Rec>, bool, bool)> = Vec::new();
    let stamp = format!("{}-{}", wall_ns(), std::process::id());
    let mut lineage_promoted = 0;
    let mut lineage_disagreed = 0;
    let mut revoked_stamps = HashSet::new();
    let mut near: Vec<String> = Vec::new();
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
    // Plain second witnesses: (promoted, agreed, disagreed, near a timeout).
    let mut pw = (0usize, 0usize, 0usize, 0usize);
    let mut plain_near: Vec<String> = Vec::new();
    let facts_held = out.facts == ledger.facts;
    for r in &out.ran {
        if r.plain && plain_ids.contains(&r.id) {
            let old = ledger.files.remove(&r.id).or_else(|| prev.remove(&r.id));
            let Some(mut rec) = old else {
                continue;
            };
            if !facts_held {
                // A plain run cannot discover the changed tools' read set.
                // Drop the old record: retaining it while updating facts
                // below would pair old trust with the new toolchain.
                unrec.insert(
                    r.id.clone(),
                    "tools changed: requires a fresh recording".into(),
                );
                continue;
            }
            let ok = r.state == "pass" || r.state == "skip";
            let tests = test_signature(r);
            rec.runs += 1;
            if rec.net && !r.denied {
                // A networked record's witness must run without network.
                ledger.files.insert(r.id.clone(), rec);
                continue;
            }
            if !ok || tests != rec.w.tests {
                if let Some(stamp) = &rec.promotion_stamp {
                    revoked_stamps.insert(stamp.clone());
                    lineage_disagreed += 1;
                }
                rec.w.disagreements += 1;
                rec.w.n = 1;
                rec.quarantined = true;
                pw.2 += 1;
                eprintln!(
                    "quod vitest: second witness DISAGREES (plain): {} (quarantined until it re-proves)",
                    rec.file
                );
                ledger.files.insert(r.id.clone(), rec);
                continue;
            }
            rec.duration = r.duration;
            rec.result = r.result.clone();
            rec.near_timeout = !r.near_timeout.is_empty();
            rec.plain_n += 1;
            rec.lineage = None;
            rec.margin = rec.margin.max(r.ratio);
            if let Some(t) = r.near_timeout.first() {
                pw.3 += 1;
                plain_near.push(format!("{} ({t})", r.file));
                if timing_replay() {
                    // A plain pass of the unchanged key is a witness of the
                    // code; only the label keeps the margin.
                    rec.w.n += 1;
                }
            } else {
                let was = rec.w.n >= bar(need, &rec.w);
                rec.w.n += 1;
                pw.1 += 1;
                if !was && rec.w.n >= bar(need, &rec.w) {
                    pw.0 += 1;
                }
            }
            ledger.files.insert(r.id.clone(), rec);
            continue;
        }
        let old = ledger.files.remove(&r.id).or_else(|| prev.remove(&r.id));
        // Quarantine is re-proved, not permanent: a quarantined file runs
        // every time, and once `reprove()` recordings in a row matched (a
        // one-off difference: a timeout under load), it may replay again.
        // A mismatch starts the count over.
        let was = old.as_ref().is_some_and(|o| o.quarantined);
        let bad = mismatched.contains(&r.id);
        if bad && let Some(stamp) = old.as_ref().and_then(|r| r.promotion_stamp.as_ref()) {
            revoked_stamps.insert(stamp.clone());
            lineage_disagreed += 1;
        }
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
        let per_run: Vec<&str> = keys
            .iter()
            .filter_map(|k| {
                k.strip_prefix("env:").filter(|n| {
                    volatile_env(n)
                        && !STEP_VOLATILE.contains(n)
                        && !VALUE_KEYED.contains(n)
                        && !n.starts_with("QUOD_")
                })
            })
            .collect();
        if !per_run.is_empty() {
            let v = per_run.join(", ");
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
        if let Some(t) = r.near_timeout.first() {
            near.push(format!("{} ({t})", r.file));
        }
        // Under plain witnessing a duration measured while recording never
        // decides the margin: the plain witness's does.
        let hooked_near = !r.near_timeout.is_empty() && !plain_mode;
        if need > 1 {
            witness_todo.push((
                r.id.clone(),
                old.clone(),
                mismatched.contains(&r.id),
                !r.near_timeout.is_empty(),
            ));
        }
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
                w: Witness {
                    tests: test_signature(r),
                    ..Witness::default()
                },
                near_timeout: hooked_near,
                plain_n: 0,
                lineage: None,
                promotion_stamp: None,
                margin: 0.0,
                net: r.net,
            },
        );
    }
    for id in &out.replayed {
        if invalidated_replays.contains(id) {
            continue;
        }
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
    lap("ingest", &mut phases);
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
    lap("states", &mut phases);
    let (mut promoted, mut disagreed, mut witnessed_again) = (0usize, 0usize, 0usize);
    let factor = near_timeout_factor();
    if !near.is_empty() {
        if plain_mode {
            eprintln!(
                "quod vitest: {} files had a test past {factor} of its timeout while recording (the plain second witness decides; e.g. {})",
                near.len(),
                near[0]
            );
        } else {
            eprintln!(
                "quod vitest: {} files had a test past {factor} of its timeout: recorded, not replayed until a run within it (e.g. {})",
                near.len(),
                near[0]
            );
        }
    }
    if !plain_near.is_empty() {
        eprintln!(
            "quod vitest: {} files had a test past {factor} of its timeout run plain: not replayed until a plain run within it (e.g. {})",
            plain_near.len(),
            plain_near[0]
        );
    }
    for (id, predecessor, mismatch, hooked_near) in witness_todo {
        let key = {
            let r = &ledger.files[&id];
            let mut h = blake3::Hasher::new();
            for x in ledger
                .args
                .iter()
                .chain(ledger.facts.iter().flat_map(|(k, v)| [k, v]))
            {
                h.update(x.as_bytes());
                h.update(b"\0");
            }
            for d in &r.deps {
                h.update(ledger.keys[*d as usize].as_bytes());
                h.update(b"\0");
                h.update(ledger.states[*d as usize].as_bytes());
                h.update(b"\0");
            }
            h.finalize().to_hex().to_string()
        };
        let r = ledger.files.get_mut(&id).expect("recorded above");
        r.w.key = key;
        let mut old = predecessor
            .as_ref()
            .map(|p| p.w.clone())
            .unwrap_or_default();
        let faulty_parent = predecessor
            .as_ref()
            .and_then(|p| p.promotion_stamp.as_ref())
            .is_some_and(|stamp| faulty_stamps.contains(stamp));
        if faulty_parent || (predecessor.as_ref().is_some_and(|p| p.near_timeout) && !plain_mode) {
            // A run near a timeout is no witness: the count starts over
            // from the first run within it. A known-faulty cohort cannot
            // supply a witness or let a replacement escape under a new stamp.
            old.key.clear();
        }
        r.w.disagreements = old.disagreements;
        let same_key = global_ok && !old.key.is_empty() && old.key == r.w.key;
        // A networked record is vouched for only by a witness with no
        // network (the plain one), never by another networked run.
        let agrees = same_key && !old.tests.is_empty() && old.tests == r.w.tests && !r.net;
        if mismatch || (same_key && !agrees) {
            // A second run of the same key disagreed (or an audit did).
            r.w.disagreements += 1;
            r.w.n = 1;
            r.quarantined = true;
            disagreed += 1;
            eprintln!(
                "quod vitest: second witness DISAGREES: {} (quarantined until it re-proves)",
                r.file
            );
        } else if agrees {
            // Keep batch provenance until the record's inputs change, so
            // a later bad plain audit still revokes its audited peers.
            r.promotion_stamp = predecessor.as_ref().and_then(|p| p.promotion_stamp.clone());
            r.lineage = predecessor.as_ref().and_then(|p| p.lineage.clone());
            let was = old.n >= bar(need, &old);
            r.w.n = old.n + 1;
            witnessed_again += 1;
            if r.w.n >= bar(need, &r.w) {
                r.quarantined = false;
                if !was {
                    promoted += 1;
                }
            }
        } else {
            r.w.n = 1;
            if lineage_enabled()
                && !faulty_parent
                && facts_held
                && lineage_context
                && !hooked_near
                && let Some(p) = &predecessor
                && promote_lineage(need, p, r, &stamp)
            {
                lineage_promoted += 1;
            }
        }
    }
    let lineage_revoked = revoke_lineage(&mut ledger.files, &revoked_stamps);
    if let Some(recovered) = recovery {
        for r in recovered.ran {
            // A recovery is execution evidence for this job, not a new
            // witness of the faulty recorder's dependency set.
            if let Some(rec) = ledger.files.get_mut(&r.id) {
                rec.runs += 1;
                if invalidated_replays.contains(&r.id)
                    && (!matches!(r.state.as_str(), "pass" | "skip")
                        || test_signature(&r) != rec.w.tests)
                {
                    rec.quarantined = true;
                    rec.w.disagreements += 1;
                }
            }
            let mut r = r;
            if let Some(index) = out.ran.iter().position(|old| old.id == r.id) {
                r.duration += out.ran[index].duration;
                out.ran[index] = r;
            } else {
                out.ran.push(r);
            }
        }
        out.replayed.retain(|id| !invalidated_replays.contains(id));
        narrowed.retain(|id, _| !invalidated_replays.contains(id));
        report_base.narrowed = narrowed.len();
    }
    eprintln!(
        "quod vitest: lineage: {lineage_promoted} promoted, {} plain audits, {lineage_disagreed} disagreed, {lineage_revoked} revoked",
        lineage_audits.len()
    );
    lap("witness", &mut phases);
    // After the witnesses: a run that makes every record trusted renders.
    if let Some(sb) = standby.filter(|_| everything_recorded(&ledger)) {
        let tr = Instant::now();
        match render(&a, &root, &dir, sb, &ledger) {
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

    lap("render", &mut phases);
    ledger.compact();
    if adaptive_recording {
        let shared = shared_fingerprint(&ledger, |id| ledger.states.get(id as usize).cloned());
        ledger
            .recording_admission
            .observe(shared, unstable_shared, false);
    }
    save_ledger(&ledger_path, &ledger)?;
    lap("write", &mut phases);
    let t_done = t0.elapsed();

    let quod_ms = (t_select + (t_done - t_run)).as_millis();
    if need > 1 {
        let plain_note = if plain_ids.is_empty() {
            String::new()
        } else {
            let (p, g, d, n) = pw;
            let all = plain_ids.len();
            format!(" ({all} plain: {p} trusted, {g} agreed, {d} disagreed, {n} near a timeout)")
        };
        eprintln!(
            "quod vitest: second witness: {} records now trusted, {} witnessed again, {} disagreed{plain_note}",
            promoted + pw.0,
            witnessed_again + pw.1,
            disagreed + pw.2,
        );
    }
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
    let ran_facts: Vec<_> = out
        .ran
        .iter()
        .map(|r| (r.id.clone(), r.file.clone(), r.state.clone(), r.duration))
        .collect();
    let replayed_ids: Vec<_> = out.replayed.iter().collect();
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
            "recording_policy": "record",
            "ran": out.ran.len(),
            "replayed": out.replayed.len(),
            "audited": audit.len(),
            "audit_mismatches": out.audit_mismatch,
            "failed": failed_all,
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
            "phases": phases,
            "timing_sensitive_replays": timing_note.len(),
            "lineage_promoted": lineage_promoted,
            "lineage_audits": lineage_audits.len(),
            "lineage_disagreed": lineage_disagreed,
            "lineage_revoked": lineage_revoked,
            "lineage_recovery": recovery_note,
            "witnesses_needed": need,
            "provisional": provisional,
            "promoted": promoted + pw.0,
            "witnessed_again": witnessed_again + pw.1,
            "witness_disagreed": disagreed + pw.2,
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
    let mut ledger = compatible_ledger(
        &fs::read(&ledger_path).context("no ledger to absorb into")?,
        &code_hash(),
    )
    .context("cannot absorb into a ledger from different recorder code or schema; record again")?;
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
    let need = witnesses_needed();
    for (id, rec) in &ledger.files {
        if rec.quarantined || !(rec.state == "pass" || rec.state == "skip") {
            return verdict(format!("{id} must run"));
        }
        if !replayable(need, rec) {
            return verdict(format!("{id} has one witness"));
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
    let Some((kind, p)) = key.split_once(':') else {
        return false;
    };
    // Restoring identical package bytes cannot recreate inode, timestamps
    // or other metadata, nor prove an arbitrary symlink resolution graph.
    // Check these observations before a job skips installation; the
    // lockfile is insufficient evidence for them.
    if matches!(kind, "sm" | "sl" | "rp") {
        return false;
    }
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
            l.files.get(id).is_some_and(|r| {
                !r.quarantined
                    && (r.state == "pass" || r.state == "skip")
                    && replayable(witnesses_needed(), r)
            })
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

/// The actual JS, Rust key/trust semantics, dependency versions and package
/// version. A changed recorder must collect its own dependencies before any
/// witness, lineage promotion or replay can trust an earlier observation.
fn code_hash() -> String {
    let mut h = blake3::Hasher::new();
    h.update(include_bytes!("vitest.rs"));
    h.update(include_bytes!("../../quod-key/src/lib.rs"));
    h.update(include_bytes!("../../../Cargo.lock"));
    h.update(b"\0");
    for (_, s) in js() {
        h.update(s.as_bytes());
        h.update(b"\0");
    }
    h.update(env!("CARGO_PKG_VERSION").as_bytes());
    h.update(b"\0");
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

/// Only successful fresh results and unaffected cached results can stand
/// in for execution during a plain retry. In particular, an audit's old
/// replay entry must not overwrite the failure that the audit just found.
fn plain_retry_replays(
    replay: &BTreeMap<String, String>,
    out: &Out,
    invalidated: &HashSet<String>,
) -> BTreeMap<String, String> {
    let mut again = replay.clone();
    again.retain(|id, _| !invalidated.contains(id));
    for r in &out.ran {
        again.remove(&r.id);
        if matches!(r.state.as_str(), "pass" | "skip") {
            again.insert(r.id.clone(), r.result.clone());
        }
    }
    again
}

/// Recompute current replays whose promotion cohort just failed an audit.
/// Preserve unrelated trusted replays and successful results executed in
/// this invocation. Failures run plain too, as in confirm_failures; no
/// stale passing result may replace a failed audit in the combined report.
fn recover_lineage(
    a: &VitestArgs,
    code_dir: &Path,
    plan: &serde_json::Value,
    replay: &BTreeMap<String, String>,
    out: &Out,
    dir: &Path,
    invalidated: &HashSet<String>,
) -> Result<(u8, Out)> {
    let again = plain_retry_replays(replay, out, invalidated);
    eprintln!(
        "quod vitest: lineage audit invalidated {} current replays; recomputing them plain before accepting the job",
        invalidated.len()
    );
    let mut p = plan.clone();
    p["replay"] = serde_json::to_value(again)?;
    p["record"] = serde_json::json!(false);
    p["audit"] = serde_json::json!([]);
    p["expect"] = serde_json::to_value(&out.facts)?;
    let pp = dir.join("plan-lineage-recover.json");
    let op = dir.join("out-lineage-recover.json");
    let _ = fs::remove_file(&op);
    fs::write(&pp, serde_json::to_vec(&p)?)?;
    let status = without_credentials(&mut Command::new(&a.node))
        .env("QUOD_VITEST_HIDE", hidden_env(a))
        .arg(code_dir.join("driver.mjs"))
        .arg(&pp)
        .arg(&op)
        .args(&a.args)
        .status()
        .context("starting plain lineage recovery")?;
    let recovered: Out = serde_json::from_slice(&fs::read(&op).context("recovery left no result")?)
        .context("invalid recovery result")?;
    anyhow::ensure!(
        !recovered.interrupted && recovered.facts == out.facts,
        "recovery interrupted or toolchain changed"
    );
    let expected: HashSet<_> = out
        .ran
        .iter()
        .map(|r| &r.id)
        .chain(out.replayed.iter())
        .collect();
    let actual: HashSet<_> = recovered
        .ran
        .iter()
        .map(|r| &r.id)
        .chain(recovered.replayed.iter())
        .collect();
    anyhow::ensure!(
        expected == actual && actual.len() == recovered.ran.len() + recovered.replayed.len(),
        "recovery changed the selected test inventory"
    );
    anyhow::ensure!(
        invalidated
            .iter()
            .all(|id| recovered.ran.iter().any(|r| &r.id == id)),
        "recovery did not execute every invalidated replay"
    );
    anyhow::ensure!(
        recovered
            .ran
            .iter()
            .all(|r| matches!(r.state.as_str(), "pass" | "skip" | "fail")),
        "recovery left an incomplete test result"
    );
    Ok((
        status.code().map_or(1, |c| c.clamp(0, 255) as u8),
        recovered,
    ))
}

/// The failures of an inline recording, run again plain (see the caller).
/// None when there is nothing to confirm (no failed file, or a file in
/// another state) or the confirming run could not run.
fn confirm_failures(
    a: &VitestArgs,
    code_dir: &Path,
    plan: &serde_json::Value,
    replay: &BTreeMap<String, String>,
    out: &Out,
    dir: &Path,
) -> Option<u8> {
    let failed = out.ran.iter().filter(|r| r.state == "fail").count();
    if failed == 0
        || out
            .ran
            .iter()
            .any(|r| !matches!(r.state.as_str(), "pass" | "skip" | "fail"))
    {
        return None;
    }
    let again = plain_retry_replays(replay, out, &HashSet::new());
    eprintln!(
        "quod vitest: {failed} files failed while recording; running them again without recording"
    );
    let mut p = plan.clone();
    p["replay"] = serde_json::to_value(&again).ok()?;
    p["record"] = serde_json::json!(false);
    p["audit"] = serde_json::json!([]);
    p["expect"] = serde_json::to_value(&out.facts).ok()?;
    let pp = dir.join("plan-confirm.json");
    fs::write(&pp, serde_json::to_vec(&p).ok()?).ok()?;
    let status = without_credentials(&mut Command::new(&a.node))
        .arg(code_dir.join("driver.mjs"))
        .arg(&pp)
        .arg(dir.join("out-confirm.json"))
        .args(&a.args)
        .status()
        .ok()?;
    let code = status.code().map_or(1, |c| c.clamp(0, 255) as u8);
    eprintln!(
        "quod vitest: without recording, the run {}",
        if code == 0 { "passes" } else { "fails too" }
    );
    Some(code)
}

/// A render started with the run: the driver loads vitest and its config
/// while the tests run, then waits for the go file (the records to replay)
/// or is killed. Rendering costs the replay itself, not a vitest start.
struct Standby {
    child: Option<std::process::Child>,
    rd: PathBuf,
    go: PathBuf,
    out: PathBuf,
}

impl Standby {
    /// Into `render-next` (the ledger's render stays usable until this one
    /// replaces it).
    fn start(a: &VitestArgs, dir: &Path, code_dir: &Path) -> std::result::Result<Self, String> {
        let rd = dir.join("render-next");
        let _ = fs::remove_dir_all(&rd);
        fs::create_dir_all(&rd).map_err(|e| e.to_string())?;
        let pp = rd.join("plan.json");
        let op = rd.join("out.json");
        let go = rd.join("go.json");
        let plan = serde_json::json!({
            "stateDir": dir, "replay": {}, "audit": [], "record": false,
            "render": rd, "standby": go,
        });
        fs::write(&pp, serde_json::to_vec(&plan).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let file = |n: &str| fs::File::create(rd.join(n)).map_err(|e| e.to_string());
        let child = without_credentials(&mut Command::new(&a.node))
            .env("QUOD_VITEST_HIDE", hidden_env(a))
            .arg(code_dir.join("driver.mjs"))
            .arg(&pp)
            .arg(&op)
            .args(&a.args)
            .env("QUOD_VITEST_QUIET", "1")
            .stdin(std::process::Stdio::null())
            .stdout(file("stdout")?)
            .stderr(file("stderr")?)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(Standby {
            child: Some(child),
            rd,
            go,
            out: op,
        })
    }
}

impl Standby {
    fn go(&self, v: &serde_json::Value) -> std::result::Result<(), String> {
        let tmp = self.go.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec(v).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        fs::rename(&tmp, &self.go).map_err(|e| e.to_string())
    }
}

/// Not rendering: told to stop (it exits at once, or as soon as vitest has
/// loaded, never in the middle of loading the config), killed after 2 s.
impl Drop for Standby {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = self.go(&serde_json::json!({ "abort": true }));
            let t = Instant::now();
            while t.elapsed() < std::time::Duration::from_secs(2) {
                if !matches!(c.try_wait(), Ok(None)) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Shadow side: replay every file through vitest once more (no tests run)
/// and keep what its reporters printed and wrote.
fn render(
    a: &VitestArgs,
    root: &Path,
    dir: &Path,
    mut sb: Standby,
    ledger: &Ledger,
) -> std::result::Result<Render, String> {
    let rd = sb.rd.clone();
    let replay: BTreeMap<&String, &String> =
        ledger.files.iter().map(|(id, r)| (id, &r.result)).collect();
    sb.go(&serde_json::json!({ "replay": replay, "expect": ledger.facts }))?;
    let status = sb
        .child
        .take()
        .ok_or("no render process")?
        .wait()
        .map_err(|e| e.to_string())?;
    let op = sb.out.clone();
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
    let fin = dir.join("render");
    let _ = fs::remove_dir_all(&fin);
    fs::rename(&rd, &fin).map_err(|e| e.to_string())?;
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
    if ledger.files.values().any(|r| r.lineage.is_some()) {
        return Err("pending lineage audits require selection".into());
    }
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

pub(crate) fn state(
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
        // VALUE_KEYED names are per-run in general but keyed by value (a
        // runner where the value differs is a miss): not the constant.
        "env" if volatile_env(p) && !VALUE_KEYED.contains(&p) => ("volatile".into(), None),
        "env" => (
            std::env::var_os(p).map_or("unset".into(), |v| {
                let v = yarn_stable(&v.to_string_lossy());
                blake3::hash(v.as_bytes()).to_hex()[..32].to_string()
            }),
            None,
        ),
        // set or not (record.mjs: a descriptor asked for by name)
        "envp" => (
            if std::env::var_os(p).is_some() {
                "set".into()
            } else {
                "unset".into()
            },
            None,
        ),
        "envall" => (envall.get_or_init(|| env_all(false)).clone(), None),
        "envtool" => (env_all(true), None),
        k if k
            .strip_prefix("ae")
            .or_else(|| k.strip_prefix('a'))
            .is_some_and(|mode| mode.len() == 1 && matches!(mode.as_bytes()[0], b'0'..=b'7')) =>
        {
            let effective = k.starts_with("ae");
            let mode = u32::from(k.as_bytes()[k.len() - 1] - b'0');
            let flags = if effective {
                rustix::fs::AtFlags::EACCESS
            } else {
                rustix::fs::AtFlags::empty()
            };
            let result = match rustix::fs::accessat(
                rustix::fs::CWD,
                path(),
                rustix::fs::Access::from_bits_retain(mode),
                flags,
            ) {
                Ok(()) => "ok".into(),
                Err(e) => format!("errno:{}", e.raw_os_error()),
            };
            (result, None)
        }
        "e" => (exists(&path()), None),
        "rp" => (
            realpath_footprint(&path())
                .unwrap_or_else(|| format!("unobservable:realpath:{}", ulid::Ulid::generate())),
            None,
        ),
        "sm" | "sl" => (stat_metadata(&path(), kind == "sl"), None),
        // A proven synchronous stat whose result is used only by native
        // isFile/isDirectory or a success check. Follow links exactly as
        // stat does, and retain errors: a direct caller may inspect them.
        "st" => (
            match fs::metadata(path()) {
                Ok(m) if m.is_file() => "file".into(),
                Ok(m) if m.is_dir() => "directory".into(),
                Ok(_) => "other".into(),
                Err(e) => e.raw_os_error().map_or_else(
                    || format!("unobservable:stat-type:{}", ulid::Ulid::generate()),
                    |errno| format!("errno:{errno}"),
                ),
            },
            None,
        ),
        "d" => (listing(&path(), false), None),
        "rj" => (listing_of(&path(), true, true), None),
        "pjf" | "pjq" => {
            let (fp, fields) = p.split_once('\0').unwrap_or((p, "[]"));
            let fp = if fp.starts_with('/') {
                PathBuf::from(fp)
            } else {
                root.join(fp)
            };
            (
                if kind == "pjq" {
                    pj_fields_query(&fp, fields, true)
                } else {
                    pj_fields(&fp, fields)
                },
                None,
            )
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
///
/// Only for a shadow (`--expect-report`), whose recording is checked against
/// the uninstrumented run of the same commit. Inline (the job that runs the
/// tests records them) nothing is hidden: the tests run exactly as plain
/// vitest's, and a result that depends on a per-run value is a second
/// witness that disagrees (quarantined).
fn hidden_env(a: &VitestArgs) -> String {
    if a.expect_report.is_none() {
        return String::new();
    }
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

/// Whether bubblewrap can give a process a network namespace of its own
/// here (unprivileged; hosted Ubuntu runners allow it).
fn bwrap_net_ok() -> bool {
    std::env::var("QUOD_VITEST_NETNS").map_or(true, |v| v != "0")
        && Command::new("bwrap")
            .args(["--dev-bind", "/", "/", "--unshare-net", "--", "true"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

fn origin_node_options(preload: &Path, previous: Option<&std::ffi::OsStr>) -> Option<String> {
    // Node exposes environment strings as UTF-8. Do not rewrite an invalid
    // value that its JS preload could not restore byte-for-byte.
    let preload = preload.to_str()?.replace('\\', "\\\\").replace('"', "\\\"");
    let mut options = format!("--require=\"{preload}\"");
    if let Some(previous) = previous {
        options.push(' ');
        options.push_str(previous.to_str()?);
    }
    Some(options)
}

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

/// The observable Linux Stats fields, without a content-only cache or
/// package-level substitution. statx supplies birth time as well as the
/// traditional stat fields. Unknown syscall support fails closed with an
/// unrepeatable key, so it cannot authorize a replay.
#[cfg(target_os = "linux")]
fn stat_metadata(p: &Path, nofollow: bool) -> String {
    use rustix::fs::{AtFlags, CWD, StatxFlags, statx};
    let flags = if nofollow {
        AtFlags::SYMLINK_NOFOLLOW
    } else {
        AtFlags::empty()
    };
    match statx(CWD, p, flags, StatxFlags::BASIC_STATS | StatxFlags::BTIME) {
        Ok(m) if m.stx_mask & StatxFlags::BASIC_STATS.bits() == StatxFlags::BASIC_STATS.bits() => {
            let fields: [i128; 21] = [
                m.stx_mask.into(),
                m.stx_blksize.into(),
                m.stx_nlink.into(),
                m.stx_uid.into(),
                m.stx_gid.into(),
                m.stx_mode.into(),
                m.stx_ino.into(),
                m.stx_size.into(),
                m.stx_blocks.into(),
                m.stx_dev_major.into(),
                m.stx_dev_minor.into(),
                m.stx_rdev_major.into(),
                m.stx_rdev_minor.into(),
                m.stx_atime.tv_sec.into(),
                m.stx_atime.tv_nsec.into(),
                m.stx_mtime.tv_sec.into(),
                m.stx_mtime.tv_nsec.into(),
                m.stx_ctime.tv_sec.into(),
                m.stx_ctime.tv_nsec.into(),
                m.stx_btime.tv_sec.into(),
                m.stx_btime.tv_nsec.into(),
            ];
            let mut h = blake3::Hasher::new();
            for field in fields {
                h.update(&field.to_le_bytes());
            }
            h.finalize().to_hex().to_string()
        }
        Err(rustix::io::Errno::NOENT) => "errno:ENOENT".into(),
        Err(rustix::io::Errno::NOTDIR) => "errno:ENOTDIR".into(),
        _ => format!("unobservable:stat:{}", ulid::Ulid::generate()),
    }
}

#[cfg(not(target_os = "linux"))]
fn stat_metadata(_p: &Path, _nofollow: bool) -> String {
    format!("unobservable:stat:{}", ulid::Ulid::generate())
}

fn wall_ns() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i128)
}

/// A successful realpath depends on its path components and every symlink
/// target traversed, not only on the last component's type. Retain both
/// lexical and kernel paths: Node's JS implementation normalizes link
/// targets while its native implementation delegates to the OS.
///
/// This only adds a dependency. It does not authorize dropping any stat
/// observation made inside realpath. Errors, special files, non-UTF-8 link
/// targets and excessive expansion fail closed instead of guessing an
/// equivalent Node error or an incomplete resolution graph.
fn realpath_footprint(input: &Path) -> Option<String> {
    use std::path::Component;

    fn lexical(path: &Path) -> PathBuf {
        let mut result = PathBuf::new();
        for part in path.components() {
            match part {
                Component::ParentDir => {
                    result.pop();
                }
                Component::CurDir => {}
                _ => result.push(part.as_os_str()),
            }
        }
        result
    }

    if !input.is_absolute() || input.to_str().is_none() {
        return None;
    }
    let normalized = lexical(input);
    // Do not reuse a negative lookup even if another path spelling succeeds.
    let native = fs::canonicalize(input).ok()?;
    let normalized_native = fs::canonicalize(&normalized).ok()?;
    let mut pending = vec![input.to_path_buf(), normalized];
    let mut visited = HashSet::new();
    let mut observations = BTreeMap::new();
    while let Some(path) = pending.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        if visited.len() > 128 || path.as_os_str().len() > 8192 {
            return None;
        }
        let parts: Vec<_> = path.components().collect();
        let mut prefix = PathBuf::new();
        for (index, part) in parts.iter().enumerate() {
            prefix.push(part.as_os_str());
            let metadata = fs::symlink_metadata(&prefix).ok()?;
            let value = if metadata.file_type().is_symlink() {
                let target = fs::read_link(&prefix).ok()?;
                let text = target.to_str()?;
                let mut expanded = if target.is_absolute() {
                    target.clone()
                } else {
                    prefix.parent()?.join(&target)
                };
                for suffix in &parts[index + 1..] {
                    expanded.push(suffix.as_os_str());
                }
                pending.push(lexical(&expanded));
                pending.push(expanded);
                format!("link:{text}")
            } else if metadata.is_dir() {
                "directory".into()
            } else if metadata.is_file() {
                "file".into()
            } else {
                return None;
            };
            if let Some(previous) = observations.insert(prefix.clone(), value.clone())
                && previous != value
            {
                return None;
            }
            if observations.len() > 2048 || pending.len() > 4096 {
                return None;
            }
        }
    }
    let mut hash = blake3::Hasher::new();
    for (path, value) in observations {
        let bytes = path.as_os_str().as_encoded_bytes();
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
        hash.update(&(value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    for resolved in [native, normalized_native] {
        let bytes = resolved.as_os_str().as_encoded_bytes();
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Some(hash.finalize().to_hex().to_string())
}

pub(crate) fn exists(p: &Path) -> String {
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

/// The package.json fields that list dependencies (keyed by their names).
const DEP_FIELDS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

/// A package.json the config parsed (`pjf:<path>\0<fields>`, record.mjs):
/// its top-level names in order, and the raw text of each field it read
/// (absent, or the file whole when it is not a JSON object).
fn pj_fields(path: &Path, fields: &str) -> String {
    pj_fields_query(path, fields, false)
}

/// The observed values plus language-level truthiness queries. Every raw
/// read remains in `values`; an additional boolean query never weakens it.
fn pj_fields_query(path: &Path, fields: &str, queries: bool) -> String {
    let Ok(bytes) = fs::read(path) else {
        return if path.exists() {
            "unreadable".into()
        } else {
            "absent".into()
        };
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Queries {
        values: Vec<Vec<String>>,
        truthy: Vec<String>,
    }
    let parsed = if queries {
        serde_json::from_str::<Queries>(fields).map(|q| (q.values, q.truthy))
    } else {
        serde_json::from_str::<Vec<Vec<String>>>(fields).map(|q| (q, Vec::new()))
    };
    let Ok((wanted, truthy)) = parsed else {
        // Unknown schemas cannot safely narrow a whole-file read.
        return blake3::hash(&bytes).to_hex().to_string();
    };
    // JSON.parse observes syntax errors even in fields the application
    // never subsequently reads. The raw-field scanner preserves spelling
    // and order, but is not itself a complete JSON grammar validator.
    if serde_json::from_slice::<serde_json::Value>(&bytes).is_err() {
        return blake3::hash(&bytes).to_hex().to_string();
    }
    let Some(top) = top_fields(&bytes) else {
        return blake3::hash(&bytes).to_hex().to_string();
    };
    let mut h = blake3::Hasher::new();
    for (k, _) in &top {
        h.update(k.as_bytes());
        h.update(b"\0");
    }
    h.update(b"\n");
    for path in &wanted {
        h.update(&serde_json::to_vec(path).unwrap_or_default());
        let mut value = bytes.as_slice();
        let mut found = true;
        for name in path {
            let next = top_fields(value).and_then(|fields| {
                fields
                    .into_iter()
                    .rev()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v)
            });
            match next {
                Some(v) => value = v,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if !found {
            h.update(b"!absent");
        } else if path.len() == 1 && DEP_FIELDS.contains(&path[0].as_str()) {
            // The top-level getter observes names; nested getters record
            // every version value actually read (including spread/clone).
            if let Some(fields) = top_fields(value) {
                h.update(b"=names:");
                for (name, _) in fields {
                    h.update(name.as_bytes());
                    h.update(b"\0");
                }
            } else {
                h.update(value);
            }
        } else {
            h.update(b"=");
            h.update(value);
        }
        h.update(b"\0");
    }
    for name in &truthy {
        h.update(b"?truthy:");
        h.update(name.as_bytes());
        h.update(b"\0");
        let value = top.iter().rev().find(|(k, _)| k == name);
        if let Some((_, field_bytes)) = value {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(field_bytes) else {
                return blake3::hash(&bytes).to_hex().to_string();
            };
            let yes = match value {
                serde_json::Value::Null => false,
                serde_json::Value::Bool(v) => v,
                serde_json::Value::Number(v) => v.as_f64().is_some_and(|n| n != 0.0),
                serde_json::Value::String(v) => !v.is_empty(),
                serde_json::Value::Array(_) | serde_json::Value::Object(_) => true,
            };
            h.update(if yes { b"true" } else { b"false" });
        } else {
            h.update(b"absent");
        }
        h.update(b"\0");
    }
    format!(
        "{}:{}",
        if queries { "pjq" } else { "pjf" },
        h.finalize().to_hex()
    )
}

/// A JSON object's top-level names and the raw text of their values, in
/// order; None when the text is not one.
pub(crate) fn top_fields(b: &[u8]) -> Option<Vec<(String, &[u8])>> {
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

pub(crate) fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
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
        Some(("envp", v)) => format!("whether the variable {v} is set"),
        Some(("d", p)) => format!("the directory listing of {p}"),
        Some(("rj", p)) => format!("the .json files under {p}"),
        Some(("pjf" | "pjq", p)) => {
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
    fn origin_prefix_preserves_absence_empty_and_quoted_original_options() {
        use std::ffi::OsStr;
        let p = Path::new("/tmp/quoted \"path\"/origin-preload.cjs");
        let prefix = "--require=\"/tmp/quoted \\\"path\\\"/origin-preload.cjs\"";
        assert_eq!(origin_node_options(p, None).as_deref(), Some(prefix));
        assert_eq!(
            origin_node_options(p, Some(OsStr::new(""))),
            Some(format!("{prefix} "))
        );
        let original = "--require=\"/tmp/user preload.cjs\" --import=data:text/javascript,void%200";
        assert_eq!(
            origin_node_options(p, Some(OsStr::new(original))),
            Some(format!("{prefix} {original}"))
        );
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            origin_node_options(p, Some(OsStr::from_bytes(&[0xff]))),
            None
        );
    }

    #[test]
    fn access_keys_see_permission_changes_without_content_changes() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("tool");
        fs::write(&p, b"same bytes").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
        let st = |key: &str| state(t.path(), key, None, 0, &std::sync::OnceLock::new(), None).0;
        let before = st("a1:tool");
        let content = st("f:tool");
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        assert_ne!(before, st("a1:tool"));
        assert_eq!(content, st("f:tool"));
        assert_eq!(st("a1:tool"), "ok");
    }

    #[test]
    fn stat_type_queries_follow_links_and_preserve_missing_and_directory_states() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("file");
        let dir = t.path().join("dir");
        fs::write(&file, "one").unwrap();
        fs::create_dir(&dir).unwrap();
        let st = |key: &str| state(t.path(), key, None, 0, &std::sync::OnceLock::new(), None).0;
        assert_eq!(st("st:file"), "file");
        fs::write(&file, "different bytes and size").unwrap();
        assert_eq!(st("st:file"), "file");
        assert_eq!(st("st:dir"), "directory");
        assert!(st("st:missing").starts_with("errno:"));
        assert_eq!(st("st:missing"), st("st:missing"));
        #[cfg(unix)]
        {
            let link = t.path().join("link");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert_eq!(st("st:link"), "file");
            fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink(&dir, &link).unwrap();
            assert_eq!(st("st:link"), "directory");
            fs::remove_dir(&dir).unwrap();
            assert_eq!(st("st:link"), st("st:missing"));
            // ENOTDIR and ENOENT must not collapse: a caller may choose
            // different tests depending on which error statSync throws.
            assert_ne!(st("st:file/child"), st("st:missing/child"));
        }
    }

    #[test]
    fn boolean_manifest_queries_keep_value_readers_and_fall_back_on_unknown_schemas() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("package.json");
        let state = |text: &str, schema: &str| {
            fs::write(&p, text).unwrap();
            pj_fields_query(&p, schema, true)
        };
        let truth = r#"{"values":[["name"]],"truthy":["exports"]}"#;
        let a = r#"{"name":"x","exports":{".":"./a.js","./unused":"./old.js"}}"#;
        let b = r#"{"name":"x","exports":{".":"./b.js"}}"#;
        assert_eq!(state(a, truth), state(b, truth));
        let values = r#"{"values":[["name"],["exports"]],"truthy":["exports"]}"#;
        assert_ne!(state(a, values), state(b, values));
        assert_ne!(
            state(a, truth),
            state(r#"{"name":"y","exports":{}}"#, truth)
        );
        assert_ne!(
            state(a, truth),
            state(r#"{"name":"x","exports":null}"#, truth)
        );
        assert_ne!(state(a, truth), state(r#"{"name":"x"}"#, truth));
        // JavaScript's ToBoolean, including empty objects/arrays and -0.
        for value in ["{}", "[]", "true", "1", "-1", r#""x""#] {
            assert_eq!(
                state(a, truth),
                state(&format!(r#"{{"name":"x","exports":{value}}}"#), truth)
            );
        }
        let no = state(r#"{"name":"x","exports":null}"#, truth);
        for value in ["null", "false", "0", "-0", r#""""#] {
            assert_eq!(
                no,
                state(&format!(r#"{{"name":"x","exports":{value}}}"#), truth)
            );
        }
        for schema in [
            "[]",
            r#"{"values":[],"unknown":[]}"#,
            r#"{"values":[],"truthy":{},"extra":0}"#,
        ] {
            assert_eq!(
                state(a, schema),
                blake3::hash(a.as_bytes()).to_hex().to_string()
            );
        }
        // A field outside serde_json's number range must retain the whole
        // manifest, including unrelated value inputs and property names.
        let huge = r#"{"name":"x","exports":1e999}"#;
        assert_eq!(
            state(huge, truth),
            blake3::hash(huge.as_bytes()).to_hex().to_string()
        );
    }

    #[test]
    fn manifest_queries_cannot_hide_invalid_json_in_unread_fields() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("package.json");
        let valid = r#"{"name":"x","exports":{},"unused":1}"#;
        for (schema, queries) in [
            (r#"[["name"]]"#, false),
            (r#"{"values":[["name"]],"truthy":["exports"]}"#, true),
        ] {
            fs::write(&p, valid).unwrap();
            let before = pj_fields_query(&p, schema, queries);
            for invalid in [
                r#"{"name":"x","exports":{},"unused":undefined}"#,
                r#"{"name":"x","exports":{},"unused":1,}"#,
                r#"{"name":"x","exports":{},"unused":[1,]}"#,
                r#"{"name":"x","exports":{},"unused":"\q"}"#,
            ] {
                fs::write(&p, invalid).unwrap();
                let after = pj_fields_query(&p, schema, queries);
                assert_ne!(before, after);
                assert_eq!(after, blake3::hash(invalid.as_bytes()).to_hex().to_string());
            }
        }
    }

    #[test]
    fn pj_fields_keys_names_and_the_fields_read() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("package.json");
        let st = |s: &str, fields: &str| {
            fs::write(&p, s).unwrap();
            pj_fields(&p, fields)
        };
        let deps = r#"[["dependencies"]]"#;
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
        // a dependency's version changes: the same (dependency lists are
        // keyed by their names; the files that load `b` key it themselves)
        assert_eq!(
            a,
            st(
                r#"{"name":"x","version":"1","dependencies":{"a":"1","b":"3"}}"#,
                deps
            )
        );
        // Config code may branch on a dependency version without ever
        // loading that package. Such a value must be keyed, too.
        let values = r#"[["dependencies"],["dependencies","b"]]"#;
        assert_ne!(
            st(r#"{"dependencies":{"a":"1","b":"2"}}"#, values),
            st(r#"{"dependencies":{"a":"1","b":"3"}}"#, values)
        );
        assert_eq!(
            st(r#"{"dependencies":{"a":"1","b":"2"}}"#, values),
            st(r#"{"dependencies":{"a":"9","b":"2"}}"#, values)
        );
        // another field read changes: not
        let scripts = r#"[["scripts"]]"#;
        assert_ne!(
            st(r#"{"scripts":{"t":"vitest"}}"#, scripts),
            st(r#"{"scripts":{"t":"vitest run"}}"#, scripts)
        );
        // its order changes, a name is added or removed: not
        assert_ne!(
            a,
            st(
                r#"{"name":"x","version":"1","dependencies":{"a":"1"}}"#,
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
    fn recording_backoff_requires_repeated_instability_and_recovers_on_stability() {
        let mut p = RecordingAdmission::default();
        assert!(!p.skip("cold", true));
        p.observe("cold".into(), false, false);
        for next in ["first-miss", "second-miss"] {
            assert!(!p.skip(next, true));
            p.observe(next.into(), true, false);
        }
        assert!(p.skip("third-miss", true));
        p.observe("third-miss".into(), true, true);
        assert!(
            !p.skip("third-miss", true),
            "stable inputs resume recording immediately"
        );
        p.observe("third-miss".into(), false, false);
        assert_eq!(p.misses, 0);
        assert!(!p.skip("changed-again", true));
    }

    #[test]
    fn recording_backoff_retries_after_seven_plain_runs() {
        let mut p = RecordingAdmission {
            shared: "old".into(),
            misses: 2,
            skipped: 0,
        };
        for i in 0..7 {
            let next = i.to_string();
            assert!(p.skip(&next, true));
            p.observe(next, true, true);
        }
        assert!(!p.skip("probe", true));
        p.observe("probe".into(), true, false);
        assert!(p.skip("next", true));
    }

    #[test]
    fn recording_backoff_never_overrides_required_recording_or_missing_state() {
        let p = RecordingAdmission {
            shared: "old".into(),
            misses: u32::MAX,
            skipped: 0,
        };
        assert!(!p.skip("new", false));
        assert!(!p.skip("", true));
        assert!(
            !RecordingAdmission {
                shared: String::new(),
                ..p.clone()
            }
            .skip("new", true)
        );
        let mut p = p;
        p.observe("new".into(), true, true);
        assert_eq!(p.misses, u32::MAX);
        assert_eq!(p.skipped, 1);
    }

    #[test]
    fn recording_hint_fingerprints_names_values_and_missing_inputs() {
        let mut l = Ledger {
            keys: vec!["sm:a".into(), "env:B".into()],
            global: vec![0, 1],
            ..Ledger::default()
        };
        let a = shared_fingerprint(&l, |id| Some(id.to_string()));
        l.global.reverse();
        assert_eq!(a, shared_fingerprint(&l, |id| Some(id.to_string())));
        assert_ne!(a, shared_fingerprint(&l, |id| Some((id + 1).to_string())));
        l.keys[0] = "sm:other".into();
        assert_ne!(a, shared_fingerprint(&l, |id| Some(id.to_string())));
        assert!(shared_fingerprint(&l, |_| None).is_empty());
        l.global.push(99);
        assert!(shared_fingerprint(&l, |id| Some(id.to_string())).is_empty());
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
    fn recorder_changes_invalidate_records_before_selection_or_witnessing() {
        let code = code_hash();
        let ledger = Ledger {
            version: LEDGER_VERSION,
            code: code.clone(),
            files: BTreeMap::from([(
                "trusted".into(),
                Rec {
                    state: "pass".into(),
                    plain_n: 2,
                    promotion_stamp: Some("old-recorder".into()),
                    ..Rec::default()
                },
            )]),
            ..Ledger::default()
        };
        let mut json = serde_json::to_value(&ledger).unwrap();
        let decode =
            |v: &serde_json::Value| compatible_ledger(&serde_json::to_vec(v).unwrap(), &code);
        assert_eq!(decode(&json).unwrap().files["trusted"].plain_n, 2);
        json["code"] = "other-recorder".into();
        assert!(decode(&json).is_none());
        json.as_object_mut().unwrap().remove("code");
        assert!(decode(&json).is_none(), "legacy ledgers must record anew");
        json["code"] = code.clone().into();
        json["version"] = (LEDGER_VERSION + 1).into();
        assert!(decode(&json).is_none());
        assert!(compatible_ledger(b"not json", "code").is_none());
        assert!(compatible_ledger(&serde_json::to_vec(&Ledger::default()).unwrap(), "").is_none());
    }

    #[test]
    fn absorb_rejects_incompatible_recorder_without_mutating_the_ledger() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let manifest = root.join("manifest.json");
        fs::write(&manifest, r#"{"entries":[]}"#).unwrap();
        let ledger_path = root.join("ledger.json");
        for (version, code) in [
            (LEDGER_VERSION, String::new()),
            (LEDGER_VERSION, "old-recorder".to_string()),
            (LEDGER_VERSION + 1, code_hash()),
        ] {
            let bytes = serde_json::to_vec(&Ledger {
                version,
                code,
                ..Ledger::default()
            })
            .unwrap();
            fs::write(&ledger_path, &bytes).unwrap();
            let err = absorb(root, root, &root.join("code"), &manifest).unwrap_err();
            assert!(
                err.to_string()
                    .contains("different recorder code or schema")
            );
            assert_eq!(fs::read(&ledger_path).unwrap(), bytes);
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
            code: code_hash(),
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
    fn realpath_observes_parent_links_without_volatile_metadata() {
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        for name in ["a", "b"] {
            fs::create_dir(root.join(name)).unwrap();
            fs::write(root.join(name).join("file"), "same").unwrap();
        }
        symlink("a", root.join("current")).unwrap();
        let input = root.join("current/file");
        let before = realpath_footprint(&input).unwrap();
        fs::File::options()
            .write(true)
            .open(root.join("a/file"))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH))
            .unwrap();
        assert_eq!(realpath_footprint(&input).unwrap(), before);
        fs::remove_file(root.join("a/file")).unwrap();
        fs::write(root.join("a/file"), "same").unwrap();
        assert_eq!(
            realpath_footprint(&input).unwrap(),
            before,
            "a new inode at the same resolved path is immaterial"
        );
        fs::remove_file(root.join("current")).unwrap();
        symlink("b", root.join("current")).unwrap();
        assert_ne!(realpath_footprint(&input).unwrap(), before);
        assert_eq!(
            exists(&input),
            "file",
            "leaf existence alone misses this change"
        );
        assert!(!installed("rp:node_modules/package/index.js"));
    }

    #[test]
    fn realpath_covers_links_inside_targets_and_parent_components() {
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        for name in ["a", "b"] {
            fs::create_dir_all(root.join(name).join("child")).unwrap();
            fs::write(root.join(name).join("file"), "same").unwrap();
        }
        symlink("a/child", root.join("middle")).unwrap();
        symlink("middle/../file", root.join("outer")).unwrap();
        fs::write(root.join("file"), "same").unwrap();
        let before = realpath_footprint(&root.join("outer")).unwrap();
        fs::remove_file(root.join("middle")).unwrap();
        symlink("b/child", root.join("middle")).unwrap();
        assert_ne!(realpath_footprint(&root.join("outer")).unwrap(), before);
        assert_eq!(
            fs::read_link(root.join("outer")).unwrap(),
            Path::new("middle/../file")
        );
    }

    #[test]
    fn realpath_failures_do_not_produce_reusable_keys() {
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        fs::write(root.join("file"), "same").unwrap();
        symlink("loop", root.join("loop")).unwrap();
        for p in ["missing", "file/child", "loop"] {
            assert!(realpath_footprint(&root.join(p)).is_none());
        }
        let env = std::sync::OnceLock::new();
        let first = state(root, "rp:missing", None, wall_ns(), &env, None).0;
        let next = state(root, "rp:missing", None, wall_ns(), &env, None).0;
        assert_ne!(first, next);
    }

    #[test]
    fn realpath_rejects_non_utf8_targets() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let target = std::ffi::OsStr::from_bytes(b"file-\xff");
        fs::write(root.join(target), "same").unwrap();
        symlink(target, root.join("link")).unwrap();
        assert!(fs::canonicalize(root.join("link")).is_ok());
        assert!(realpath_footprint(&root.join("link")).is_none());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn stat_metadata_sees_permissions_timestamps_and_link_identity() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let input = root.join("input");
        fs::write(&input, "unchanged bytes").unwrap();
        fs::set_permissions(&input, fs::Permissions::from_mode(0o644)).unwrap();
        std::os::unix::fs::symlink("input", root.join("link")).unwrap();
        let env = std::sync::OnceLock::new();
        let st = |k: &str| state(root, k, None, wall_ns(), &env, None).0;
        let content = st("f:input");
        let before = st("sm:input");
        assert_eq!(before, st("sm:input"), "unchanged metadata can replay");
        assert_eq!(before, st("sm:link"), "stat follows the link");
        let link = st("sl:link");
        fs::set_permissions(&input, fs::Permissions::from_mode(0o755)).unwrap();
        let permissions = st("sm:input");
        assert_ne!(before, permissions);
        assert_eq!(content, st("f:input"), "permission edit preserves bytes");
        assert_eq!(link, st("sl:link"), "lstat observes the link itself");
        fs::File::options()
            .write(true)
            .open(&input)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800)),
            )
            .unwrap();
        assert_ne!(permissions, st("sm:input"));
        assert_eq!(content, st("f:input"), "timestamp edit preserves bytes");
        assert_eq!(st("sm:missing"), "errno:ENOENT");
        assert_eq!(st("sm:input/child"), "errno:ENOTDIR");
        assert!(!installed("sm:node_modules/pkg/index.js"));
        assert!(!installed("sl:node_modules/pkg"));
        assert!(installed("p:node_modules/pkg"));
    }

    #[test]
    fn failed_audit_never_becomes_an_old_pass_during_plain_retry() {
        let replay = BTreeMap::from([
            ("audit".into(), "old-pass".into()),
            ("peer".into(), "cohort-pass".into()),
            ("control".into(), "independent-pass".into()),
        ]);
        let out = Out {
            ran: vec![
                Ran {
                    id: "audit".into(),
                    state: "fail".into(),
                    result: "old-pass".into(),
                    ..Ran::default()
                },
                Ran {
                    id: "fresh".into(),
                    state: "pass".into(),
                    result: "current-pass".into(),
                    ..Ran::default()
                },
            ],
            ..Out::default()
        };
        let retry = plain_retry_replays(&replay, &out, &HashSet::new());
        assert!(
            !retry.contains_key("audit"),
            "the failed audit must really execute again"
        );
        assert_eq!(retry.get("fresh").map(String::as_str), Some("current-pass"));
        assert!(retry.contains_key("peer") && retry.contains_key("control"));
        let recovered = plain_retry_replays(&replay, &out, &HashSet::from(["peer".into()]));
        assert!(!recovered.contains_key("peer") && !recovered.contains_key("audit"));
        assert!(recovered.contains_key("control") && recovered.contains_key("fresh"));
    }

    #[test]
    fn lineage_fault_detection_keeps_audited_cohort_provenance() {
        let ran = Ran {
            id: "audit".into(),
            state: "pass".into(),
            tests: Some("tests".into()),
            plain: true,
            ..Ran::default()
        };
        let mut ledger = Ledger::default();
        ledger.files.insert(
            "audit".into(),
            Rec {
                lineage: None,
                promotion_stamp: Some("batch".into()),
                w: Witness {
                    tests: test_signature(&ran),
                    ..Witness::default()
                },
                ..Rec::default()
            },
        );
        let mut out = Out {
            ran: vec![ran],
            ..Out::default()
        };
        let plain = HashSet::from(["audit".into()]);
        assert!(lineage_faults(&ledger, &out, &plain).0.is_empty());
        out.ran[0].state = "fail".into();
        let (stamps, files) = lineage_faults(&ledger, &out, &plain);
        assert_eq!(stamps, HashSet::from(["batch".into()]));
        assert_eq!(files, plain);
        assert!(lineage_faults(&ledger, &out, &HashSet::new()).0.is_empty());
        out.facts.insert("node".into(), "changed".into());
        assert!(lineage_faults(&ledger, &out, &plain).0.is_empty());
        out.audit_mismatch.push("audit".into());
        assert_eq!(lineage_faults(&ledger, &out, &HashSet::new()).0, stamps);
    }

    #[test]
    fn lineage_requires_plain_predecessor_same_reads_and_safe_matching_results() {
        let p = Rec {
            state: "pass".into(),
            deps: vec![0, 1],
            plain_n: 1,
            w: Witness {
                n: 2,
                key: "old".into(),
                tests: "test names and states".into(),
                disagreements: 0,
            },
            ..Rec::default()
        };
        let q = Rec {
            plain_n: 0,
            w: Witness {
                n: 1,
                key: "changed".into(),
                ..p.w.clone()
            },
            ..p.clone()
        };
        let mut promoted = q.clone();
        assert!(promote_lineage(2, &p, &mut promoted, "batch"));
        assert_eq!(promoted.w.n, 2);
        assert_eq!(promoted.plain_n, 0);
        assert_eq!(promoted.lineage.as_deref(), Some("batch"));
        // A shared input can force a new execution while the file's own
        // input states (and therefore its per-file witness key) stay equal.
        let mut shared_change = q.clone();
        shared_change.w.key = p.w.key.clone();
        assert!(promote_lineage(2, &p, &mut shared_change, "shared"));
        // Consecutive changed commits cannot invent a plain predecessor.
        let mut next = q.clone();
        next.w.key = "changed again".into();
        assert!(!promote_lineage(2, &promoted, &mut next, "next batch"));
        let mut no_plain = p.clone();
        no_plain.plain_n = 0;
        assert!(!promote_lineage(2, &no_plain, &mut q.clone(), "x"));
        for case in 0..8 {
            let mut unsafe_q = q.clone();
            match case {
                0 => unsafe_q.deps.push(2),
                1 => unsafe_q.w.tests = "changed test state".into(),
                2 => unsafe_q.state = "fail".into(),
                3 => unsafe_q.near_timeout = true,
                4 => unsafe_q.net = true,
                5 => unsafe_q.quarantined = true,
                6 => unsafe_q.w.key.clear(),
                _ => {
                    unsafe_q.pinned.insert(0, "old state".into());
                }
            }
            assert!(!promote_lineage(2, &p, &mut unsafe_q, "x"), "case {case}");
        }
        assert!(!promote_lineage(3, &p, &mut q.clone(), "x"));
    }

    #[test]
    fn lineage_audits_are_stable_and_revoke_the_whole_batch() {
        let rec = Rec {
            lineage: Some("batch".into()),
            promotion_stamp: Some("batch".into()),
            plain_n: 0,
            w: Witness {
                n: 2,
                ..Witness::default()
            },
            ..Rec::default()
        };
        let mut files: BTreeMap<_, _> = (0..17)
            .map(|i| (format!("file-{i}"), rec.clone()))
            .collect();
        let sampled = lineage_sample(files.iter());
        assert_eq!(sampled.len(), 3);
        assert_eq!(sampled, lineage_sample(files.iter().rev()));
        // An earlier agreeing audit clears pending status, not provenance.
        files.get_mut(&sampled[0]).unwrap().lineage = None;
        files.get_mut(&sampled[0]).unwrap().plain_n = 1;
        files.get_mut(&sampled[1]).unwrap().quarantined = true;
        files.insert(
            "other".into(),
            Rec {
                promotion_stamp: Some("other".into()),
                ..rec
            },
        );
        assert_eq!(
            revoke_lineage(&mut files, &HashSet::from(["batch".into()])),
            17
        );
        assert!(files[&sampled[1]].quarantined);
        assert_eq!(files[&sampled[0]].plain_n, 0);
        assert_eq!(files[&sampled[0]].w.n, 1);
        assert_eq!(files["other"].w.n, 2);
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
                w: Witness::default(),
                near_timeout: false,
                plain_n: 0,
                lineage: None,
                promotion_stamp: None,
                margin: 0.0,
                net: false,
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

    /// through-the-wall A1 (b), bench run 42035026447: stream.property's
    /// record (c0) and its plain witness (c1, 0.82 of the timeout) agreed;
    /// c2 ran it plain again for 85 s. Two agreeing witnesses, one of them
    /// plain, are enough with timing-sensitive replay on; the recording
    /// alone never is, and off means near-timeout files always run.
    #[test]
    fn a_near_timeout_file_replays_once_its_two_witnesses_agree() {
        let rec = |plain_n: u32, n: u32| Rec {
            project: String::new(),
            file: "stream.property.test.ts".into(),
            state: "pass".into(),
            duration: 85.0,
            result: "results/x.json".into(),
            deps: vec![0],
            quarantined: false,
            good: 0,
            runs: 2,
            replays: 0,
            cov: BTreeMap::new(),
            pinned: BTreeMap::new(),
            w: Witness {
                n,
                key: "k".into(),
                tests: "t".into(),
                disagreements: 0,
            },
            near_timeout: true,
            plain_n,
            lineage: None,
            promotion_stamp: None,
            margin: 0.82,
            net: false,
        };
        // c1 left it so: the recording and one plain witness, agreeing
        let after_c1 = rec(1, 2);
        assert!(timing_ok_if(true, 2, &after_c1), "c2 replays it");
        assert!(!timing_ok_if(false, 2, &after_c1), "off: it runs");
        assert!(
            !witnessed(2, &after_c1),
            "the witness policy alone keeps it running"
        );
        // the recording alone: no plain witness measured the margin
        assert!(!timing_ok_if(true, 2, &rec(0, 1)));
        // a plain witness, but the witnesses do not yet agree twice
        assert!(!timing_ok_if(true, 2, &rec(1, 1)));
        let mut q = rec(1, 2);
        q.quarantined = true;
        assert!(
            !timing_ok_if(true, 2, &q),
            "a timeout in an audit quarantines it"
        );
        let mut d = rec(1, 2);
        d.w.disagreements = 1;
        assert!(!timing_ok_if(true, 2, &d), "a disagreement raises the bar");
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

    #[test]
    fn value_keyed_names_hash_their_value_not_a_constant() {
        // RUNNER_TEMP, GITHUB_EVENT_PATH and GITHUB_ACTION_PATH are per-run
        // in general (VOLATILE, DENYLIST) but a fixed path on a hosted
        // runner: a read keys the value, so a runner where it differs is a
        // miss. They hashed to the constant "volatile" before the
        // exemption; a truly per-run name still does.
        let none = std::sync::OnceLock::new();
        let t = tempfile::tempdir().unwrap();
        // Explicit proof controls must participate even if an exemption is
        // accidentally omitted from VALUE_KEYED (the packaged off-switch
        // regression exposed exactly that omission for BOOLEAN_PROOF).
        for n in VALUE_KEYED
            .iter()
            .copied()
            .chain(["QUOD_VITEST_NAMES_PROOF", "QUOD_VITEST_BOOLEAN_PROOF"])
        {
            assert!(volatile_env(n), "{n} is no longer per-run in general");
            let want = std::env::var_os(n).map_or("unset".to_string(), |v| {
                let v = yarn_stable(&v.to_string_lossy());
                blake3::hash(v.as_bytes()).to_hex()[..32].to_string()
            });
            let got = state(t.path(), &format!("env:{n}"), None, 0, &none, None).0;
            assert_ne!(got, "volatile", "{n}");
            assert_eq!(got, want, "{n}");
        }
        let got = state(t.path(), "env:GITHUB_SHA", None, 0, &none, None).0;
        assert_eq!(got, "volatile");
    }
}
