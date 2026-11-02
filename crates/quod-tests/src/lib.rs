//! Test-level replay (docs/test-level.md): the unit is a pytest test.
//!
//! A pytest step runs with Quod's plugin (`PLUGIN_SOURCE`) injected. The
//! plugin marks windows (each test's setup/call/teardown, each test
//! module's collection, each module- or session-scoped fixture, each
//! module's first import) with lookups the observer logs in order with the
//! step's file accesses (`quod_fs::Trace`), records which modules each
//! window imported, and records every executed test's reports. `attribute`
//! turns the trace and the plugin's outputs into one manifest per test and
//! one for the session, which every test's replay also requires.
//!
//! The rule everywhere: when an access cannot be attributed to one test for
//! certain, it is the session's, so it re-runs every test when it changes.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use quod_cas::{Hash, Kind};
use quod_fs::{Trace, Traced};
use quod_manifest::Access;
use serde::{Deserialize, Serialize};

/// The plugin, vendored: written into the sandbox at each test-level run.
pub const PLUGIN_SOURCE: &str = include_str!("../python/quod_pytest.py");
/// Its module name (`PYTEST_PLUGINS`).
pub const PLUGIN_MODULE: &str = "quod_pytest";
/// Where the plugin, the plan and the plugin's outputs live in the sandbox.
pub const SANDBOX_DIR: &str = "/quod/tl";
/// The step env entry that marks a step key as run at test level
/// (`QUOD_TEST_LEVEL=<KEY_TAG>`): a test-level run executes with the plugin,
/// so it is never the same artifact as a plain run of the step. Bumped when
/// the plugin's behaviour or the attribution rules change, which starts
/// every test of every step over.
pub const KEY_TAG: &str = "pytest-v1";
pub const KEY_ENV: &str = "QUOD_TEST_LEVEL";
/// Every Nth test-level run of a step executes every test (the full run
/// that checks replayed tests against what they do now, in their natural
/// order; docs/test-level.md, "Order dependence").
pub const FULL_EVERY: u64 = 10;

#[derive(Debug, thiserror::Error)]
pub enum TestsError {
    #[error("{0}")]
    Unusable(String),
    #[error("plugin output {path}: {source}")]
    Output {
        path: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

fn unusable(why: impl Into<String>) -> TestsError {
    TestsError::Unusable(why.into())
}

/// A test's key: the step key and its pytest node id (which names the
/// parameters of a parametrized test).
pub fn test_key(step: &Hash, nodeid: &str) -> Hash {
    Hash::of(format!("quod test key v1\0{}\0{nodeid}", step.hex()).as_bytes())
}

/// The key the step's session manifest is recorded under: what every test
/// depends on (conftest, plugins, config, imports from them, directory
/// listings, absent paths).
pub fn session_key(step: &Hash) -> Hash {
    Hash::of(format!("quod test session v1\0{}", step.hex()).as_bytes())
}

/// Does `script` run pytest (`pytest`, `py.test`, `python -m pytest`, a
/// path ending in `/pytest`)? Word-wise; a script that only mentions it in
/// a string is a false positive, which costs nothing but the plugin.
pub fn runs_pytest(script: &str) -> bool {
    let words: Vec<&str> = script
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '`'))
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().enumerate().any(|(i, w)| {
        let base = w.rsplit('/').next().unwrap_or(w);
        base == "pytest"
            || base == "py.test"
            || (*w == "pytest" && i > 0 && words[i - 1] == "-m")
    })
}

/// One test's recorded result, as the plan hands it back to the plugin.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub reports: Vec<serde_json::Value>,
    pub warnings: Vec<serde_json::Value>,
}

impl Record {
    /// The bytes stored as the test result's stdout blob: canonical JSON
    /// (sorted keys), the same for two runs that reported the same thing.
    pub fn to_blob(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "reports": self.reports,
            "warnings": self.warnings,
        }))
        .expect("JSON values serialize")
    }

    pub fn from_blob(b: &[u8]) -> Result<Record, serde_json::Error> {
        serde_json::from_slice(b)
    }

    /// Did any phase fail (a failure or an error)?
    pub fn failed(&self) -> bool {
        self.reports
            .iter()
            .any(|r| r.get("outcome").and_then(|o| o.as_str()) == Some("failed"))
    }
}

