//! `quod gotest`: package-level replay for `go test`, recording off the
//! critical path (the vitest shadow design; docs/gotest-shadow.md).
//!
//! The unit is a package's test binary. A package replays when every input
//! its record holds is unchanged:
//!
//! - global: the toolchain (`go env`, but GOCACHE and the per-invocation
//!   GOGCCFLAGS), every environment variable (but the per-run CI ones),
//!   go test's flags, the directory, the CPU count, go.work;
//! - its compiled closure, from `go list -deps -test`: every source, cgo,
//!   assembly, syso and embedded file of every package its test binary
//!   links, by content (module-cache packages by path@version, standard
//!   ones by the toolchain), each module's go.mod, default.pgo;
//! - what its test binary touched while it ran, as strace saw it in the
//!   shadow: every file it opened, every directory it listed, every path it
//!   stat'd or whose link it read, by their states now.
//!
//! **Critical path** (`--no-record`): go list, hash, select; `go test` on
//! exactly the packages that must run, with the developer's flags and
//! nothing added; the replayed packages' recorded output is merged into go
//! test's stdout in go test's own package order, so the stream (text or
//! `-json`) and the exit code are a full run's. The ledger is read, never
//! written; `--report` writes what ran for the shadow.
//!
//! **Shadow** (a separate, non-blocking job): the same selection plus
//! `--audits` replayable packages, run with `-count=1 -exec <strace
//! wrapper>`. A package records only when its trace is complete (no process
//! but its own binary, no network beyond its own loopback listeners, no
//! write to the workspace or outside the temp directories, no syscall the
//! observer does not know) and its results match what the critical path
//! reported for it (`--expect-report`). An audit that differs from its
//! record, and a recording that changed behaviour, quarantine the package:
//! it always runs from then on. A failure never records: it always runs.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

mod go_src;

/// Bumped when what a key means or what is recorded changes.
const LEDGER_VERSION: u32 = 1;

/// Variables the GitHub runner sets per step; not keyed (as `quod vitest`).
const STEP_VOLATILE: &[&str] = &[
    "GITHUB_OUTPUT",
    "GITHUB_ENV",
    "GITHUB_PATH",
    "GITHUB_STATE",
    "GITHUB_STEP_SUMMARY",
    "OLDPWD",
    "_",
];

/// `go env` entries that do not change what a test does: where the build
/// cache lives (it is content-addressed), and the C flags go spells with a
/// fresh temporary directory on every invocation (the C flags that matter
/// are CGO_*, keyed).
const UNKEYED_GO_ENV: &[&str] = &["GOCACHE", "GOGCCFLAGS"];

/// What strace logs of a test binary: every syscall that names a path, and
/// the socket calls that say where it connected and what it listened on.
const TRACE_SET: &str = "%file,fchdir,socket,connect,bind,sendto,sendmsg,getsockname";

/// The `go list` fields selection reads.
const LIST_FIELDS: &str = "Dir,ImportPath,Name,Standard,ForTest,DepOnly,Module,GoFiles,CgoFiles,CFiles,CXXFiles,MFiles,HFiles,FFiles,SFiles,SwigFiles,SwigCXXFiles,SysoFiles,EmbedFiles,TestGoFiles,XTestGoFiles,TestEmbedFiles,XTestEmbedFiles,Deps,Error,DepsErrors,Incomplete";

/// go test's boolean flags `quod gotest` understands. Anything else (a
/// profile, coverage, -exec, -args, a benchmark, -c, -o, ...) runs plain go
/// test with no replay.
const BOOL_FLAGS: &[&str] = &[
    "v", "json", "race", "msan", "asan", "short", "failfast", "trimpath", "fullpath", "a", "x",
];

/// go test's flags with a value `quod gotest` understands (keyed as given).
const VALUE_FLAGS: &[&str] = &[
    "run", "skip", "count", "timeout", "tags", "p", "parallel", "cpu", "mod", "gcflags", "ldflags",
    "asmflags", "vet", "shuffle", "buildvcs", "pgo",
];

/// The flags that change which packages `go list` loads.
const LIST_FLAGS: &[&str] = &["tags", "mod", "race", "msan", "asan"];

#[derive(clap::Args)]
pub struct GotestArgs {
    /// Where the ledger lives (default: .quod/gotest under the current
    /// directory).
    #[arg(long)]
    dir: Option<PathBuf>,
    /// Run every package (and, recording, record every one). Also
    /// QUOD_GOTEST_FULL=1.
    #[arg(long)]
    full: bool,
    /// The critical path: select, run what must run with plain go test
    /// (nothing recorded, nothing added) and replay the rest; the ledger is
    /// not touched. A shadow run of the same commit without this flag
    /// records the packages that ran, off the job's critical path.
    #[arg(long)]
    no_record: bool,
    /// Shadow runs: replayable packages to run and compare with their
    /// records (a mismatch quarantines the package).
    #[arg(long, default_value_t = 4)]
    audits: usize,
    /// Shadow runs: the critical path's `--report` of the same commit. A
    /// package records only if its results here match the report's;
    /// otherwise recording changed its behaviour and it is quarantined.
    #[arg(long, value_name = "REPORT")]
    expect_report: Option<PathBuf>,
    /// Write what ran (each package's output, status and test results) as
    /// JSON: the shadow's --expect-report.
    #[arg(long, value_name = "FILE")]
    report: Option<PathBuf>,
    /// Write this run's numbers (packages run, replayed, why, timings).
    #[arg(long)]
    summary: Option<PathBuf>,
    /// The go command.
    #[arg(long, default_value = "go")]
    go: String,
    /// The strace the shadow records with.
    #[arg(long, default_value = "strace")]
    strace: String,
    /// go test's flags and packages, e.g. `-json ./...`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct Ledger {
    version: u32,
    /// `code_hash` of the quod that wrote it.
    code: String,
    /// By import path.
    pkgs: BTreeMap<String, Rec>,
    /// What go list and go env said at the last record, and what that
    /// depended on: the critical path reuses it while all of that holds.
    #[serde(default)]
    world: Option<Cached>,
    /// The shapes of the files records narrow on, by file state.
    #[serde(default)]
    shapes: BTreeMap<String, FileShape>,
}

/// `World` as the shadow listed it, with what the listing depended on.
#[derive(Serialize, Deserialize, Clone, Default)]
struct Cached {
    /// `Tree::env`, `Tree::names`, `Tree::meta`.
    env: String,
    names: String,
    meta: String,
    /// Every .go file under the root: its state and its header
    /// (`go_src::header`; None: unreadable to it).
    files: BTreeMap<String, (String, Option<String>)>,
    roots: Vec<String>,
    pkgs: Vec<Pkg>,
    goenv: BTreeMap<String, String>,
}

/// A file's skeleton and its functions' bodies and lines (`go_src::Shape`).
#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
struct FileShape {
    skeleton: String,
    funcs: BTreeMap<String, (String, u32)>,
}

impl From<&go_src::Shape> for FileShape {
    fn from(s: &go_src::Shape) -> FileShape {
        FileShape {
            skeleton: s.skeleton.clone(),
            funcs: s
                .funcs
                .iter()
                .map(|f| (f.id.clone(), (f.body.clone(), f.line)))
                .collect(),
        }
    }
}

/// What go list's answer depends on, as of now.
struct Tree {
    /// The go command's identity, every environment variable, the go env
    /// file, the directory, the flags go list takes, the patterns.
    env: String,
    /// Every path under the root (names and kinds, not contents).
    names: String,
    /// go.mod, go.sum, go.work, go.work.sum and modules.txt contents.
    meta: String,
    /// Every .go file under the root, by state.
    gofiles: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct Rec {
    /// `global_key` when recorded.
    global: String,
    /// `closures` of the package when recorded.
    closure: String,
    /// What its test binary touched (`f:`, `d:`, `s:`, `l:` + path), with
    /// each one's state.
    inputs: BTreeMap<String, String>,
    /// go test's stdout for the package, as the critical path printed it.
    output: String,
    /// `ok` or `nofiles` (a failure is never recorded).
    status: String,
    #[serde(default)]
    tests: BTreeMap<String, String>,
    /// Why it always runs.
    #[serde(default)]
    quarantined: Option<String>,
    /// `Clo::structure` when recorded.
    #[serde(default)]
    structure: String,
    /// `Clo::local` when recorded.
    #[serde(default)]
    local: BTreeMap<String, String>,
    /// Per file of `local`: the functions with statements its test binary
    /// ran none of (a change there cannot change its results).
    #[serde(default)]
    cold: BTreeMap<String, BTreeSet<String>>,
    /// Coverage was recorded: `cold` can be trusted.
    #[serde(default)]
    covered: bool,
}

/// One package's part of go test's stdout.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
struct Chunk {
    output: String,
    /// `ok`, `fail`, `nofiles`, or empty (go test said nothing final).
    status: String,
    /// Per test: pass, fail or skip (from -json events or -v lines).
    #[serde(default)]
    tests: BTreeMap<String, String>,
    /// The output was not UTF-8 (`output` is lossy; never recorded).
    #[serde(default)]
    binary: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Report {
    args: Vec<String>,
    ran: BTreeMap<String, Chunk>,
    replayed: Vec<String>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "PascalCase")]
struct Module {
    #[serde(default)]
    path: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    main: bool,
    #[serde(default)]
    go_mod: String,
    #[serde(default)]
    replace: Option<Box<Module>>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "PascalCase")]
struct Pkg {
    #[serde(default)]
    dir: String,
    #[serde(default)]
    import_path: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    standard: bool,
    /// Set on the test build's own variants (`P [P.test]`, `P_test
    /// [P.test]`, a dependency recompiled against P's test files).
    #[serde(default)]
    for_test: String,
    #[serde(default)]
    module: Option<Module>,
    #[serde(default)]
    go_files: Vec<String>,
    #[serde(default)]
    cgo_files: Vec<String>,
    #[serde(default)]
    c_files: Vec<String>,
    #[serde(default, rename = "CXXFiles")]
    cxx_files: Vec<String>,
    #[serde(default)]
    m_files: Vec<String>,
    #[serde(default)]
    h_files: Vec<String>,
    #[serde(default)]
    f_files: Vec<String>,
    #[serde(default)]
    s_files: Vec<String>,
    #[serde(default)]
    swig_files: Vec<String>,
    #[serde(default, rename = "SwigCXXFiles")]
    swig_cxx_files: Vec<String>,
    #[serde(default)]
    syso_files: Vec<String>,
    #[serde(default)]
    embed_files: Vec<String>,
    #[serde(default)]
    test_go_files: Vec<String>,
    #[serde(default)]
    x_test_go_files: Vec<String>,
    #[serde(default)]
    test_embed_files: Vec<String>,
    #[serde(default)]
    x_test_embed_files: Vec<String>,
    #[serde(default)]
    deps: Vec<String>,
    #[serde(default)]
    error: Option<serde_json::Value>,
    #[serde(default)]
    deps_errors: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    incomplete: bool,
}

impl Pkg {
    /// The files compiled into a test binary that links this entry: a
    /// package's own test files only in the test build's variants (a plain
    /// dependency's _test.go files are not linked into anything here).
    fn lists(&self) -> Vec<(&'static str, &Vec<String>)> {
        let all = [
            ("go", &self.go_files),
            ("cgo", &self.cgo_files),
            ("c", &self.c_files),
            ("cxx", &self.cxx_files),
            ("m", &self.m_files),
            ("h", &self.h_files),
            ("f", &self.f_files),
            ("s", &self.s_files),
            ("swig", &self.swig_files),
            ("swigcxx", &self.swig_cxx_files),
            ("syso", &self.syso_files),
            ("embed", &self.embed_files),
            ("test", &self.test_go_files),
            ("xtest", &self.x_test_go_files),
            ("testembed", &self.test_embed_files),
            ("xtestembed", &self.x_test_embed_files),
        ];
        let tests = !self.for_test.is_empty();
        all.into_iter()
            .filter(|(kind, _)| tests || !kind.contains("test"))
            .collect()
    }

    /// Whether its files are hashed by content: anything but a package of
    /// a versioned module in the module cache (read-only, checked against
    /// go.sum), whose path@version is its content.
    fn hashed(&self, modcache: &str) -> bool {
        match &self.module {
            Some(m) => {
                m.main
                    || m.version.is_empty()
                    || m.replace.as_ref().is_some_and(|r| r.version.is_empty())
                    || modcache.is_empty()
                    || !Path::new(&self.dir).starts_with(modcache)
            }
            None => true,
        }
    }

