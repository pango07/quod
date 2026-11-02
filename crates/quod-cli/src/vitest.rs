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

/// Bumped when what a key means or what is recorded changes: a ledger of
/// another version is dropped (every file runs once).
const LEDGER_VERSION: u32 = 1;

/// How old a file's times must be for its hash to be cached by identity
/// (git's racy-index rule, as `quod_cas`'s import stat cache).
const SETTLE_NS: i128 = 2_000_000_000;

/// Variables the GitHub runner sets per step (file-command paths); not keyed
/// by `envall:`, like `quod_key::unkeyed`'s per-run ones.
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
    /// Instead of running: fold an observer manifest (`quod inspect` of a
    /// `quod run` of the same suite) into the ledger. Every path the
    /// observer saw read that no recorded key accounts for becomes an input
    /// of every test file.
    #[arg(long, value_name = "MANIFEST")]
    absorb: Option<PathBuf>,
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
    #[serde(default)]
    runs: u32,
    #[serde(default)]
    replays: u32,
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
    // vite leaves them alone: the project's node_modules.
    let code = if root.join("node_modules").is_dir() {
        root.join("node_modules/.quod-vitest")
    } else {
        dir.join("code")
    };
    fs::create_dir_all(&code)?;
    for (name, body) in [
        ("driver.mjs", DRIVER),
        ("setup.mjs", SETUP),
        ("record.mjs", RECORD),
    ] {
        let p = code.join(name);
        if fs::read_to_string(&p).ok().as_deref() != Some(body) {
            fs::write(&p, body)?;
        }
    }
    if let Some(m) = &a.absorb {
        return absorb(&root, &dir, &code, m);
    }
    let full = a.full || std::env::var("QUOD_VITEST_FULL").is_ok_and(|v| v == "1");
    let ledger_path = dir.join("ledger.json");
    let mut ledger: Ledger = fs::read(&ledger_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|l: &Ledger| l.version == LEDGER_VERSION)
        .unwrap_or_default();
    ledger.version = LEDGER_VERSION;
    let t_load = t0.elapsed();

    // Every recorded input, hashed once.
    let live = ledger.live();
    let cur = states(&root, &ledger, &live);
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
        && ledger.args == a.args
        && ledger.global.iter().all(|k| !changed.contains(k));
    let mut replay: BTreeMap<String, String> = BTreeMap::new();
    let mut affected: Vec<String> = Vec::new();
    for (id, rec) in &ledger.files {
        let ok = global_ok
            && !rec.quarantined
            && (rec.state == "pass" || rec.state == "skip")
            && rec.deps.iter().all(|d| !changed.contains(d));
        if ok {
            replay.insert(id.clone(), rec.result.clone());
        } else {
            affected.push(id.clone());
        }
    }

    // Audits on the workers the files to run leave idle.
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    let n_audit = match a.audits.as_str() {
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
    while audit.len() < n_audit && !candidates.is_empty() {
        let i = rng.below(candidates.len());
        audit.push(candidates.swap_remove(i).clone());
    }

    let plan_path = dir.join("plan.json");
    let out_path = dir.join("out.json");
    let _ = fs::remove_file(&out_path);
    let plan = serde_json::json!({
        "stateDir": dir,
        "replay": replay,
        "audit": audit,
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

    let status = Command::new(&a.node)
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
    let (mut recorded, mut failed) = (0usize, 0usize);
    for r in &out.ran {
        let old = ledger.files.remove(&r.id);
        let quarantined = old.as_ref().is_some_and(|o| o.quarantined) || mismatched.contains(&r.id);
        for u in &r.unobservable {
            *unobservable.entry(u.clone()).or_default() += 1;
        }
        let Some(keys) = &r.keys else { continue };
        if !r.unobservable.is_empty() {
            continue;
        }
        if let Some(v) = keys
            .iter()
            .find_map(|k| k.strip_prefix("env:").filter(|n| volatile_env(n)))
        {
            *unobservable
                .entry(format!("reads the per-run variable {v}"))
                .or_default() += 1;
            continue;
        }
        if r.state != "pass" && r.state != "skip" {
            failed += 1;
            continue;
        }
        let mut deps: Vec<u32> = keys.iter().map(|k| key(&mut ledger, k)).collect();
        deps.sort_unstable();
        deps.dedup();
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
                runs: old.as_ref().map_or(0, |o| o.runs) + 1,
                replays: old.as_ref().map_or(0, |o| o.replays),
            },
        );
    }
    for id in &out.replayed {
        if let Some(r) = ledger.files.get_mut(id) {
            r.replays += 1;
        }
    }
    ledger.facts = out.facts;
    ledger.args = a.args.clone();

    // States: the selection's for keys it hashed, new keys now.
    let live = ledger.live();
    let missing: Vec<u32> = live
        .iter()
        .copied()
        .filter(|k| !now.contains_key(k))
        .collect();
    let fresh = states(&root, &ledger, &missing);
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
    eprintln!(
        "quod vitest: ran {} files ({} recorded, {} failed, {} unrecordable), replayed {}, audited {}{}; quod {} ms (select {} ms, record {} ms)",
        out.ran.len(),
        recorded,
        failed,
        out.ran.len() - recorded - failed,
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
    if let Some(p) = &a.summary {
        let s = serde_json::json!({
            "recorded_files": ledger.files.len(),
            "ran": out.ran.len(),
            "replayed": out.replayed.len(),
            "audited": audit.len(),
            "audit_mismatches": out.audit_mismatch,
            "failed": failed,
            "unrecordable": unobservable,
            "shared_inputs_changed": !global_ok,
            "shared_changed_keys": shared_changed.iter().take(20).collect::<Vec<_>>(),
            "inputs_hashed": live.len(),
            "load_ms": t_load.as_secs_f64() * 1e3,
            "select_ms": t_select.as_secs_f64() * 1e3,
            "vitest_ms": (t_run - t_select).as_secs_f64() * 1e3,
            "record_ms": (t_done - t_run).as_secs_f64() * 1e3,
            "driver": out.timings,
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
                format!("{}:{a}", if k == "f" { "f" } else { "e" })
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
    let st = states(root, &ledger, &new);
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
fn states(root: &Path, ledger: &Ledger, keys: &[u32]) -> Vec<(String, Option<[i64; 6]>)> {
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
        state(root, &ledger.keys[i], cached, started, &envall)
    })
}

fn state(
    root: &Path,
    key: &str,
    cached: Option<([i64; 6], &str)>,
    started: i128,
    envall: &std::sync::OnceLock<String>,
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
                blake3::hash(v.as_encoded_bytes()).to_hex()[..32].to_string()
            }),
            None,
        ),
        "envall" => (envall.get_or_init(env_all).clone(), None),
        "e" => (exists(&path()), None),
        "d" => (listing(&path(), false), None),
        "r" => (listing(&path(), true), None),
        "f" | "pj" | "p" => {
            let mut path = path();
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
            // Keep the identity only for a file settled before the read and
            // unchanged by it.
            let settled = started - (id[3].max(id[4]) as i128) > SETTLE_NS;
            let same = fs::metadata(&path).is_ok_and(|m2| ident(&m2) == id);
            (h, (settled && same).then_some(id))
        }
        _ => (format!("unknown kind {kind}"), None),
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

/// A directory's names and types (recursively: every path under it).
fn listing(p: &Path, recursive: bool) -> String {
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

/// A workspace package.json as the tools read it: the dependency version
/// values blanked (what is installed is keyed by the `p:` keys of the
/// packages loaded; a test that reads package.json itself keys it by `f:`).
fn pj_hash(bytes: &[u8]) -> String {
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return blake3::hash(bytes).to_hex().to_string();
    };
    for field in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(serde_json::Value::Object(m)) = v.get_mut(field) {
            for val in m.values_mut() {
                *val = serde_json::Value::String(String::new());
            }
        }
    }
    let canon = serde_json::to_vec(&v).unwrap_or_default();
    format!("pj:{}", blake3::hash(&canon).to_hex())
}