/// What the plugin is told (plan.json).
#[derive(Debug, Default, Serialize)]
pub struct Plan {
    pub token: String,
    pub replay: BTreeMap<String, Record>,
}

/// One process's output (`<pid>.json`), as the plugin writes it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PluginOut {
    pub v: u32,
    pub pid: u32,
    /// main (pytest without xdist), master or worker (xdist).
    pub role: String,
    pub workerid: Option<String>,
    /// [n, "b"|"e", kind, id]
    pub events: Vec<(u64, String, String, String)>,
    /// Begin marker of a window ("0": outside every window) to the modules
    /// it imported.
    pub edges: BTreeMap<String, Vec<String>>,
    /// Windows an import ran in off the main thread.
    pub offthread: Vec<u64>,
    pub collected: Vec<String>,
    pub tests: BTreeMap<String, TestOut>,
    pub replayed: Vec<String>,
    pub refused: Vec<String>,
    pub taint: Option<String>,
    pub done: bool,
    pub exitstatus: Option<i32>,
    pub imports_tracked: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct TestOut {
    pub module: Option<String>,
    pub reports: Vec<serde_json::Value>,
    pub warnings: Vec<serde_json::Value>,
    #[serde(default)]
    pub duration: f64,
}

/// Reads every process's output from `<dir>/out`.
pub fn read_outputs(dir: &Path) -> Result<Vec<PluginOut>, TestsError> {
    let out = dir.join("out");
    let mut v = Vec::new();
    let entries = match std::fs::read_dir(&out) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(v),
        Err(e) => {
            return Err(TestsError::Output {
                path: out.display().to_string(),
                source: e.into(),
            });
        }
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let path = e.path();
        let bytes = std::fs::read(&path).map_err(|s| TestsError::Output {
            path: path.display().to_string(),
            source: s.into(),
        })?;
        v.push(
            serde_json::from_slice(&bytes).map_err(|s| TestsError::Output {
                path: path.display().to_string(),
                source: s.into(),
            })?,
        );
    }
    v.sort_by_key(|o: &PluginOut| o.pid);
    Ok(v)
}

/// An executed test, attributed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Executed {
    pub accesses: BTreeSet<Access>,
    pub record: Record,
    pub duration_ms: u64,
}

/// The outcome of `attribute`.
#[derive(Clone, Debug, Default)]
pub struct Attribution {
    /// What every test depends on.
    pub session: BTreeSet<Access>,
    pub executed: BTreeMap<String, Executed>,
    /// Every test the session collected (after the command's own `-k`/`-m`
    /// selection), executed or replayed.
    pub collected: Vec<String>,
    pub replayed: BTreeSet<String>,
    pub refused: BTreeSet<String>,
    /// The owner process's pytest exit status.
    pub exitstatus: i32,
}

/// What a window is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Win {
    Test(String),
    /// A test module's collection, or a module- or class-scoped fixture.
    Module(String),
    /// A session- or package-scoped fixture.
    Session,
    /// A module's first import.
    Load(String),
}

fn win(kind: &str, id: &str) -> Result<Win, TestsError> {
    Ok(match kind {
        "t" => Win::Test(id.into()),
        "c" | "m" => Win::Module(id.into()),
        "s" => Win::Session,
        "l" => Win::Load(id.into()),
        k => return Err(unusable(format!("unknown window kind {k:?}"))),
    })
}

/// One process's windows over sequence numbers: `(start, window)`, sorted;
/// a window holds the accesses after `start` up to the next start. `None`
/// is outside every window.
struct Segments {
    starts: Vec<(u64, Option<(u64, Win)>)>,
}

impl Segments {
    fn at(&self, seq: u64) -> Option<&(u64, Win)> {
        let i = self.starts.partition_point(|(s, _)| *s < seq);
        if i == 0 {
            return None;
        }
        self.starts[i - 1].1.as_ref()
    }
}