    /// The generated test main (`P.test`): its files live in a temporary
    /// directory and follow from the test packages' own.
    fn is_testmain(&self) -> bool {
        self.import_path.ends_with(".test")
    }
}

/// What `go list` and `go env` say about the packages under test.
struct World {
    /// The packages go test tests, in the order it prints them.
    roots: Vec<String>,
    /// Every listed package (test variants as `Q [P.test]`), by import path.
    pkgs: HashMap<String, Pkg>,
    goenv: BTreeMap<String, String>,
}

/// A package's compiled closure: its key; the key of everything but the
/// contents of its local, non-test .go files; and those files' states.
#[derive(Clone)]
struct Clo {
    full: String,
    structure: String,
    local: BTreeMap<String, String>,
}

/// A package's compiled closure, or why it must run.
type Closure = std::result::Result<Clo, String>;

/// One listed package's part of a `Clo`.
struct Entry {
    full: String,
    structure: String,
    local: Vec<(String, String)>,
}

#[derive(Debug, Default, PartialEq)]
struct Parsed {
    /// Every argument but the package patterns, in order (values joined).
    flags: Vec<String>,
    /// The flags go list needs to load the same packages.
    list: Vec<String>,
    patterns: Vec<String>,
    json: bool,
    count: bool,
}

struct Decision {
    pkg: String,
    /// Why it runs; None: it replays.
    run: Option<String>,
    /// It replays although its compiled code changed: only in functions its
    /// tests never ran.
    narrowed: bool,
}

struct Outcome {
    exit: u8,
    chunks: BTreeMap<String, Chunk>,
}

pub fn cli(a: GotestArgs) -> Result<u8> {
    let t0 = Instant::now();
    let parsed = match parse_args(&a.args) {
        Ok(p) => p,
        Err(why) => {
            eprintln!("quod gotest: {why} is not supported; running go test with no replay");
            return plain(&a.go, &a.args);
        }
    };
    let recording = !a.no_record;
    let root = std::env::current_dir().context("the current directory")?;
    let dir = a.dir.clone().unwrap_or_else(|| root.join(".quod/gotest"));
    let full = a.full || std::env::var("QUOD_GOTEST_FULL").is_ok_and(|v| v == "1");
    if recording {
        check_strace(&a.strace)?;
    }
    let ledger_path = dir.join("ledger.json");
    let mut ledger = read_ledger(&ledger_path);
    let tree = tree(&a.go, &parsed, &root, &dir);
    let reused = if recording || full {
        None
    } else {
        ledger.world.as_ref().and_then(|c| reuse(c, &tree))
    };
    let fast = reused.is_some();
    let world = match reused.map_or_else(|| load(&a.go, &parsed), Ok) {
        Ok(w) if !w.roots.is_empty() => w,
        Ok(_) if !recording => return plain(&a.go, &a.args),
        Ok(_) => bail!("go list found no packages for {:?}", parsed.patterns),
        Err(e) if !recording => {
            eprintln!("quod gotest: {e:#}; running go test with no replay");
            return plain(&a.go, &a.args);
        }
        Err(e) => return Err(e),
    };
    let global = global_key(&world, &parsed, &root);
    let closures = closures(&world, &tree.gofiles);
    let decisions = select(&ledger, &world.roots, &global, &closures, full);
    let select_ms = t0.elapsed().as_millis() as u64;

    let mut audited: BTreeSet<String> = BTreeSet::new();
    if recording {
        let mut replayable: Vec<&str> = decisions
            .iter()
            .filter(|d| d.run.is_none())
            .map(|d| d.pkg.as_str())
            .collect();
        let mut rng = Rng::new();
        while audited.len() < a.audits && !replayable.is_empty() {
            let i = rng.below(replayable.len());
            audited.insert(replayable.swap_remove(i).to_string());
        }
    }
    let run: Vec<String> = decisions
        .iter()
        .filter(|d| d.run.is_some() || audited.contains(&d.pkg))
        .map(|d| d.pkg.clone())
        .collect();
    let replays: BTreeMap<String, String> = decisions
        .iter()
        .filter(|d| d.run.is_none() && !audited.contains(&d.pkg))
        .filter_map(|d| {
            ledger
                .pkgs
                .get(&d.pkg)
                .map(|r| (d.pkg.clone(), r.output.clone()))
        })
        .collect();
    eprintln!(
        "quod gotest: {} packages: {} run, {} replay{}",
        world.roots.len(),
        run.len() - audited.len(),
        replays.len(),
        if audited.is_empty() {
            String::new()
        } else {
            format!(", {} audited", audited.len())
        }
    );

    let coverpkgs = coverpkgs(&world);
    // Coverage changes what go test prints: only with the critical path's
    // report, whose output is what replays.
    let cover = recording
        && a.expect_report.is_some()
        && !coverpkgs.is_empty()
        && std::env::var("QUOD_GOTEST_NO_COVER").is_err();
    let mut cmd = Command::new(&a.go);
    cmd.arg("test").args(&parsed.flags);
    let trace_dir = std::env::temp_dir().join(format!("quod-gotest-{}", std::process::id()));
    if recording {
        if !parsed.count {
            cmd.arg("-count=1");
        }
        let _ = fs::remove_dir_all(&trace_dir);
        fs::create_dir_all(&trace_dir).context("the trace directory")?;
        let wrapper = trace_dir.join("exec.sh");
        fs::write(&wrapper, wrapper_script(&a.strace))?;
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))?;
        cmd.arg(format!("-exec={}", wrapper.display()));
        cmd.env("QUOD_GOTEST_TRACE", &trace_dir);
        if cover {
            cmd.args(["-cover", "-covermode=set"])
                .arg(format!("-coverpkg={}", coverpkgs.join(",")));
        }
    }
    if !recording && run.len() == world.roots.len() {
        // Everything runs: exactly the developer's command.
        cmd.args(&parsed.patterns);
    } else {
        cmd.args(&run);
    }
    let t1 = Instant::now();
    let outcome = run_go(cmd, &run, &world.roots, &replays, parsed.json)?;
    let go_ms = t1.elapsed().as_millis() as u64;

    if let Some(p) = &a.report {
        write_json(
            p,
            &Report {
                args: a.args.clone(),
                ran: outcome.chunks.clone(),
                replayed: replays.keys().cloned().collect(),
            },
        )?;
    }
    let reasons: BTreeMap<&str, &str> = decisions
        .iter()
        .filter_map(|d| d.run.as_deref().map(|w| (d.pkg.as_str(), w)))
        .collect();
    let cached = outcome
        .chunks
        .values()
        .filter(|c| c.output.contains("(cached)"))
        .count();
    let mut summary = serde_json::json!({
        "mode": if recording { "shadow" } else { "critical" },
        "packages": world.roots.len(),
        "ran": run.len() - audited.len(),
        "replayed": replays.len(),
        "audited": audited.len(),
        "ran_pkgs": run.iter().filter(|p| !audited.contains(*p)).collect::<Vec<_>>(),
        "reasons": reasons,
        "go_cached": cached,
        "narrowed": decisions.iter().filter(|d| d.narrowed).count(),
        "fast_world": fast,
        "covered": cover,
        "select_ms": select_ms,
        "go_ms": go_ms,
        "exit": outcome.exit,
    });
    if recording {
        let t2 = Instant::now();
        let expect: Option<Report> = match &a.expect_report {
            Some(p) => {
                let r: Report = serde_json::from_slice(
                    &fs::read(p).with_context(|| format!("reading {}", p.display()))?,
                )
                .with_context(|| format!("parsing {}", p.display()))?;
                if r.args != a.args {
                    bail!(
                        "{} is a report of go test {:?}, not of {:?}",
                        p.display(),
                        r.args,
                        a.args
                    );
                }
                Some(r)
            }
            None => None,
        };
        let mut paths: Vec<&String> = run
            .iter()
            .filter_map(|p| closures.get(p)?.as_ref().ok())
            .flat_map(|c| c.local.keys())
            .collect();
        paths.sort();
        paths.dedup();
        let parsed_shapes = par_map(&paths, |p| fs::read(p).ok().and_then(|b| go_src::shape(&b)));
        let shapes_now: HashMap<String, Option<go_src::Shape>> = paths
            .iter()
            .map(|p| (*p).clone())
            .zip(parsed_shapes)
            .collect();
        let dirs: HashMap<String, String> = world
            .pkgs
            .values()
            .filter(|p| p.for_test.is_empty() && !p.standard)
            .map(|p| (p.import_path.clone(), p.dir.clone()))
            .collect();
        let rec = record(
            &mut ledger,
            &Ctx {
                global: &global,
                closures: &closures,
                expect: expect.as_ref(),
                traces: &traces(&trace_dir),
                world: &world,
                workspace: &fs::canonicalize(&root).unwrap_or_else(|_| root.clone()),
                scratch: &scratch(),
                go: &a.go,
                cover,
                shapes_now: &shapes_now,
                dirs: &dirs,
            },
            &run,
            &audited,
            &outcome,
        );
        ledger.pkgs.retain(|p, _| world.roots.contains(p));
        let used: HashSet<&String> = ledger
            .pkgs
            .values()
            .flat_map(|r| r.local.values())
            .collect();
        let keep: BTreeMap<String, FileShape> = ledger
            .shapes
            .iter()
            .filter(|(k, _)| used.contains(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        ledger.shapes = keep;
        ledger.world = cache_world(&world, &tree);
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let tmp = dir.join("ledger.json.tmp");
        fs::write(&tmp, serde_json::to_vec(&ledger)?)?;
        fs::rename(tmp, ledger_path)?;
        let _ = fs::remove_dir_all(trace_dir);
        if let (Some(s), Some(r)) = (summary.as_object_mut(), rec.as_object()) {
            s.extend(r.clone());
            s.insert("record_ms".into(), (t2.elapsed().as_millis() as u64).into());
        }
    }
    if let Some(p) = &a.summary {
        if let Some(s) = summary.as_object_mut() {
            s.insert("total_ms".into(), (t0.elapsed().as_millis() as u64).into());
        }
        write_json(p, &summary)?;
    }
    Ok(outcome.exit)
}

/// go test with the developer's arguments, nothing replayed.
fn plain(go: &str, args: &[String]) -> Result<u8> {
    let st = Command::new(go)
        .arg("test")
        .args(args)
        .status()
        .with_context(|| format!("running {go} test"))?;
    Ok(exit_code(st))
}

fn exit_code(st: std::process::ExitStatus) -> u8 {
    st.code().map_or(1, |c| c.clamp(0, 255) as u8)
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    fs::write(p, serde_json::to_vec(v)?).with_context(|| format!("writing {}", p.display()))
}

fn parse_args(args: &[String]) -> std::result::Result<Parsed, String> {
    let mut p = Parsed::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        let Some(body) = a.strip_prefix('-') else {
            if a.ends_with(".go") {
                return Err(format!("a file argument ({a})"));
            }
            p.patterns.push(a.clone());
            continue;
        };
        let body = body.strip_prefix('-').unwrap_or(body);
        let (name, value) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (body, None),
        };
        let name = name.strip_prefix("test.").unwrap_or(name);
        if BOOL_FLAGS.contains(&name) {
            if value.is_some_and(|v| v != "true" && v != "false") {
                return Err(format!("the flag {a}"));
            }
            if name == "json" {
                p.json = value != Some("false");
            }
            p.flags.push(a.clone());
            if LIST_FLAGS.contains(&name) {
                p.list.push(a.clone());
            }
        } else if VALUE_FLAGS.contains(&name) {
            let v = match value {
                Some(v) => v.to_string(),
                None => {
                    let Some(v) = args.get(i) else {
                        return Err(format!("the flag {a} with no value"));
                    };
                    i += 1;
                    v.clone()
                }
            };
            match name {
                "shuffle" if v != "off" => return Err(format!("-shuffle={v}")),
                "pgo" if v != "auto" && v != "off" => return Err(format!("-pgo={v}")),
                "count" => p.count = true,
                _ => {}
            }
            let joined = format!("-{name}={v}");
            if LIST_FLAGS.contains(&name) {
                p.list.push(joined.clone());
            }
            p.flags.push(joined);
        } else {
            return Err(format!("the flag {a}"));
        }
    }
    if p.patterns.is_empty() {
        p.patterns.push(".".into());
    }
    Ok(p)
}