/// Every environment variable but the per-run and per-step ones.
fn env_all() -> String {
    let mut vars: Vec<(String, String)> = std::env::vars_os()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.to_string_lossy().into_owned(),
            )
        })
        .filter(|(k, _)| !volatile_env(k))
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

/// A per-run or per-step CI variable, or Quod's own.
fn volatile_env(name: &str) -> bool {
    quod_key::unkeyed(name) || STEP_VOLATILE.contains(&name) || name.starts_with("QUOD_")
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relativize_keeps_outside_paths_and_env() {
        let root = Path::new("/w/repo");
        assert_eq!(relativize(root, "f:/w/repo/src/a.ts"), "f:src/a.ts");
        assert_eq!(relativize(root, "f:/tmp/x"), "f:/tmp/x");
        assert_eq!(relativize(root, "env:HOME"), "env:HOME");
        assert_eq!(relativize(root, "envall:"), "envall:");
    }

    #[test]
    fn package_json_versions_do_not_count_but_the_rest_does() {
        let a = pj_hash(br#"{"name":"x","type":"module","devDependencies":{"eslint":"^9.1.0"}}"#);
        let b = pj_hash(br#"{"name":"x","type":"module","devDependencies":{"eslint":"^9.2.0"}}"#);
        let c = pj_hash(br#"{"name":"x","type":"commonjs","devDependencies":{"eslint":"^9.2.0"}}"#);
        let d = pj_hash(
            br#"{"name":"x","type":"module","devDependencies":{"eslint":"^9.2.0","vite":"1"}}"#,
        );
        assert_eq!(a, b);
        assert_ne!(b, c);
        assert_ne!(b, d, "a dependency added or removed counts");
    }

    #[test]
    fn states_see_content_listing_and_links() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        fs::write(root.join("a"), "1").unwrap();
        fs::create_dir(root.join("d")).unwrap();
        std::os::unix::fs::symlink("a", root.join("l")).unwrap();
        let env = std::sync::OnceLock::new();
        let st = |k: &str| state(root, k, None, wall_ns(), &env).0;
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
        let (s, _) = state(root, "f:a", Some((id, "cached")), wall_ns(), &env);
        assert_eq!(s, "cached");
        let mut other = id;
        other[2] += 1;
        let (s, _) = state(root, "f:a", Some((other, "cached")), wall_ns(), &env);
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
                runs: 1,
                replays: 0,
            },
        );
        l.compact();
        assert_eq!(l.keys, vec!["f:a".to_string(), "f:b".to_string()]);
        assert_eq!(l.global, vec![1]);
        assert_eq!(l.files["x"].deps, vec![0, 1]);
        assert_eq!(l.states, vec!["1".to_string(), "3".to_string()]);
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