/// Attributes a test-level run's accesses (docs/test-level.md,
/// "Attribution"). `Err` when the run cannot be attributed with certainty
/// (a process's markers lost, a tainted plugin, a session that did not
/// finish): the caller then records nothing at test level.
pub fn attribute(trace: &Trace, outs: &[PluginOut]) -> Result<Attribution, TestsError> {
    let owners: Vec<&PluginOut> = outs
        .iter()
        .filter(|o| o.role == "main" || o.role == "master")
        .collect();
    let [owner] = owners.as_slice() else {
        return Err(unusable(format!(
            "{} pytest sessions owned the step (one is needed)",
            owners.len()
        )));
    };
    let runners: Vec<&PluginOut> = match owner.role.as_str() {
        "main" => vec![owner],
        _ => outs.iter().filter(|o| o.role == "worker").collect(),
    };
    if runners.is_empty() {
        return Err(unusable("xdist ran with no worker output"));
    }
    for o in outs {
        if let Some(t) = &o.taint {
            return Err(unusable(format!("pytest process {}: {t}", o.pid)));
        }
        if !o.done {
            return Err(unusable(format!(
                "pytest process {} did not finish its session",
                o.pid
            )));
        }
        if o.role != "master" && !o.imports_tracked {
            return Err(unusable(format!(
                "pytest process {}: imports not tracked",
                o.pid
            )));
        }
    }
    let tgid = |tid: u32| trace.procs.get(&tid).map_or(tid, |p| p.tgid);
    let mut segments: HashMap<u32, Segments> = HashMap::new();
    for o in &runners {
        let markers: Vec<(u64, u64)> = trace
            .events
            .iter()
            .filter_map(|e| match e.what {
                Traced::Marker(n) if tgid(e.tid) == o.pid => Some((e.seq, n)),
                _ => None,
            })
            .collect();
        if markers.len() != o.events.len()
            || markers
                .iter()
                .zip(&o.events)
                .any(|((_, n), (m, ..))| n != m)
        {
            return Err(unusable(format!(
                "pytest process {}: {} markers traced, {} logged",
                o.pid,
                markers.len(),
                o.events.len()
            )));
        }
        let mut stack: Vec<(u64, Win)> = Vec::new();
        let mut starts = Vec::with_capacity(markers.len());
        for ((seq, n), (_, be, kind, id)) in markers.iter().zip(&o.events) {
            match be.as_str() {
                "b" => stack.push((*n, win(kind, id)?)),
                "e" => {
                    let w = win(kind, id)?;
                    match stack.pop() {
                        Some((_, open)) if open == w => {}
                        _ => {
                            return Err(unusable(format!(
                                "pytest process {}: window {kind} {id} closed out of order",
                                o.pid
                            )));
                        }
                    }
                }
                x => return Err(unusable(format!("unknown marker {x:?}"))),
            }
            starts.push((*seq, stack.last().cloned()));
        }
        segments.insert(o.pid, Segments { starts });
    }
    let offthread: BTreeSet<(u32, u64)> = runners
        .iter()
        .flat_map(|o| o.offthread.iter().map(|n| (o.pid, *n)))
        .collect();

    // The process whose windows an access belongs to: the requester's
    // thread group or its nearest ancestor that runs tests.
    let runner_of = |tid: u32| -> Option<u32> {
        if tid == 0 {
            return None;
        }
        let mut p = tgid(tid);
        for _ in 0..64 {
            if segments.contains_key(&p) {
                return Some(p);
            }
            let info = trace.procs.get(&p)?;
            if info.ppid <= 1 {
                return None;
            }
            p = trace.procs.get(&info.ppid).map_or(info.ppid, |i| i.tgid);
        }
        None
    };

    let mut session: BTreeSet<Access> = BTreeSet::new();
    let mut tests: BTreeMap<String, BTreeSet<Access>> = BTreeMap::new();
    let mut modules: BTreeMap<String, BTreeSet<Access>> = BTreeMap::new();
    let mut loads: BTreeMap<String, BTreeSet<Access>> = BTreeMap::new();
    for e in &trace.events {
        let Traced::Access(a) = &e.what else {
            continue;
        };
        // Only a file's every access reaches the observer again in a later
        // window (quod_fs::Options::trace); a listing, an absent name or a
        // symlink may be answered from a cache, so whoever asked first is
        // not the only one who depends on it.
        if a.kind != Kind::File {
            session.insert(a.clone());
            continue;
        }
        let at = runner_of(e.tid).and_then(|p| {
            segments[&p]
                .at(e.seq)
                .map(|(n, w)| (p, *n, w))
        });
        let bucket = match at {
            None => &mut session,
            Some((p, n, _)) if offthread.contains(&(p, n)) => &mut session,
            Some((_, _, Win::Session)) => &mut session,
            Some((_, _, Win::Test(id))) => tests.entry(id.clone()).or_default(),
            Some((_, _, Win::Module(id))) => modules.entry(id.clone()).or_default(),
            Some((_, _, Win::Load(m))) => loads.entry(m.clone()).or_default(),
        };
        bucket.insert(a.clone());
    }

    // Imports: each window's, by what it is.
    let mut session_imports: BTreeSet<String> = BTreeSet::new();
    let mut test_imports: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut module_imports: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for o in &runners {
        let windows: HashMap<u64, Win> = o
            .events
            .iter()
            .filter(|(_, be, ..)| be == "b")
            .map(|(n, _, k, id)| Ok((*n, win(k, id)?)))
            .collect::<Result<_, TestsError>>()?;
        for (n, mods) in &o.edges {
            let n: u64 = n
                .parse()
                .map_err(|_| unusable(format!("bad window {n:?} in edges")))?;
            let into = match windows.get(&n) {
                None if n == 0 => &mut session_imports,
                None => return Err(unusable(format!("edges of unknown window {n}"))),
                Some(_) if offthread.contains(&(o.pid, n)) => &mut session_imports,
                Some(Win::Session) => &mut session_imports,
                Some(Win::Test(id)) => test_imports.entry(id.clone()).or_default(),
                Some(Win::Module(id)) => module_imports.entry(id.clone()).or_default(),
                Some(Win::Load(m)) => deps.entry(m.clone()).or_default(),
            };
            into.extend(mods.iter().cloned());
        }
    }
    let closure = |roots: &BTreeSet<String>| -> BTreeSet<Access> {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut todo: Vec<&str> = roots.iter().map(String::as_str).collect();
        let mut out = BTreeSet::new();
        while let Some(m) = todo.pop() {
            if !seen.insert(m) {
                continue;
            }
            if let Some(r) = loads.get(m) {
                out.extend(r.iter().cloned());
            }
            if let Some(d) = deps.get(m) {
                todo.extend(d.iter().map(String::as_str));
            }
        }
        out
    };
    session.extend(closure(&session_imports));

    let (empty, none) = (BTreeSet::new(), BTreeSet::new());
    let mut executed = BTreeMap::new();
    let mut collected = Vec::new();
    let mut replayed = BTreeSet::new();
    let mut refused = BTreeSet::new();
    for o in outs {
        replayed.extend(o.replayed.iter().cloned());
    }
    for o in &runners {
        refused.extend(o.refused.iter().cloned());
        if collected.is_empty() {
            collected = o.collected.clone();
        } else if collected != o.collected {
            return Err(unusable("xdist workers collected different tests"));
        }
        for (nodeid, t) in &o.tests {
            let module = t.module.clone().unwrap_or_default();
            let mut imports = test_imports.get(nodeid).cloned().unwrap_or_default();
            imports.extend(module_imports.get(&module).unwrap_or(&empty).iter().cloned());
            let mut accesses = closure(&imports);
            accesses.extend(tests.get(nodeid).unwrap_or(&none).iter().cloned());
            accesses.extend(modules.get(&module).unwrap_or(&none).iter().cloned());
            let rec = Executed {
                accesses,
                record: Record {
                    reports: t.reports.clone(),
                    warnings: t.warnings.clone(),
                },
                duration_ms: (t.duration.max(0.0) * 1000.0) as u64,
            };
            if executed.insert(nodeid.clone(), rec).is_some() {
                return Err(unusable(format!("{nodeid} ran twice")));
            }
        }
    }
    for t in &replayed {
        if executed.contains_key(t) {
            return Err(unusable(format!("{t} both replayed and executed")));
        }
    }
    let all: BTreeSet<&String> = collected.iter().collect();
    if executed.keys().chain(replayed.iter()).any(|t| !all.contains(t))
        || all.len() != executed.len() + replayed.len()
    {
        return Err(unusable(format!(
            "{} tests collected, {} executed and {} replayed",
            all.len(),
            executed.len(),
            replayed.len()
        )));
    }
    Ok(Attribution {
        session,
        executed,
        collected,
        replayed,
        refused,
        exitstatus: owner.exitstatus.unwrap_or(-1),
    })
}

#[cfg(test)]
mod tests;