fn go_out(go: &str, args: &[String]) -> Result<Vec<u8>> {
    let out = Command::new(go)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {go} {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "{go} {} failed: {}",
            args.first().map_or("", |s| s.as_str()),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

fn load(go: &str, p: &Parsed) -> Result<World> {
    let mut order: Vec<String> = vec!["list".into(), "-e".into()];
    order.extend(p.list.iter().cloned());
    order.extend(p.patterns.iter().cloned());
    let mut deps: Vec<String> = vec![
        "list".into(),
        "-e".into(),
        "-deps".into(),
        "-test".into(),
        format!("-json={LIST_FIELDS}"),
    ];
    deps.extend(p.list.iter().cloned());
    deps.extend(p.patterns.iter().cloned());
    let env: Vec<String> = vec!["env".into(), "-json".into()];
    let (o, d, e) = std::thread::scope(|s| {
        let o = s.spawn(|| go_out(go, &order));
        let d = s.spawn(|| go_out(go, &deps));
        let e = go_out(go, &env);
        (o.join(), d.join(), e)
    });
    let o = o.map_err(|_| anyhow::anyhow!("go list panicked"))??;
    let d = d.map_err(|_| anyhow::anyhow!("go list panicked"))??;
    let e = e?;
    let roots: Vec<String> = String::from_utf8(o)
        .context("go list printed non-UTF-8")?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let mut pkgs = HashMap::new();
    for p in serde_json::Deserializer::from_slice(&d).into_iter::<Pkg>() {
        let p = p.context("parsing go list -json")?;
        pkgs.insert(p.import_path.clone(), p);
    }
    let goenv: BTreeMap<String, String> =
        serde_json::from_slice(&e).context("parsing go env -json")?;
    Ok(World { roots, pkgs, goenv })
}

fn code_hash() -> String {
    let mut h = blake3::Hasher::new();
    h.update(include_str!("gotest_shadow.rs").as_bytes());
    h.update(include_str!("gotest_shadow/go_src.rs").as_bytes());
    h.update(b"\0");
    h.update(env!("CARGO_PKG_VERSION").as_bytes());
    h.finalize().to_hex()[..16].to_string()
}

fn global_key(w: &World, p: &Parsed, root: &Path) -> String {
    let mut h = blake3::Hasher::new();
    let mut put = |s: &str| {
        h.update(s.as_bytes());
        h.update(b"\0");
    };
    put(&format!("ledger v{LEDGER_VERSION}"));
    put(&code_hash());
    for (k, v) in &w.goenv {
        if !UNKEYED_GO_ENV.contains(&k.as_str()) {
            put(k);
            put(v);
        }
    }
    put(&env_all());
    for f in &p.flags {
        put(f);
    }
    put(&root.to_string_lossy());
    put(&std::thread::available_parallelism()
        .map_or(0, |n| n.get())
        .to_string());
    if let Some(work) = w.goenv.get("GOWORK")
        && !work.is_empty()
        && work != "off"
    {
        put(&file_state(Path::new(work)));
    }
    h.finalize().to_hex().to_string()
}

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

/// A per-run or per-step CI variable, Quod's own, or where the build cache is.
fn volatile_env(name: &str) -> bool {
    quod_key::unkeyed(name)
        || STEP_VOLATILE.contains(&name)
        || name.starts_with("QUOD_")
        || name == "GOCACHE"
}

/// Every root's compiled closure: the packages its test binary links
/// (`P.test` and its Deps; a package with no test files: itself and its
/// Deps), each by `entry_key`, and its directory's default.pgo. `known`:
/// file states already computed (`Tree::gofiles`).
fn closures(w: &World, known: &BTreeMap<String, String>) -> HashMap<String, Closure> {
    let modcache = w.goenv.get("GOMODCACHE").cloned().unwrap_or_default();
    let mut sets: Vec<(String, std::result::Result<Vec<String>, String>)> = Vec::new();
    for r in &w.roots {
        let set = match w.pkgs.get(&format!("{r}.test")).or_else(|| w.pkgs.get(r)) {
            None => Err(format!("go list did not list {r}")),
            Some(p) => {
                let mut v = p.deps.clone();
                v.push(p.import_path.clone());
                v.sort();
                v.dedup();
                Ok(v)
            }
        };
        sets.push((r.clone(), set));
    }
    let mut names: BTreeSet<String> = BTreeSet::new();
    for (_, s) in &sets {
        if let Ok(v) = s {
            names.extend(v.iter().cloned());
        }
    }
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    for n in &names {
        let Some(p) = w.pkgs.get(n) else { continue };
        if p.standard || p.is_testmain() || !p.hashed(&modcache) {
            continue;
        }
        for (_, list) in p.lists() {
            for f in list {
                files.insert(Path::new(&p.dir).join(f));
            }
        }
        for m in p
            .module
            .iter()
            .chain(p.module.as_ref().and_then(|m| m.replace.as_deref()))
        {
            if !m.go_mod.is_empty() {
                files.insert(PathBuf::from(&m.go_mod));
            }
        }
    }
    for r in &w.roots {
        if let Some(p) = w.pkgs.get(r) {
            files.insert(Path::new(&p.dir).join("default.pgo"));
        }
    }
    let todo: Vec<PathBuf> = files
        .iter()
        .filter(|f| f.to_str().is_none_or(|s| !known.contains_key(s)))
        .cloned()
        .collect();
    let hashes = par_map(&todo, |f| file_state(f));
    let mut fh: HashMap<PathBuf, String> = todo.into_iter().zip(hashes).collect();
    for f in &files {
        if let Some(s) = f.to_str().and_then(|s| known.get(s)) {
            fh.insert(f.clone(), s.clone());
        }
    }
    let entries: HashMap<&str, std::result::Result<Entry, String>> = names
        .iter()
        .map(|n| (n.as_str(), entry_key(w.pkgs.get(n), n, &modcache, &fh)))
        .collect();
    sets.into_iter()
        .map(|(r, set)| {
            let key = set.and_then(|v| {
                let mut full = blake3::Hasher::new();
                let mut structure = blake3::Hasher::new();
                let mut local = BTreeMap::new();
                for n in &v {
                    let e = entries
                        .get(n.as_str())
                        .ok_or_else(|| format!("go list did not list {n}"))?
                        .as_ref()
                        .map_err(String::clone)?;
                    for (h, k) in [(&mut full, &e.full), (&mut structure, &e.structure)] {
                        h.update(n.as_bytes());
                        h.update(b"=");
                        h.update(k.as_bytes());
                        h.update(b"\0");
                    }
                    local.extend(e.local.iter().cloned());
                }
                if let Some(p) = w.pkgs.get(&r) {
                    let pgo = Path::new(&p.dir).join("default.pgo");
                    let st = fh.get(&pgo).map_or("unhashed", String::as_str);
                    for h in [&mut full, &mut structure] {
                        h.update(b"pgo=");
                        h.update(st.as_bytes());
                    }
                }
                Ok(Clo {
                    full: full.finalize().to_hex().to_string(),
                    structure: structure.finalize().to_hex().to_string(),
                    local,
                })
            });
            (r, key)
        })
        .collect()
}

/// One listed package's part of a closure, or why the packages linking it
/// must run (it has errors, or cgo: the host's C toolchain and headers are
/// not keyed). Its local, non-test .go files are `local`: in `full` by
/// content, in `structure` by name only.
fn entry_key(
    p: Option<&Pkg>,
    name: &str,
    modcache: &str,
    fh: &HashMap<PathBuf, String>,
) -> std::result::Result<Entry, String> {
    let Some(p) = p else {
        return Err(format!("go list did not list {name}"));
    };
    if let Some(e) = &p.error {
        return Err(format!("{name} has errors: {e}"));
    }
    if p.incomplete || p.deps_errors.as_ref().is_some_and(|v| !v.is_empty()) {
        return Err(format!("{name}'s dependencies have errors"));
    }
    let fixed = |tag: &str| Entry {
        full: tag.to_string(),
        structure: tag.to_string(),
        local: Vec::new(),
    };
    if p.standard {
        return Ok(fixed("std"));
    }
    if p.is_testmain() {
        return Ok(fixed("testmain"));
    }
    if !p.cgo_files.is_empty() {
        return Err(format!("{name} uses cgo"));
    }
    let hashed = p.hashed(modcache);
    let mut full = blake3::Hasher::new();
    let mut structure = blake3::Hasher::new();
    let mut local = Vec::new();
    let mut both = |b: &[u8]| {
        full.update(b);
        structure.update(b);
    };
    for s in [p.name.as_str(), p.dir.as_str()] {
        both(s.as_bytes());
        both(b"\0");
    }
    let mut module = p.module.as_ref();
    while let Some(m) = module {
        both(format!("mod {} {} {}\0", m.path, m.version, m.main).as_bytes());
        if hashed && !m.go_mod.is_empty() {
            both(
                fh.get(Path::new(&m.go_mod))
                    .map_or("unhashed", String::as_str)
                    .as_bytes(),
            );
        }
        module = m.replace.as_deref();
    }
    let mut contents: Vec<(bool, String)> = Vec::new();
    for (kind, list) in p.lists() {
        both(kind.as_bytes());
        both(b":");
        for f in list {
            both(f.as_bytes());
            both(b"=");
            if hashed {
                let path = Path::new(&p.dir).join(f);
                let st = fh.get(&path).map_or("unhashed", String::as_str).to_string();
                if kind == "go" {
                    local.push((path.to_string_lossy().into_owned(), st.clone()));
                    contents.push((true, st));
                } else {
                    contents.push((false, st));
                }
            }
            both(b"\0");
        }
    }
    for (narrowable, st) in contents {
        full.update(st.as_bytes());
        full.update(b"\0");
        if !narrowable {
            structure.update(st.as_bytes());
            structure.update(b"\0");
        }
    }
    Ok(Entry {
        full: full.finalize().to_hex().to_string(),
        structure: structure.finalize().to_hex().to_string(),
        local,
    })
}

fn read_ledger(p: &Path) -> Ledger {
    let l: Ledger = fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if l.version == LEDGER_VERSION && l.code == code_hash() {
        l
    } else {
        Ledger {
            version: LEDGER_VERSION,
            code: code_hash(),
            ..Ledger::default()
        }
    }
}

fn select(
    ledger: &Ledger,
    roots: &[String],
    global: &str,
    closures: &HashMap<String, Closure>,
    full: bool,
) -> Vec<Decision> {
    let mut keys: Vec<String> = Vec::new();
    let mut changed: Vec<String> = Vec::new();
    if !full {
        for r in roots {
            let (Some(rec), Some(Ok(c))) = (ledger.pkgs.get(r), closures.get(r)) else {
                continue;
            };
            if rec.quarantined.is_some() || rec.global != global {
                continue;
            }
            if c.full == rec.closure {
                keys.extend(rec.inputs.keys().cloned());
            } else if rec.covered && c.structure == rec.structure {
                keys.extend(rec.inputs.keys().cloned());
                changed.extend(
                    c.local
                        .iter()
                        .filter(|(p, s)| rec.local.get(*p) != Some(*s))
                        .map(|(p, _)| p.clone()),
                );
            }
        }
    }
    keys.sort();
    keys.dedup();
    changed.sort();
    changed.dedup();
    let states = par_map(&keys, |k| state(k));
    let now: HashMap<&str, &str> = keys
        .iter()
        .map(String::as_str)
        .zip(states.iter().map(String::as_str))
        .collect();
    let parsed = par_map(&changed, |p| {
        fs::read(p)
            .ok()
            .and_then(|b| go_src::shape(&b))
            .map(|s| FileShape::from(&s))
    });
    let shapes: HashMap<&str, Option<FileShape>> =
        changed.iter().map(String::as_str).zip(parsed).collect();
    roots
        .iter()
        .map(|r| {
            let rec = ledger.pkgs.get(r);
            let run = if full {
                Some("a full run".into())
            } else {
                why_run(rec, global, closures.get(r), &now, &ledger.shapes, &shapes)
            };
            let narrowed = run.is_none()
                && matches!((rec, closures.get(r)), (Some(rec), Some(Ok(c))) if c.full != rec.closure);
            Decision {
                pkg: r.clone(),
                run,
                narrowed,
            }
        })
        .collect()
}

fn why_run(
    rec: Option<&Rec>,
    global: &str,
    closure: Option<&Closure>,
    now: &HashMap<&str, &str>,
    old: &BTreeMap<String, FileShape>,
    shapes: &HashMap<&str, Option<FileShape>>,
) -> Option<String> {
    let clo = match closure {
        Some(Ok(c)) => c,
        Some(Err(e)) => return Some(e.clone()),
        None => return Some("not listed".into()),
    };
    let Some(rec) = rec else {
        return Some("not recorded".into());
    };
    if let Some(q) = &rec.quarantined {
        return Some(format!("quarantined: {q}"));
    }
    if rec.global != global {
        return Some("the toolchain, environment or flags changed".into());
    }
    if rec.closure != clo.full
        && let Some(why) = narrow(rec, clo, old, shapes)
    {
        return Some(why);
    }
    rec.inputs
        .iter()
        .find(|(k, s)| now.get(k.as_str()).copied() != Some(s.as_str()))
        .map(|(k, _)| format!("{k} changed"))
}

/// Why a package whose compiled code changed must still run, or None: every
/// change is inside the body of a function its tests ran none of, and
/// every function they ran is byte-for-byte where it was. Anything else
/// (the skeleton: declarations, types, signatures, imports, directives; a
/// file the lexer cannot split; no coverage) runs it.
fn narrow(
    rec: &Rec,
    clo: &Clo,
    old: &BTreeMap<String, FileShape>,
    shapes: &HashMap<&str, Option<FileShape>>,
) -> Option<String> {
    if !rec.covered || rec.structure != clo.structure || rec.local.len() != clo.local.len() {
        return Some("its compiled code changed".into());
    }
    for (path, st) in &clo.local {
        let Some(st0) = rec.local.get(path) else {
            return Some("its compiled code changed".into());
        };
        if st0 == st {
            continue;
        }
        let (Some(o), Some(Some(n))) = (old.get(st0), shapes.get(path.as_str())) else {
            return Some(format!("{path} changed"));
        };
        if o.skeleton != n.skeleton {
            return Some(format!("{path} changed outside function bodies"));
        }
        let cold = rec.cold.get(path);
        for (id, b) in &o.funcs {
            if n.funcs.get(id) != Some(b) && !cold.is_some_and(|c| c.contains(id)) {
                return Some(format!(
                    "{path}: a function its tests ran changed or moved ({id})"
                ));
            }
        }
    }
    None
}

/// What one input key's path is now: `f:` a file's content (a directory's
/// listing), `d:` a directory's listing, `s:` what stat and lstat say
/// (type, permissions, a file's size, a link's target), `l:` a link's
/// target.
fn state(key: &str) -> String {
    let Some((kind, path)) = key.split_once(':') else {
        return "unknown".into();
    };
    let p = Path::new(path);
    match kind {
        "f" => file_state(p),
        "d" => match fs::metadata(p) {
            Ok(m) if m.is_dir() => listing(p),
            Ok(_) => format!("notdir:{}", file_state(p)),
            Err(e) => err_state(&e),
        },
        "s" => stat_state(p),
        "l" => fs::read_link(p).map_or_else(|e| err_state(&e), |t| format!("->{}", t.display())),
        _ => "unknown".into(),
    }
}

fn err_state(e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        "absent".into()
    } else {
        format!("err:{}", e.raw_os_error().unwrap_or(-1))
    }
}

fn file_state(p: &Path) -> String {
    match fs::metadata(p) {
        Err(e) => err_state(&e),
        Ok(m) if m.is_dir() => format!("dir:{}", listing(p)),
        Ok(m) if !m.is_file() => format!("special:{:o}", m.mode()),
        Ok(m) => match fs::read(p) {
            Ok(b) => format!(
                "{}:{:o}",
                &blake3::hash(&b).to_hex()[..32],
                m.mode() & 0o777
            ),
            Err(e) => err_state(&e),
        },
    }
}

fn listing(p: &Path) -> String {
    match fs::read_dir(p) {
        Err(e) => err_state(&e),
        Ok(rd) => {
            let mut v: Vec<(Vec<u8>, char)> = rd
                .filter_map(Result::ok)
                .map(|e| {
                    let t = e.file_type().map_or('?', |t| {
                        if t.is_dir() {
                            'd'
                        } else if t.is_symlink() {
                            'l'
                        } else if t.is_file() {
                            'f'
                        } else {
                            'o'
                        }
                    });
                    (e.file_name().as_bytes().to_vec(), t)
                })
                .collect();
            v.sort();
            let mut h = blake3::Hasher::new();
            for (n, t) in v {
                h.update(&n);
                h.update(&[b'/', t as u8, 0]);
            }
            h.finalize().to_hex()[..32].to_string()
        }
    }
}

fn meta_state(m: std::io::Result<fs::Metadata>) -> String {
    match m {
        Err(e) => err_state(&e),
        Ok(m) => {
            let perm = m.mode() & 0o7777;
            if m.is_dir() {
                format!("d{perm:o}")
            } else if m.is_file() {
                format!("f{perm:o}:{}", m.len())
            } else {
                format!("o{:o}", m.mode())
            }
        }
    }
}

fn stat_state(p: &Path) -> String {
    let link = match fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => format!(
            "->{}",
            fs::read_link(p).map_or_else(|e| err_state(&e), |t| t.display().to_string())
        ),
        r => meta_state(r),
    };
    format!("{}|{link}", meta_state(fs::metadata(p)))
}

/// Runs go test (unless nothing must run) and prints its stdout with the
/// replayed packages' recorded output merged in go test's package order.
fn run_go(
    mut cmd: Command,
    run: &[String],
    roots: &[String],
    replays: &BTreeMap<String, String>,
    json: bool,
) -> Result<Outcome> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut m = Merger::new(run, roots, replays, json);
    if run.is_empty() {
        let chunks = m.finish(&mut out)?;
        out.flush()?;
        return Ok(Outcome { exit: 0, chunks });
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .spawn()
        .context("running go test")?;
    let reader = BufReader::new(child.stdout.take().context("go test's stdout")?);
    for line in reader.split(b'\n') {
        let mut line = line.context("reading go test's stdout")?;
        line.push(b'\n');
        m.line(&line, &mut out)?;
    }
    let chunks = m.finish(&mut out)?;
    out.flush()?;
    let st = child.wait().context("waiting for go test")?;
    Ok(Outcome {
        exit: exit_code(st),
        chunks,
    })
}

/// Splits go test's stdout into the packages that ran and prints it with
/// the replayed packages' output before the first line of the next package
/// that ran after them (go test prints packages one at a time, in order).
struct Merger<'a> {
    run: &'a [String],
    runpos: HashMap<&'a str, usize>,
    index: HashMap<&'a str, usize>,
    /// (root index, recorded output), sorted; `next` are printed.
    pending: Vec<(usize, &'a str)>,
    next: usize,
    /// Text mode: the package whose output is being printed.
    cur: usize,
    json: bool,
    bufs: Vec<Vec<u8>>,
    status: Vec<String>,
    tests: Vec<BTreeMap<String, String>>,
}

impl<'a> Merger<'a> {
    fn new(
        run: &'a [String],
        roots: &'a [String],
        replays: &'a BTreeMap<String, String>,
        json: bool,
    ) -> Merger<'a> {
        let index: HashMap<&str, usize> = roots
            .iter()
            .enumerate()
            .map(|(i, r)| (r.as_str(), i))
            .collect();
        let mut pending: Vec<(usize, &str)> = replays
            .iter()
            .filter_map(|(p, o)| index.get(p.as_str()).map(|&i| (i, o.as_str())))
            .collect();
        pending.sort();
        Merger {
            run,
            runpos: run
                .iter()
                .enumerate()
                .map(|(i, r)| (r.as_str(), i))
                .collect(),
            index,
            pending,
            next: 0,
            cur: 0,
            json,
            bufs: vec![Vec::new(); run.len()],
            status: vec![String::new(); run.len()],
            tests: vec![BTreeMap::new(); run.len()],
        }
    }

    fn flush(&mut self, upto: usize, out: &mut impl Write) -> std::io::Result<()> {
        while let Some(&(i, o)) = self.pending.get(self.next)
            && i < upto
        {
            out.write_all(o.as_bytes())?;
            self.next += 1;
        }
        Ok(())
    }

    fn line(&mut self, line: &[u8], out: &mut impl Write) -> std::io::Result<()> {
        let text = std::str::from_utf8(line).ok();
        let owner = if self.json {
            text.and_then(json_root)
                .and_then(|p| self.runpos.get(p.as_str()).copied())
        } else if self.cur < self.run.len() {
            Some(self.cur)
        } else {
            None
        };
        let Some(k) = owner else {
            if !self.json {
                // After the last package: go test's own tail.
                self.flush(usize::MAX, out)?;
            }
            return out.write_all(line);
        };
        let at = self
            .index
            .get(self.run[k].as_str())
            .copied()
            .unwrap_or(usize::MAX);
        self.flush(at, out)?;
        out.write_all(line)?;
        self.bufs[k].extend_from_slice(line);
        if let Some(t) = text {
            if self.json {
                json_event(t, &mut self.status[k], &mut self.tests[k]);
            } else if let Some(s) = terminator(t, &self.run[k]) {
                self.status[k] = s.to_string();
                self.cur = k + 1;
            } else if let Some((name, st)) = verbose_result(t) {
                self.tests[k].insert(name, st);
            }
        }
        Ok(())
    }

    fn finish(mut self, out: &mut impl Write) -> std::io::Result<BTreeMap<String, Chunk>> {
        self.flush(usize::MAX, out)?;
        let mut chunks = BTreeMap::new();
        for (k, pkg) in self.run.iter().enumerate() {
            let buf = std::mem::take(&mut self.bufs[k]);
            let (output, binary) = match String::from_utf8(buf) {
                Ok(s) => (s, false),
                Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
            };
            chunks.insert(
                pkg.clone(),
                Chunk {
                    output,
                    status: std::mem::take(&mut self.status[k]),
                    tests: std::mem::take(&mut self.tests[k]),
                    binary,
                },
            );
        }
        Ok(chunks)
    }
}

/// The status of go test's last line for `pkg` (text mode), if `line` is it.
fn terminator(line: &str, pkg: &str) -> Option<&'static str> {
    for (prefix, status) in [("ok  \t", "ok"), ("FAIL\t", "fail"), ("?   \t", "nofiles")] {
        if let Some(rest) = line.strip_prefix(prefix)
            && let Some(after) = rest.strip_prefix(pkg)
            && (after.is_empty() || after.starts_with(['\t', ' ', '\n']))
        {
            return Some(status);
        }
    }
    None
}

/// A `-v` result line: `--- PASS: TestX (0.00s)` (indented for subtests).
fn verbose_result(line: &str) -> Option<(String, String)> {
    let rest = line.trim_start().strip_prefix("--- ")?;
    let (word, rest) = rest.split_once(": ")?;
    let st = match word {
        "PASS" => "pass",
        "FAIL" => "fail",
        "SKIP" => "skip",
        _ => return None,
    };
    let name = rest.split(" (").next()?.trim();
    Some((name.to_string(), st.to_string()))
}

/// The package under test a `-json` line is about: its Package, or the
/// test binary an ImportPath (`Q [P.test]`) is built for.
fn json_root(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line.trim_end()).ok()?;
    if let Some(p) = v.get("Package").and_then(|p| p.as_str()) {
        return Some(p.to_string());
    }
    let ip = v.get("ImportPath")?.as_str()?;
    Some(match ip.split_once(" [") {
        Some((_, rest)) => {
            let rest = rest.strip_suffix(']').unwrap_or(rest);
            rest.strip_suffix(".test").unwrap_or(rest).to_string()
        }
        None => ip.to_string(),
    })
}

fn json_event(line: &str, status: &mut String, tests: &mut BTreeMap<String, String>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim_end()) else {
        return;
    };
    let Some(action) = v.get("Action").and_then(|a| a.as_str()) else {
        return;
    };
    if !matches!(action, "pass" | "fail" | "skip") {
        return;
    }
    match v.get("Test").and_then(|t| t.as_str()) {
        Some(t) => {
            tests.insert(t.to_string(), action.to_string());
        }
        None if v.get("Package").is_some() => {
            *status = match action {
                "pass" => "ok",
                "fail" => "fail",
                _ => "nofiles",
            }
            .to_string();
        }
        None => {}
    }
}

/// Whether a run's results are a record's: the same status (a package
/// with no test files may say `ok` under -cover), and the same result per
/// test; test names compared as given, or else as `go_src::norm_name`
/// normalizes them when that maps both sides one to one.
fn same(c: &Chunk, status: &str, tests: &BTreeMap<String, String>) -> bool {
    let st = c.status == status
        || (status == "nofiles" && c.status == "ok" && c.tests.is_empty() && tests.is_empty());
    st && (c.tests == *tests || norm_tests(&c.tests).is_some_and(|a| norm_tests(tests) == Some(a)))
}

fn norm_tests(t: &BTreeMap<String, String>) -> Option<BTreeMap<String, String>> {
    let m: BTreeMap<String, String> = t
        .iter()
        .map(|(k, v)| (go_src::norm_name(k), v.clone()))
        .collect();
    (m.len() == t.len()).then_some(m)
}

// ---- the shadow's recording ----

fn check_strace(strace: &str) -> Result<()> {
    let out = Command::new(strace)
        .args(["-f", "-qq", "--seccomp-bpf", "-y", "-o", "/dev/null", "-e"])
        .arg(format!("trace={TRACE_SET}"))
        .args(["--", "true"])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("the shadow records with strace: running {strace}"))?;
    if !out.status.success() {
        bail!(
            "the shadow records with strace, which cannot trace here: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// The -exec wrapper: the test binary under strace, its trace and its
/// working directory (the package's) named by the wrapper's pid.
fn wrapper_script(strace: &str) -> String {
    format!(
        "#!/bin/sh\n\
         # quod gotest: a test binary under strace, logged by this script's pid;\n\
         # its coverage (-cover) copied next to its trace.\n\
         d=\"$QUOD_GOTEST_TRACE\"\n\
         pwd -P >\"$d/$$.dir\" || exit 97\n\
         cov=\n\
         for a in \"$@\"; do\n\
         \x20 case \"$a\" in -test.gocoverdir=*) cov=${{a#-test.gocoverdir=}} ;; esac\n\
         done\n\
         {} -f -qq --seccomp-bpf -s 4096 -y -e signal=none -e trace={TRACE_SET} -o \"$d/$$.trace\" -- \"$@\"\n\
         rc=$?\n\
         if [ -n \"$cov\" ] && [ -d \"$cov\" ]; then mkdir -p \"$d/$$.cov\" && cp -R \"$cov/.\" \"$d/$$.cov/\"; fi\n\
         exit $rc\n",
        sh_quote(strace)
    )
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The local, non-main, non-cgo packages a test binary can link: what the
/// shadow's coverage instruments.
fn coverpkgs(w: &World) -> Vec<String> {
    let modcache = w.goenv.get("GOMODCACHE").cloned().unwrap_or_default();
    let mut v: Vec<String> = w
        .pkgs
        .values()
        .filter(|p| {
            p.for_test.is_empty()
                && !p.standard
                && !p.is_testmain()
                && p.name != "main"
                && p.cgo_files.is_empty()
                && !p.go_files.is_empty()
                && p.hashed(&modcache)
        })
        .map(|p| p.import_path.clone())
        .collect();
    v.sort();
    v
}

/// The go command the PATH finds, by path and file identity.
fn go_ident(go: &str) -> String {
    let p = if go.contains('/') {
        Some(PathBuf::from(go))
    } else {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|d| d.join(go))
                .find(|p| p.is_file())
        })
    };
    match p.and_then(|p| fs::metadata(&p).ok().map(|m| (p, m))) {
        Some((p, m)) => format!(
            "{}:{}:{}:{}:{}.{}",
            p.display(),
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec()
        ),
        None => "unknown".into(),
    }
}

/// go's own settings file (`go env -w`), by content.
fn goenv_file() -> String {
    let p = match std::env::var_os("GOENV") {
        Some(v) => PathBuf::from(v),
        None => match std::env::var_os("XDG_CONFIG_HOME") {
            Some(x) => PathBuf::from(x).join("go/env"),
            None => std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".config/go/env"))
                .unwrap_or_default(),
        },
    };
    file_state(&p)
}

/// What go list's answer depends on now (`Tree`). `skip`: the ledger's
/// directory, when it is under the root.
fn tree(go: &str, p: &Parsed, root: &Path, skip: &Path) -> Tree {
    let mut env = blake3::Hasher::new();
    for s in [
        code_hash(),
        go_ident(go),
        env_all(),
        goenv_file(),
        root.display().to_string(),
        p.list.join("\0"),
        p.patterns.join("\0"),
    ] {
        env.update(s.as_bytes());
        env.update(b"\0");
    }
    let mut names = blake3::Hasher::new();
    let mut meta_files: Vec<PathBuf> = Vec::new();
    let mut gofiles: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else {
            names.update(b"unreadable:");
            names.update(d.as_os_str().as_bytes());
            continue;
        };
        let mut ents: Vec<fs::DirEntry> = rd.filter_map(Result::ok).collect();
        ents.sort_by_key(fs::DirEntry::file_name);
        for e in ents {
            let p = e.path();
            let Ok(t) = e.file_type() else {
                names.update(b"untyped:");
                names.update(p.as_os_str().as_bytes());
                continue;
            };
            let n = e.file_name();
            if t.is_dir() && (n == ".git" || n == ".quod" || p == skip) {
                continue;
            }
            names.update(p.as_os_str().as_bytes());
            if t.is_dir() {
                names.update(b"/d\0");
                stack.push(p);
                continue;
            }
            if t.is_symlink() {
                names.update(b"/l:");
                if let Ok(target) = fs::read_link(&p) {
                    names.update(target.as_os_str().as_bytes());
                }
            }
            names.update(b"\0");
            let b = n.as_bytes();
            if b.ends_with(b".go") {
                gofiles.push(p);
            } else if matches!(
                b,
                b"go.mod" | b"go.sum" | b"go.work" | b"go.work.sum" | b"modules.txt"
            ) {
                meta_files.push(p);
            }
        }
    }
    let go_states = par_map(&gofiles, |f| file_state(f));
    let meta_states = par_map(&meta_files, |f| file_state(f));
    let mut meta = blake3::Hasher::new();
    for (f, st) in meta_files.iter().zip(&meta_states) {
        meta.update(f.as_os_str().as_bytes());
        meta.update(b"=");
        meta.update(st.as_bytes());
        meta.update(b"\0");
    }
    Tree {
        env: env.finalize().to_hex().to_string(),
        names: names.finalize().to_hex().to_string(),
        meta: meta.finalize().to_hex().to_string(),
        gofiles: gofiles
            .iter()
            .map(|f| f.to_string_lossy().into_owned())
            .zip(go_states)
            .collect(),
    }
}

/// The shadow's listing, if nothing it depended on changed: the same
/// toolchain, environment, paths and module files, and every .go file that
/// changed still has the header it had (build constraints, package,
/// imports, embeds).
fn reuse(c: &Cached, t: &Tree) -> Option<World> {
    if c.env != t.env || c.names != t.names || c.meta != t.meta || c.files.len() != t.gofiles.len()
    {
        return None;
    }
    let mut changed: Vec<(&String, &Option<String>)> = Vec::new();
    for (p, st) in &t.gofiles {
        let (st0, h0) = c.files.get(p)?;
        if st0 != st {
            changed.push((p, h0));
        }
    }
    let ok = par_map(&changed, |(p, h0)| {
        h0.is_some() && fs::read(p).ok().and_then(|b| go_src::header(&b)) == **h0
    });
    if !ok.into_iter().all(|b| b) {
        return None;
    }
    Some(World {
        roots: c.roots.clone(),
        pkgs: c
            .pkgs
            .iter()
            .map(|p| (p.import_path.clone(), p.clone()))
            .collect(),
        goenv: c.goenv.clone(),
    })
}

/// What the shadow stores for the next critical paths' `reuse`.
fn cache_world(w: &World, t: &Tree) -> Option<Cached> {
    let paths: Vec<&String> = t.gofiles.keys().collect();
    let heads = par_map(&paths, |p| {
        fs::read(p).ok().and_then(|b| go_src::header(&b))
    });
    let files = paths
        .iter()
        .zip(heads)
        .map(|(p, h)| ((*p).clone(), (t.gofiles[*p].clone(), h)))
        .collect();
    Some(Cached {
        env: t.env.clone(),
        names: t.names.clone(),
        meta: t.meta.clone(),
        files,
        roots: w.roots.clone(),
        pkgs: w.pkgs.values().cloned().collect(),
        goenv: w.goenv.clone(),
    })
}

/// The traces the wrapper left, by the (canonical) directory they ran in.
fn traces(trace_dir: &Path) -> HashMap<PathBuf, Vec<PathBuf>> {
    let mut out: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    let Ok(rd) = fs::read_dir(trace_dir) else {
        return out;
    };
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "dir")
            && let Ok(d) = fs::read_to_string(&p)
        {
            let d = PathBuf::from(d.trim_end_matches('\n'));
            let d = fs::canonicalize(&d).unwrap_or(d);
            out.entry(d).or_default().push(p.with_extension("trace"));
        }
    }
    out
}

/// Where a test may make scratch files: what it reads there that it did
/// not create is an input like any other.
fn scratch() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = ["/tmp", "/var/tmp", "/dev/shm"]
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if let Ok(t) = std::env::var("TMPDIR")
        && t.starts_with('/')
    {
        v.push(normal(Path::new(&t)));
    }
    let canon: Vec<PathBuf> = v.iter().filter_map(|s| fs::canonicalize(s).ok()).collect();
    v.extend(canon);
    v.sort();
    v.dedup();
    v
}

struct Ctx<'a> {
    global: &'a str,
    closures: &'a HashMap<String, Closure>,
    expect: Option<&'a Report>,
    traces: &'a HashMap<PathBuf, Vec<PathBuf>>,
    world: &'a World,
    workspace: &'a Path,
    scratch: &'a [PathBuf],
    go: &'a str,
    /// The shadow ran with -cover: each binary's coverage is next to its trace.
    cover: bool,
    /// The local files of the packages that ran, split (`go_src::shape`).
    shapes_now: &'a HashMap<String, Option<go_src::Shape>>,
    /// Package directories by import path (coverage names files by package).
    dirs: &'a HashMap<String, String>,
}

/// Coverage blocks by file: (line, column) of each block's start, and
/// whether it ran.
type Blocks = HashMap<String, Vec<(u32, u32, bool)>>;

enum Fail {
    /// It failed: it always runs, nothing to record.
    Failed,
    /// Its trace shows something Quod cannot key: it runs until a later
    /// shadow records it.
    Unrecordable(String),
    /// Recording changed its results: quarantined.
    Changed(String),
}

fn record(
    ledger: &mut Ledger,
    cx: &Ctx,
    run: &[String],
    audited: &BTreeSet<String>,
    outcome: &Outcome,
) -> serde_json::Value {
    let mut recorded = Vec::new();
    let mut unrecordable = BTreeMap::new();
    let mut changed = BTreeMap::new();
    let mut mismatches = BTreeMap::new();
    let mut audits_ok = 0;
    for pkg in run {
        let got = outcome.chunks.get(pkg);
        if audited.contains(pkg) {
            let Some(rec) = ledger.pkgs.get_mut(pkg) else {
                continue;
            };
            match got {
                Some(g) if same(g, &rec.status, &rec.tests) => audits_ok += 1,
                _ => {
                    let why = format!(
                        "an audit ran it: recorded {} ({} tests), ran {}",
                        rec.status,
                        rec.tests.len(),
                        got.map_or_else(
                            || "nothing".to_string(),
                            |g| format!("{} ({} tests)", g.status, g.tests.len())
                        )
                    );
                    rec.quarantined = Some(why.clone());
                    mismatches.insert(pkg.clone(), why);
                }
            }
            continue;
        }
        if ledger
            .pkgs
            .get(pkg)
            .is_some_and(|r| r.quarantined.is_some())
        {
            continue;
        }
        ledger.pkgs.remove(pkg);
        match cx.record(pkg, got) {
            Ok(rec) => {
                if rec.covered {
                    for (path, st) in &rec.local {
                        if let Some(Some(sh)) = cx.shapes_now.get(path) {
                            ledger.shapes.insert(st.clone(), FileShape::from(sh));
                        }
                    }
                }
                ledger.pkgs.insert(pkg.clone(), rec);
                recorded.push(pkg.clone());
            }
            Err(Fail::Failed) => {}
            Err(Fail::Unrecordable(why)) => {
                unrecordable.insert(pkg.clone(), why);
            }
            Err(Fail::Changed(why)) => {
                let why = format!("recording changed behaviour: {why}");
                ledger.pkgs.insert(
                    pkg.clone(),
                    Rec {
                        quarantined: Some(why.clone()),
                        ..Rec::default()
                    },
                );
                changed.insert(pkg.clone(), why);
            }
        }
    }
    for (p, why) in &changed {
        eprintln!("quod gotest: {p}: {why}; quarantined");
    }
    for (p, why) in &mismatches {
        eprintln!("quod gotest: {p}: {why}; quarantined");
    }
    eprintln!(
        "quod gotest: recorded {}, unrecordable {}, behaviour changed {}, audits {} ok / {} mismatched",
        recorded.len(),
        unrecordable.len(),
        changed.len(),
        audits_ok,
        mismatches.len()
    );
    serde_json::json!({
        "recorded": recorded.len(),
        "unrecordable": unrecordable,
        "behaviour_changed": changed,
        "audit_mismatches": mismatches,
        "audits_ok": audits_ok,
    })
}

impl Ctx<'_> {
    fn record(&self, pkg: &str, got: Option<&Chunk>) -> std::result::Result<Rec, Fail> {
        let got = got.ok_or_else(|| Fail::Unrecordable("go test printed nothing for it".into()))?;
        if got.status != "ok" && got.status != "nofiles" {
            return Err(Fail::Failed);
        }
        if got.binary {
            return Err(Fail::Unrecordable("its output is not UTF-8".into()));
        }
        let want = match self.expect {
            None => got,
            Some(r) => r
                .ran
                .get(pkg)
                .ok_or_else(|| Fail::Unrecordable("the critical path did not run it".into()))?,
        };
        if !same(got, &want.status, &want.tests) {
            return Err(Fail::Changed(format!(
                "the critical path reported {} ({} tests), the recorded run {} ({} tests)",
                want.status,
                want.tests.len(),
                got.status,
                got.tests.len()
            )));
        }
        let closure = match self.closures.get(pkg) {
            Some(Ok(c)) => c.clone(),
            Some(Err(e)) => return Err(Fail::Unrecordable(e.clone())),
            None => return Err(Fail::Unrecordable("not listed".into())),
        };
        let mut keys: BTreeSet<String> = BTreeSet::new();
        let mut covered = false;
        let mut cold: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        if got.status == "ok" {
            let p = self
                .world
                .pkgs
                .get(pkg)
                .ok_or_else(|| Fail::Unrecordable("not listed".into()))?;
            let d = PathBuf::from(&p.dir);
            let d = fs::canonicalize(&d).unwrap_or(d);
            let files = self
                .traces
                .get(&d)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| Fail::Unrecordable("no trace of its test binary".into()))?;
            for f in files {
                if fs::metadata(f).is_ok_and(|m| m.len() > 1 << 30) {
                    return Err(Fail::Unrecordable("its trace is over 1 GiB".into()));
                }
                let text = fs::read_to_string(f)
                    .map_err(|e| Fail::Unrecordable(format!("its trace: {e}")))?;
                let calls = parse_trace(&text).map_err(Fail::Unrecordable)?;
                keys.extend(
                    observe(&calls, &d, self.workspace, self.scratch)
                        .map_err(Fail::Unrecordable)?,
                );
            }
            if self.cover
                && let Some(blocks) = self.coverage(files)
            {
                covered = true;
                for path in closure.local.keys() {
                    let Some(Some(sh)) = self.shapes_now.get(path) else {
                        covered = false;
                        break;
                    };
                    let mut has = vec![false; sh.funcs.len()];
                    let mut hit = vec![false; sh.funcs.len()];
                    for &(l, c, h) in blocks.get(path).into_iter().flatten() {
                        if let Some(i) = go_src::owner(&sh.funcs, l, c) {
                            has[i] = true;
                            hit[i] |= h;
                        }
                    }
                    let set: BTreeSet<String> = sh
                        .funcs
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| has[*i] && !hit[*i])
                        .map(|(_, f)| f.id.clone())
                        .collect();
                    if !set.is_empty() {
                        cold.insert(path.clone(), set);
                    }
                }
            }
        }
        let keys: Vec<String> = keys.into_iter().collect();
        let states = par_map(&keys, |k| state(k));
        Ok(Rec {
            global: self.global.to_string(),
            closure: closure.full,
            inputs: keys.into_iter().zip(states).collect(),
            output: want.output.clone(),
            status: want.status.clone(),
            tests: want.tests.clone(),
            quarantined: None,
            structure: closure.structure,
            local: closure.local,
            cold: if covered { cold } else { BTreeMap::new() },
            covered,
        })
    }

    /// The coverage blocks of a package's test binaries, by file: (line,
    /// column) of each block's start, and whether it ran. None: a binary
    /// left no coverage, or `go tool covdata` could not read it.
    fn coverage(&self, traces: &[PathBuf]) -> Option<Blocks> {
        let mut out: Blocks = HashMap::new();
        for t in traces {
            let d = t.with_extension("cov");
            if !d.is_dir() {
                return None;
            }
            let txt = t.with_extension("cov.txt");
            let ok = Command::new(self.go)
                .args(["tool", "covdata", "textfmt"])
                .arg(format!("-i={}", d.display()))
                .arg(format!("-o={}", txt.display()))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()?
                .success();
            if !ok {
                return None;
            }
            let text = fs::read_to_string(&txt).ok()?;
            for line in text.lines() {
                if line.starts_with("mode:") || line.is_empty() {
                    continue;
                }
                let mut it = line.rsplitn(3, ' ');
                let (count, _, loc) = (it.next()?, it.next()?, it.next()?);
                let (name, range) = loc.rsplit_once(':')?;
                let (start, _) = range.split_once(',')?;
                let (l, c) = start.split_once('.')?;
                let (ip, file) = name.rsplit_once('/')?;
                let Some(dir) = self.dirs.get(ip) else {
                    continue;
                };
                out.entry(format!("{dir}/{file}")).or_default().push((
                    l.parse().ok()?,
                    c.parse().ok()?,
                    count != "0",
                ));
            }
        }
        Some(out)
    }
}

// ---- strace ----

#[derive(Debug, Clone, PartialEq)]
struct Call {
    pid: u32,
    name: String,
    args: Vec<String>,
    ret: String,
}

impl Call {
    fn ok(&self) -> bool {
        !self.ret.is_empty() && !self.ret.starts_with('-') && !self.ret.starts_with('?')
    }

    fn arg(&self, i: usize) -> &str {
        self.args.get(i).map_or("", String::as_str)
    }
}

/// strace -f -o output as calls: an interrupted call's two lines (`<unfinished
/// ...>`, `<... name resumed>`) joined; a call a process never returned from
/// keeps `?` as its result.
fn parse_trace(text: &str) -> std::result::Result<Vec<Call>, String> {
    let mut pending: HashMap<u32, String> = HashMap::new();
    let mut calls = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let Some((pid, rest)) = line.split_once(' ') else {
            return Err(format!("an unparsed trace line: {line}"));
        };
        let Ok(pid) = pid.parse::<u32>() else {
            return Err(format!("an unparsed trace line: {line}"));
        };
        let rest = rest.trim_start();
        if rest.starts_with("+++") || rest.starts_with("---") {
            continue;
        }
        if rest.ends_with("<detached ...>") {
            // The process left in the middle of a call (exit_group from
            // another thread): what it had started stays as `?`.
            continue;
        }
        let full = if let Some(head) = rest.strip_suffix("<unfinished ...>") {
            pending.insert(pid, head.trim_end().to_string());
            continue;
        } else if let Some(r) = rest.strip_prefix("<... ") {
            let Some((_, tail)) = r.split_once(" resumed>") else {
                return Err(format!("an unparsed trace line: {line}"));
            };
            let Some(head) = pending.remove(&pid) else {
                return Err(format!("a resumed call that never started: {line}"));
            };
            format!("{head}{tail}")
        } else {
            rest.to_string()
        };
        calls
            .push(parse_call(pid, &full).ok_or_else(|| format!("an unparsed trace line: {line}"))?);
    }
    let mut left: Vec<(u32, String)> = pending.into_iter().collect();
    left.sort();
    for (pid, head) in left {
        let s = format!("{head}) = ?");
        calls.push(parse_call(pid, &s).ok_or_else(|| format!("an unparsed trace line: {head}"))?);
    }
    Ok(calls)
}

/// Offsets of `s`'s top-level commas, and of its first unmatched `)`.
/// Strings and strace's fd annotations (`3</path>`, `AT_FDCWD</path>`) are
/// skipped whole.
fn scan(s: &str) -> (Vec<usize>, Option<usize>) {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut commas = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'<' if i > 0 && (b[i - 1].is_ascii_digit() || b[i - 1] == b'D') => {
                while i < b.len() && b[i] != b'>' {
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' if depth == 0 => return (commas, Some(i)),
            b')' | b']' | b'}' => depth = (depth - 1).max(0),
            b',' if depth == 0 => commas.push(i),
            _ => {}
        }
        i += 1;
    }
    (commas, None)
}

fn parse_call(pid: u32, s: &str) -> Option<Call> {
    let open = s.find('(')?;
    let name = &s[..open];
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    let body = &s[open + 1..];
    let (commas, close) = scan(body);
    let close = close?;
    let mut args = Vec::new();
    let mut start = 0;
    for &c in &commas {
        args.push(body[start..c].trim().to_string());
        start = c + 1;
    }
    let last = body[start..close].trim();
    if !last.is_empty() || !args.is_empty() {
        args.push(last.to_string());
    }
    let ret = body[close + 1..]
        .trim_start()
        .strip_prefix('=')
        .map_or("", str::trim)
        .to_string();
    Some(Call {
        pid,
        name: name.to_string(),
        args,
        ret,
    })
}

/// A quoted C string argument, unescaped; None if it is not one, or was
/// truncated.
fn cstr(arg: &str) -> Option<Vec<u8>> {
    let rest = arg.trim().strip_prefix('"')?;
    let b = rest.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        i += 1;
        match c {
            b'"' => {
                return if rest[i..].starts_with("...") {
                    None
                } else {
                    Some(out)
                };
            }
            b'\\' => {
                let e = *b.get(i)?;
                i += 1;
                match e {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'v' => out.push(0x0b),
                    b'f' => out.push(0x0c),
                    b'a' => out.push(0x07),
                    b'b' => out.push(0x08),
                    b'\\' | b'"' | b'\'' | b'?' => out.push(e),
                    b'x' => {
                        let mut v = 0u32;
                        let mut n = 0;
                        while n < 2 && i < b.len() && b[i].is_ascii_hexdigit() {
                            v = v * 16 + char::from(b[i]).to_digit(16)?;
                            i += 1;
                            n += 1;
                        }
                        if n == 0 {
                            return None;
                        }
                        out.push(v as u8);
                    }
                    b'0'..=b'7' => {
                        let mut v = u32::from(e - b'0');
                        let mut n = 1;
                        while n < 3 && i < b.len() && (b'0'..=b'7').contains(&b[i]) {
                            v = v * 8 + u32::from(b[i] - b'0');
                            i += 1;
                            n += 1;
                        }
                        out.push(v as u8);
                    }
                    _ => return None,
                }
            }
            _ => out.push(c),
        }
    }
    None
}

/// The path in strace's fd annotation: `3</a/b>` or `AT_FDCWD</a>`.
fn annotation(arg: &str) -> Option<&str> {
    let s = arg.find('<')?;
    let rest = &arg[s + 1..];
    let e = rest.rfind('>')?;
    Some(&rest[..e])
}

/// A path argument relative to a dirfd argument (or the working directory).
fn at_path(dirfd: &str, raw: &[u8], cwd: &Path) -> Option<PathBuf> {
    let p = Path::new(OsStr::from_bytes(raw));
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    let base = match annotation(dirfd) {
        Some(d) => PathBuf::from(d),
        None if dirfd.starts_with("AT_FDCWD") => cwd.to_path_buf(),
        None => return None,
    };
    Some(if raw.is_empty() { base } else { base.join(p) })
}

/// `..` and `.` resolved lexically (for classifying a path, not for keying it).
fn normal(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
}

#[derive(Debug, PartialEq)]
enum Addr {
    None,
    Unspec,
    Inet { local: bool, port: u16 },
    Unix(Vec<u8>),
    Other(String),
}

fn between<'s>(s: &'s str, a: &str, b: &str) -> Option<&'s str> {
    let i = s.find(a)? + a.len();
    let j = s[i..].find(b)?;
    Some(&s[i..i + j])
}

/// A socket address strace printed (`{sa_family=AF_INET, sin_port=htons(80),
/// sin_addr=inet_addr("1.2.3.4")}`).
fn sockaddr(s: &str) -> Addr {
    let Some(i) = s.find("sa_family=") else {
        return Addr::None;
    };
    let rest = &s[i + "sa_family=".len()..];
    let fam = rest.split([',', '}']).next().unwrap_or("").trim();
    let port = |key: &str| {
        between(rest, key, ")")
            .and_then(|p| p.parse().ok())
            .unwrap_or(0)
    };
    match fam {
        "AF_UNSPEC" => Addr::Unspec,
        "AF_INET" => Addr::Inet {
            local: between(rest, "inet_addr(\"", "\"")
                .is_some_and(|a| a.starts_with("127.") || a == "0.0.0.0"),
            port: port("sin_port=htons("),
        },
        "AF_INET6" => Addr::Inet {
            local: between(rest, "inet_pton(AF_INET6, \"", "\"").is_some_and(|a| {
                a == "::1" || a == "::" || a.starts_with("::ffff:127.") || a == "::ffff:0.0.0.0"
            }),
            port: port("sin6_port=htons("),
        },
        "AF_UNIX" => match rest.find("sun_path=") {
            Some(j) => {
                let p = &rest[j + "sun_path=".len()..];
                let (abstract_ns, p) = match p.strip_prefix('@') {
                    Some(p) => (true, p),
                    None => (false, p),
                };
                match cstr(p) {
                    Some(mut v) => {
                        if abstract_ns {
                            v.insert(0, b'@');
                        }
                        Addr::Unix(v)
                    }
                    None => Addr::Other("AF_UNIX".into()),
                }
            }
            None => Addr::Other("AF_UNIX (unnamed)".into()),
        },
        f => Addr::Other(f.to_string()),
    }
}

/// What a test binary's trace says it depended on; or why it cannot be
/// keyed.
struct Obs<'a> {
    workspace: &'a Path,
    scratch: &'a [PathBuf],
    objdir: PathBuf,
    pids: HashSet<u32>,
    /// Entries directly in a scratch directory the test created.
    tops: HashSet<PathBuf>,
    ports: HashSet<u16>,
    unix: HashSet<Vec<u8>>,
    inputs: BTreeSet<String>,
}

impl Obs<'_> {
    /// Paths that are not inputs: devices, the process's own /proc, the
    /// runtime's start-up reads, the test binary's own directory.
    fn ignored(&self, n: &Path) -> bool {
        n.starts_with("/dev")
            || n.starts_with("/proc/self")
            || n.starts_with("/proc/thread-self")
            || n.starts_with("/sys/kernel/mm/transparent_hugepage")
            || n.starts_with("/sys/fs/cgroup")
            || (!self.objdir.as_os_str().is_empty() && n.starts_with(&self.objdir))
            || n.strip_prefix("/proc")
                .ok()
                .and_then(|r| r.components().next())
                .and_then(|c| c.as_os_str().to_str())
                .and_then(|c| c.parse::<u32>().ok())
                .is_some_and(|pid| self.pids.contains(&pid))
    }

    fn read(&mut self, kind: &str, p: &Path) -> std::result::Result<(), String> {
        let n = normal(p);
        if self.ignored(&n) {
            return Ok(());
        }
        if !n.starts_with(self.workspace) {
            for s in self.scratch {
                if let Ok(r) = n.strip_prefix(s) {
                    let Some(first) = r.components().next() else {
                        // The temp directory itself (os.RemoveAll opens it).
                        return Ok(());
                    };
                    if self.tops.contains(&s.join(first.as_os_str())) {
                        return Ok(());
                    }
                }
            }
        }
        let key = p
            .to_str()
            .ok_or_else(|| format!("read a non-UTF-8 path {}", p.display()))?;
        self.inputs.insert(format!("{kind}:{key}"));
        Ok(())
    }

    fn wrote(&self, p: &Path) -> std::result::Result<(), String> {
        let n = normal(p);
        if self.ignored(&n) {
            return Ok(());
        }
        if n.starts_with(self.workspace) {
            return Err(format!("wrote {}", n.display()));
        }
        if self.scratch.iter().any(|s| n.starts_with(s)) {
            return Ok(());
        }
        Err(format!(
            "wrote {} (outside the workspace and the temp directories)",
            n.display()
        ))
    }

    /// A call that changes a path: a write if it succeeded, else what it
    /// found there (an existence) is an input.
    fn changed(&mut self, ok: bool, p: &Path) -> std::result::Result<(), String> {
        if ok { self.wrote(p) } else { self.read("s", p) }
    }

    fn dest(&self, s: &str) -> std::result::Result<(), String> {
        match sockaddr(s) {
            Addr::None | Addr::Unspec => Ok(()),
            Addr::Inet { local: true, port } if self.ports.contains(&port) => Ok(()),
            Addr::Unix(p) if self.unix.contains(&p) => Ok(()),
            Addr::Other(f) => Err(format!("used a {f} socket address")),
            _ => Err(format!(
                "used the network: {}",
                s.chars().take(160).collect::<String>()
            )),
        }
    }
}

/// The inputs a test binary's trace shows (`f:`, `d:`, `s:`, `l:` keys), or
/// why it cannot be recorded. `cwd0` is the directory it started in.
fn observe(
    calls: &[Call],
    cwd0: &Path,
    workspace: &Path,
    scratch: &[PathBuf],
) -> std::result::Result<BTreeSet<String>, String> {
    let first = calls
        .iter()
        .find(|c| c.name == "execve")
        .ok_or("no exec of the test binary in its trace")?;
    let bin = PathBuf::from(OsStr::from_bytes(
        &cstr(first.arg(0)).ok_or("an unparsed exec of the test binary")?,
    ));
    let mut o = Obs {
        workspace,
        scratch,
        objdir: bin.parent().map(Path::to_path_buf).unwrap_or_default(),
        pids: calls.iter().map(|c| c.pid).collect(),
        tops: HashSet::new(),
        ports: HashSet::new(),
        unix: HashSet::new(),
        inputs: BTreeSet::new(),
    };
    // What the process tree listened on and created, anywhere in the trace.
    for c in calls {
        if !c.ok() {
            continue;
        }
        match c.name.as_str() {
            "bind" | "getsockname" => match sockaddr(c.arg(1)) {
                Addr::Inet { port, .. } if port != 0 => {
                    o.ports.insert(port);
                }
                Addr::Unix(p) => {
                    o.unix.insert(p);
                }
                _ => {}
            },
            "mkdir" | "mkdirat" | "open" | "openat" | "openat2" => {
                let (dirfd, i, created) = match c.name.as_str() {
                    "mkdir" => ("AT_FDCWD", 0, true),
                    "mkdirat" => (c.arg(0), 1, true),
                    "open" => ("AT_FDCWD", 0, excl(c.arg(1))),
                    _ => (c.arg(0), 1, excl(c.arg(2))),
                };
                if created
                    && let Some(p) = cstr(c.arg(i)).and_then(|raw| at_path(dirfd, &raw, cwd0))
                {
                    let n = normal(&p);
                    if scratch.iter().any(|s| n.parent() == Some(s.as_path())) {
                        o.tops.insert(n);
                    }
                }
            }
            _ => {}
        }
    }
    let mut cwd = cwd0.to_path_buf();
    for c in calls {
        let name = c.name.as_str();
        let resolve = |dirfd: &str, i: usize, cwd: &Path| -> std::result::Result<PathBuf, String> {
            let raw = cstr(c.arg(i)).ok_or_else(|| format!("an unparsed path in {name}"))?;
            at_path(dirfd, &raw, cwd).ok_or_else(|| format!("an unresolved path in {name}"))
        };
        match name {
            "execve" => {
                let p = resolve("AT_FDCWD", 0, &cwd)?;
                if p != bin {
                    return Err(format!("ran {}", p.display()));
                }
            }
            "open" | "openat" | "openat2" | "creat" => {
                let (p, flags) = match name {
                    "open" => (resolve("AT_FDCWD", 0, &cwd)?, c.arg(1)),
                    "creat" => (resolve("AT_FDCWD", 0, &cwd)?, "O_WRONLY|O_CREAT|O_TRUNC"),
                    _ => (resolve(c.arg(0), 1, &cwd)?, c.arg(2)),
                };
                let write = [
                    "O_WRONLY",
                    "O_RDWR",
                    "O_CREAT",
                    "O_TRUNC",
                    "O_APPEND",
                    "O_TMPFILE",
                ]
                .into_iter()
                .any(|f| flags.contains(f));
                if write {
                    o.changed(c.ok(), &p)?;
                } else if flags.contains("O_DIRECTORY") {
                    o.read("d", &p)?;
                } else {
                    o.read("f", &p)?;
                }
            }
            "stat" | "lstat" | "access" | "statfs" | "getxattr" | "lgetxattr" | "listxattr"
            | "llistxattr" => {
                let p = resolve("AT_FDCWD", 0, &cwd)?;
                o.read("s", &p)?;
            }
            "newfstatat" | "fstatat64" | "statx" | "faccessat" | "faccessat2" => {
                if c.arg(1) == "\"\"" {
                    continue; // fstat of an open fd
                }
                let p = resolve(c.arg(0), 1, &cwd)?;
                o.read("s", &p)?;
            }
            "readlink" => {
                let p = resolve("AT_FDCWD", 0, &cwd)?;
                o.read("l", &p)?;
            }
            "readlinkat" => {
                let p = resolve(c.arg(0), 1, &cwd)?;
                o.read("l", &p)?;
            }
            "getcwd" => {}
            "chdir" => {
                let p = resolve("AT_FDCWD", 0, &cwd)?;
                o.read("s", &p)?;
                if c.ok() {
                    cwd = p;
                }
            }
            "fchdir" => {
                if c.ok() {
                    cwd = PathBuf::from(
                        annotation(c.arg(0)).ok_or("changed to an unknown directory")?,
                    );
                }
            }
            "mkdir" | "rmdir" | "unlink" | "chmod" | "chown" | "lchown" | "truncate" | "utime"
            | "utimes" | "mknod" | "setxattr" | "lsetxattr" | "removexattr" | "lremovexattr" => {
                let p = resolve("AT_FDCWD", 0, &cwd)?;
                o.changed(c.ok(), &p)?;
            }
            "mkdirat" | "unlinkat" | "fchmodat" | "fchmodat2" | "fchownat" | "mknodat"
            | "futimesat" => {
                let p = resolve(c.arg(0), 1, &cwd)?;
                o.changed(c.ok(), &p)?;
            }
            "utimensat" => {
                if c.arg(1) != "NULL" {
                    let p = resolve(c.arg(0), 1, &cwd)?;
                    o.changed(c.ok(), &p)?;
                }
            }
            "rename" | "link" => {
                for i in 0..2 {
                    let p = resolve("AT_FDCWD", i, &cwd)?;
                    o.changed(c.ok(), &p)?;
                }
            }
            "renameat" | "renameat2" | "linkat" => {
                for (d, i) in [(0, 1), (2, 3)] {
                    let p = resolve(c.arg(d), i, &cwd)?;
                    o.changed(c.ok(), &p)?;
                }
            }
            "symlink" => {
                let p = resolve("AT_FDCWD", 1, &cwd)?;
                o.changed(c.ok(), &p)?;
            }
            "symlinkat" => {
                let p = resolve(c.arg(1), 2, &cwd)?;
                o.changed(c.ok(), &p)?;
            }
            "socket" => {
                let d = c.arg(0);
                if !matches!(d, "AF_INET" | "AF_INET6" | "AF_UNIX") {
                    return Err(format!("opened a {d} socket"));
                }
            }
            "connect" => o.dest(c.arg(1))?,
            "sendto" => o.dest(c.arg(4))?,
            "sendmsg" => {
                if !c.arg(1).contains("msg_name=NULL") {
                    o.dest(c.arg(1))?;
                }
            }
            "bind" | "getsockname" => {}
            other => return Err(format!("called {other}")),
        }
    }
    Ok(o.inputs)
}

/// open flags that create a file only if it did not exist.
fn excl(flags: &str) -> bool {
    flags.contains("O_CREAT") && flags.contains("O_EXCL")
}

// ---- helpers ----

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
        Rng(SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64)
            | 1)
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

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn args_split_flags_from_patterns() {
        let p = parse_args(&s(&[
            "-json", "-run", "TestA", "-tags=x", "./...", "-count=1",
        ]))
        .unwrap();
        assert_eq!(p.flags, s(&["-json", "-run=TestA", "-tags=x", "-count=1"]));
        assert_eq!(p.list, s(&["-tags=x"]));
        assert_eq!(p.patterns, s(&["./..."]));
        assert!(p.json && p.count);
        assert_eq!(parse_args(&[]).unwrap().patterns, s(&["."]));
        assert!(parse_args(&s(&["-coverprofile=c.out", "./..."])).is_err());
        assert!(parse_args(&s(&["-exec", "x", "./..."])).is_err());
        assert!(parse_args(&s(&["./...", "-args", "-x"])).is_err());
        assert!(parse_args(&s(&["a_test.go"])).is_err());
        assert!(parse_args(&s(&["-shuffle=on"])).is_err());
        assert!(!parse_args(&s(&["-json=false"])).unwrap().json);
    }

    #[test]
    fn text_output_merges_replays_in_order() {
        let roots = s(&["m/a", "m/b", "m/c", "m/d", "m/e"]);
        let run = s(&["m/b", "m/d"]);
        let replays: BTreeMap<String, String> = [
            ("m/a".to_string(), "ok  \tm/a\t0.1s\n".to_string()),
            (
                "m/c".to_string(),
                "?   \tm/c\t[no test files]\n".to_string(),
            ),
            ("m/e".to_string(), "ok  \tm/e\t(cached)\n".to_string()),
        ]
        .into_iter()
        .collect();
        let mut m = Merger::new(&run, &roots, &replays, false);
        let mut out = Vec::new();
        for l in [
            "--- FAIL: TestX (0.00s)\n",
            "FAIL\n",
            "FAIL\tm/b\t0.2s\n",
            "ok  \tm/d\t0.3s\n",
            "FAIL\n",
        ] {
            m.line(l.as_bytes(), &mut out).unwrap();
        }
        let chunks = m.finish(&mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "ok  \tm/a\t0.1s\n--- FAIL: TestX (0.00s)\nFAIL\nFAIL\tm/b\t0.2s\n?   \tm/c\t[no test files]\nok  \tm/d\t0.3s\nok  \tm/e\t(cached)\nFAIL\n"
        );
        assert_eq!(chunks["m/b"].status, "fail");
        assert_eq!(chunks["m/b"].tests["TestX"], "fail");
        assert_eq!(chunks["m/d"].status, "ok");
        assert_eq!(chunks["m/d"].output, "ok  \tm/d\t0.3s\n");
    }

    #[test]
    fn json_output_merges_by_package() {
        let roots = s(&["m/a", "m/b", "m/c"]);
        let run = s(&["m/a", "m/c"]);
        let replays: BTreeMap<String, String> = [(
            "m/b".to_string(),
            "{\"Action\":\"pass\",\"Package\":\"m/b\"}\n".to_string(),
        )]
        .into_iter()
        .collect();
        let mut m = Merger::new(&run, &roots, &replays, true);
        let mut out = Vec::new();
        for l in [
            "{\"Action\":\"run\",\"Package\":\"m/a\",\"Test\":\"TestA\"}\n",
            "{\"Action\":\"pass\",\"Package\":\"m/a\",\"Test\":\"TestA\"}\n",
            "{\"Action\":\"pass\",\"Package\":\"m/a\"}\n",
            "{\"ImportPath\":\"m/c [m/c.test]\",\"Action\":\"build-output\",\"Output\":\"x\"}\n",
            "{\"Action\":\"fail\",\"Package\":\"m/c\"}\n",
        ] {
            m.line(l.as_bytes(), &mut out).unwrap();
        }
        let chunks = m.finish(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        let b = out.find("m/b").unwrap();
        assert!(out.find("\"Package\":\"m/a\"}").unwrap() < b);
        assert!(b < out.find("m/c.test").unwrap());
        assert_eq!(chunks["m/a"].status, "ok");
        assert_eq!(chunks["m/a"].tests["TestA"], "pass");
        assert_eq!(chunks["m/c"].status, "fail");
    }

    #[test]
    fn nothing_to_run_prints_every_replay() {
        let roots = s(&["m/a", "m/b"]);
        let replays: BTreeMap<String, String> = [
            ("m/b".to_string(), "B\n".to_string()),
            ("m/a".to_string(), "A\n".to_string()),
        ]
        .into_iter()
        .collect();
        let m = Merger::new(&[], &roots, &replays, false);
        let mut out = Vec::new();
        m.finish(&mut out).unwrap();
        assert_eq!(out, b"A\nB\n");
    }

    #[test]
    fn terminators_match_the_exact_package() {
        assert_eq!(terminator("ok  \tm/a\t0.1s\n", "m/a"), Some("ok"));
        assert_eq!(terminator("ok  \tm/ab\t0.1s\n", "m/a"), None);
        assert_eq!(
            terminator("FAIL\tm/a [build failed]\n", "m/a"),
            Some("fail")
        );
        assert_eq!(
            terminator("?   \tm/a\t[no test files]\n", "m/a"),
            Some("nofiles")
        );
        assert_eq!(
            verbose_result("    --- PASS: TestA/sub_1 (0.00s)\n"),
            Some(("TestA/sub_1".into(), "pass".into()))
        );
    }

    #[test]
    fn cstrings_unescape() {
        assert_eq!(cstr("\"a\\nb\\\"c\"").unwrap(), b"a\nb\"c");
        assert_eq!(
            cstr("\"\\303\\251\\x41\"").unwrap(),
            vec![0o303, 0o251, 0x41]
        );
        assert_eq!(cstr("\"abc\"..."), None);
        assert_eq!(cstr("AT_FDCWD"), None);
    }

    #[test]
    fn traces_join_interrupted_calls() {
        let t = "10 execve(\"/tmp/go-build1/b001/a.test\", [\"/tmp/go-build1/b001/a.test\"], 0x7ffd /* 3 vars */) = 0\n\
                 11 openat(AT_FDCWD</w/a>, \"testdata/x, y\", O_RDONLY|O_CLOEXEC <unfinished ...>\n\
                 10 newfstatat(AT_FDCWD</w/a>, \"go.mod\", {st_mode=S_IFREG|0644, st_size=10, ...}, 0) = 0\n\
                 11 <... openat resumed>) = 3</w/a/testdata/x, y>\n\
                 12 openat(AT_FDCWD, \"/etc/hosts\", O_RDONLY <unfinished ...>\n\
                 13 ???( <detached ...>\n";
        let calls = parse_trace(t).unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[2].name, "openat");
        assert_eq!(calls[2].args.len(), 3);
        assert_eq!(calls[2].ret, "3</w/a/testdata/x, y>");
        assert_eq!(calls[3].ret, "?");
        let got = observe(
            &calls,
            Path::new("/w/a"),
            Path::new("/w"),
            &[PathBuf::from("/tmp")],
        )
        .unwrap();
        let want: BTreeSet<String> = ["f:/w/a/testdata/x, y", "s:/w/a/go.mod", "f:/etc/hosts"]
            .iter()
            .map(|x| x.to_string())
            .collect();
        assert_eq!(got, want);
    }

    fn obs(lines: &[&str]) -> std::result::Result<BTreeSet<String>, String> {
        let mut t =
            String::from("1 execve(\"/tmp/go-build9/b001/p.test\", [], 0x1 /* 1 var */) = 0\n");
        for l in lines {
            t.push_str(l);
            t.push('\n');
        }
        let calls = parse_trace(&t).unwrap();
        observe(
            &calls,
            Path::new("/w/p"),
            Path::new("/w"),
            &[PathBuf::from("/tmp")],
        )
    }

    #[test]
    fn scratch_the_test_made_is_not_an_input() {
        let got = obs(&[
            "1 mkdirat(AT_FDCWD</w/p>, \"/tmp/TestA123\", 0700) = 0",
            "1 openat(AT_FDCWD</w/p>, \"/tmp/TestA123/f\", O_RDWR|O_CREAT|O_TRUNC|O_CLOEXEC, 0666) = 3</tmp/TestA123/f>",
            "1 openat(AT_FDCWD</w/p>, \"/tmp/TestA123/f\", O_RDONLY|O_CLOEXEC) = 3</tmp/TestA123/f>",
            "1 openat(AT_FDCWD</w/p>, \"/tmp\", O_RDONLY|O_CLOEXEC|O_DIRECTORY) = 4</tmp>",
            "1 openat(AT_FDCWD</w/p>, \"/tmp/other\", O_RDONLY|O_CLOEXEC) = -1 ENOENT (No such file or directory)",
            "1 openat(AT_FDCWD</w/p>, \"/tmp/go-build9/b001/p.test\", O_RDONLY|O_CLOEXEC) = 3",
            "1 openat(AT_FDCWD</w/p>, \"/proc/self/cgroup\", O_RDONLY|O_CLOEXEC) = 3",
            "1 openat(AT_FDCWD</w/p>, \"testdata\", O_RDONLY|O_CLOEXEC|O_DIRECTORY) = 3</w/p/testdata>",
        ])
        .unwrap();
        let want: BTreeSet<String> = ["f:/tmp/other", "d:/w/p/testdata"]
            .iter()
            .map(|x| x.to_string())
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn unrecordable_traces() {
        assert!(
            obs(&["1 openat(AT_FDCWD</w/p>, \"out.txt\", O_WRONLY|O_CREAT|O_TRUNC, 0644) = 3"])
                .is_err()
        );
        assert!(obs(&["1 mkdirat(AT_FDCWD</w/p>, \"/home/u/.cache/x\", 0755) = 0"]).is_err());
        assert!(obs(&["2 execve(\"/usr/bin/git\", [\"git\"], 0x1 /* 1 var */) = 0"]).is_err());
        assert!(obs(&["1 connect(3<TCP:[1]>, {sa_family=AF_INET, sin_port=htons(443), sin_addr=inet_addr(\"140.82.112.3\")}, 16) = 0"]).is_err());
        assert!(obs(&["1 connect(3<UDP:[1]>, {sa_family=AF_INET, sin_port=htons(53), sin_addr=inet_addr(\"127.0.0.53\")}, 16) = 0"]).is_err());
        assert!(obs(&["1 socket(AF_NETLINK, SOCK_RAW|SOCK_CLOEXEC, NETLINK_ROUTE) = 3"]).is_err());
        assert!(
            obs(&["1 mount(\"x\", \"/y\", NULL, 0, NULL) = -1 EPERM (Operation not permitted)"])
                .is_err()
        );
        // Its own listener, and a failed write in the workspace (an existence check).
        let ok = obs(&[
            "1 socket(AF_INET, SOCK_STREAM|SOCK_CLOEXEC|SOCK_NONBLOCK, IPPROTO_IP) = 3",
            "1 bind(3<TCP:[1]>, {sa_family=AF_INET, sin_port=htons(0), sin_addr=inet_addr(\"127.0.0.1\")}, 16) = 0",
            "1 getsockname(3<TCP:[1]>, {sa_family=AF_INET, sin_port=htons(40123), sin_addr=inet_addr(\"127.0.0.1\")}, [112 => 16]) = 0",
            "1 connect(4<TCP:[2]>, {sa_family=AF_INET, sin_port=htons(40123), sin_addr=inet_addr(\"127.0.0.1\")}, 16) = -1 EINPROGRESS (Operation now in progress)",
            "1 mkdirat(AT_FDCWD</w/p>, \"testdata\", 0777) = -1 EEXIST (File exists)",
        ])
        .unwrap();
        assert_eq!(
            ok,
            ["s:/w/p/testdata".to_string()]
                .into_iter()
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn test_files_count_only_in_the_test_build() {
        let mut p = Pkg {
            go_files: s(&["a.go"]),
            test_go_files: s(&["a_test.go"]),
            x_test_go_files: s(&["x_test.go"]),
            ..Pkg::default()
        };
        let kinds = |p: &Pkg| -> Vec<&str> {
            p.lists()
                .into_iter()
                .filter(|(_, l)| !l.is_empty())
                .map(|(k, _)| k)
                .collect()
        };
        assert_eq!(kinds(&p), vec!["go"]);
        p.for_test = "m/a".into();
        assert_eq!(kinds(&p), vec!["go", "test", "xtest"]);
    }

    #[test]
    fn sockaddrs() {
        assert_eq!(
            sockaddr(
                "{sa_family=AF_INET6, sin6_port=htons(8080), sin6_flowinfo=htonl(0), inet_pton(AF_INET6, \"::1\", &sin6_addr), sin6_scope_id=0}"
            ),
            Addr::Inet {
                local: true,
                port: 8080
            }
        );
        assert_eq!(
            sockaddr("{sa_family=AF_UNIX, sun_path=\"/tmp/s.sock\"}"),
            Addr::Unix(b"/tmp/s.sock".to_vec())
        );
        assert_eq!(sockaddr("NULL"), Addr::None);
    }
}
