//! What a step can see. Three surfaces:
//!
//! - **Hermetic** (every keyed step): everything the step can read is in
//!   its key or observed. The image root (keyed by digest) under the job's
//!   layers, the workspace tree (observed), a fresh RUNNER_TEMP holding only
//!   the step's script and the command files at fixed paths, the files the
//!   runner provides that the step executes (an action's directory, node
//!   from the runner's externals) bound read-only and keyed by content, a
//!   fixed hostname, /etc/hosts and resolv.conf. No network, and every
//!   attempt to reach one is counted. The environment is the keyed one:
//!   volatile names and GitHub tokens are withheld (set, to a value that
//!   works as nothing), the ACTIONS_* service variables are unreachable
//!   placeholders, and values classified as secrets (`secrets::classify`:
//!   by name, shape, the workflow's provenance, or holding a known secret)
//!   are given but salted in the key and masked in everything stored.
//! - **Network** (a JavaScript action or a network-rule script, second
//!   try, after it failed or reached for the network offline): the
//!   hermetic surface plus egress, minus every secret: what it fetches may
//!   be stored (a module cache, a tool-cache install) because it never held
//!   anything to leak. Never recorded.
//! - **Runner** (tainted steps, and the last try): what actions/runner
//!   would give a container step: its mounts (never the docker socket),
//!   the whole environment, egress. Never recorded, nothing it writes
//!   reaches the store.
//!
//! All three pin where tools keep state (HOME, GOPATH, GOMODCACHE, GOCACHE,
//! the tool cache) under the observed workspace, unless the workflow set
//! one of them: then the step is tainted and keeps the workflow's value.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;
use quod_cas::{Cas, Hash};
use quod_manifest::FileCommand;
use quod_sandbox::Mount;

use crate::checkout::tree_to_json;
use crate::job::{Job, STATE_SUBDIR};
use crate::protocol::ScriptStepArgs;
use crate::secrets;
use crate::step::{self, Kind, Step};
use crate::trust;

/// Inside the hermetic RUNNER_TEMP: the step's script.
const SCRIPT: &str = "quod-step";

/// The value a withheld variable has in a keyed step: set, so presence
/// checks behave as on GitHub, and useless as a token, a sha or an id.
pub const WITHHELD: &str = "quod-withheld";

/// What a placeholder leaves in a step's result: `WITHHELD` (withheld names,
/// ACTIONS_* tokens) and the unreachable ACTIONS_* service address. A
/// result that holds one is not what GitHub would give (`job::run_step`
/// runs the step again with the runner's environment, never records it).
///
/// For `WITHHELD` the mark is its first 7 bytes, which every prefix of 7 or
/// more holds too: a short sha (`${GITHUB_SHA::7}`, the usual abbreviation)
/// cut from the placeholder is as much the placeholder.
pub const PLACEHOLDER_MARKS: &[&str] = &[WITHHELD_MARK, "quod-unreachable"];

/// `WITHHELD`'s first 7 bytes (see `PLACEHOLDER_MARKS`).
pub const WITHHELD_MARK: &str = "quod-wi";

pub fn placeholder_marks() -> Vec<Vec<u8>> {
    PLACEHOLDER_MARKS
        .iter()
        .map(|m| m.as_bytes().to_vec())
        .collect()
}

/// The first command file (GITHUB_ENV, GITHUB_OUTPUT, the step summary,
/// ...) the step wrote a placeholder into.
pub fn placeholder_in_files(files: &[PinnedFile]) -> Option<String> {
    let marks = placeholder_marks();
    let ac = aho_corasick::AhoCorasick::new(&marks).ok()?;
    files
        .iter()
        .find(|f| {
            if f.dir {
                secrets::file_containing(&f.host, &marks).map_or(true, |p| p.is_some())
            } else {
                std::fs::read(&f.host).is_ok_and(|b| ac.find(&b).is_some())
            }
        })
        .map(|f| f.name.clone())
}
/// Inside RUNNER_TEMP: where the runner keeps per-step command files.
const FILE_COMMANDS_DIR: &str = "_runner_file_commands";
/// What an offline step resolves names with: an address with no route, so
/// a lookup is counted as a network attempt instead of going to loopback.
const RESOLV_CONF: &str = quod_sandbox::OFFLINE_RESOLV_CONF;
/// The sandbox's own name (`quod_sandbox::HOSTNAME`) resolves locally, as a
/// container's does: a test that looks itself up must not reach for DNS.
const HOSTS: &str = quod_sandbox::OFFLINE_HOSTS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    Hermetic,
    Network,
    Runner,
}

pub struct Surface {
    pub kind: SurfaceKind,
    /// The complete environment the step sees.
    pub env: Vec<(String, String)>,
    /// Hermetic: the environment as the key holds it (secret values
    /// salted).
    pub key_env: Vec<(String, String)>,
    /// The command as executed.
    pub cmd: Vec<String>,
    /// Binds (host paths) besides the workspace overlay, the sealed module
    /// cache and GOCACHE.
    pub mounts: Vec<Mount>,
    pub egress: bool,
    /// Names the job's environment had that the step did not get as they
    /// were (withheld, or replaced by a placeholder), sorted: the
    /// manifest's present-but-unkeyed names.
    pub withheld: Vec<String>,
    /// Of those, the ones whose real value the step did not get (it got
    /// `WITHHELD`) and that are stable from run to run
    /// (`scan::scanned`): a file naming one means the step may have needed
    /// it.
    pub absent: Vec<String>,
    /// Hermetic and network: the command files at their fixed paths, to
    /// hand back to the runner after the step.
    pub files: Vec<PinnedFile>,
    /// Hermetic: the content identity of runner-provided files the step
    /// executes, appended to the key's command.
    pub key_extra: Vec<String>,
    /// Secret values the step was given, and the job's known secrets, in
    /// every encoding: masked in stored logs; outputs or command files
    /// containing one are not stored.
    pub masks: Vec<Vec<u8>>,
    /// The names of the variables classified as secrets (`secrets::classify`).
    pub classified: Vec<String>,
    /// The classified values (for the job's known secrets).
    pub secret_values: std::collections::BTreeSet<String>,
    /// A classified variable too short to mask reliably: the step is not
    /// recorded.
    pub short_secret: Option<String>,
    /// That short value is a certain secret (`secrets::Classified::strong`).
    pub short_strong: bool,
    /// Hermetic and network: the step's own RUNNER_TEMP on the host.
    pub temp: Option<PathBuf>,
    /// Why this step cannot be keyed (a workflow-set pin, a GOFLAGS or
    /// GODEBUG the warm state conflicts with).
    pub taint: Vec<String>,
}

/// One runner command file (GITHUB_OUTPUT, ...) as the step sees it.
pub struct PinnedFile {
    pub name: String,
    /// In the step's own RUNNER_TEMP on the host.
    pub host: PathBuf,
    /// The runner's file for this step, on the host.
    pub runner: Option<PathBuf>,
    pub dir: bool,
    /// What the runner had already written there (GITHUB_ARTIFACTS_LIST
    /// is an input): given to the step, keyed, and not handed back.
    pub initial: Vec<u8>,
}

/// What every surface sets SOURCE_DATE_EPOCH to unless the workflow does
/// (`base_env`): the CLI's Python profile's value.
pub const SOURCE_DATE_EPOCH: &str = "315532800";

/// The variables Quod pins so tool state lives in the observed tree.
const PINNED: &[&str] = &[
    "HOME",
    "GOPATH",
    "GOMODCACHE",
    "GOCACHE",
    "GOTOOLCHAIN",
    "RUNNER_TOOL_CACHE",
    "GIT_OPTIONAL_LOCKS",
];

/// Does the workflow (job container env, job/step env, an earlier step's
/// GITHUB_ENV) set `name` to something other than `pin`, as opposed to the
/// image, the runner's own defaults, or the very value Quod pins
/// (setup-go exports GOTOOLCHAIN=local)?
fn workflow_sets(job: &Job, args: &ScriptStepArgs, name: &str, pin: &str) -> bool {
    let runner_default = |v: &str| match name {
        "HOME" => v == "/github/home",
        "RUNNER_TOOL_CACHE" => step::tool_dir(job).is_some_and(|t| t == v),
        _ => false,
    };
    let container = job.container_env();
    [container.get(name), args.environment_variables.get(name)]
        .into_iter()
        .flatten()
        .any(|v| !runner_default(v) && v != pin)
}

/// The environment every surface starts from: the image's, the docker
/// defaults (GITHUB_ACTIONS, CI), the job container's, the step's, PATH
/// with the runner's prepends, and Quod's pins; and the taint when
/// the workflow set something Quod would pin.
fn base_env(
    job: &Job,
    args: &ScriptStepArgs,
    offline: bool,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    if let Some(r) = &job.rootfs {
        env.extend(r.env.iter().cloned());
    } else {
        env.insert(
            "PATH".into(),
            "/usr/local/go/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
        );
    }
    // What the runner's docker hooks add to every job container.
    env.insert("GITHUB_ACTIONS".into(), "true".into());
    env.insert("CI".into(), "true".into());
    env.extend(job.container_env());
    env.extend(args.environment_variables.clone());
    let base_path = env
        .get("PATH")
        .cloned()
        .unwrap_or_else(|| "/usr/bin:/bin".into());
    if !args.prepend_path.is_empty() {
        env.insert(
            "PATH".into(),
            format!("{}:{base_path}", args.prepend_path.join(":")),
        );
    }
    let state = format!("{}/{STATE_SUBDIR}", job.working_directory);
    let pins = [
        ("HOME", format!("{state}/home")),
        ("GOPATH", format!("{state}/go")),
        ("GOMODCACHE", format!("{state}/gomod")),
        ("GOCACHE", format!("{state}/gocache")),
        ("GOTOOLCHAIN", "local".to_string()),
        ("RUNNER_TOOL_CACHE", format!("{state}/tool")),
        // git status (go build's VCS stamping runs it) rewrites .git/index
        // when it can take the lock; with the observed tree's stat data that
        // write differs every run. Without optional locks git only reads.
        ("GIT_OPTIONAL_LOCKS", "0".to_string()),
    ];
    let mut taint = Vec::new();
    for (name, value) in pins {
        debug_assert!(PINNED.contains(&name));
        if workflow_sets(job, args, name, &value) {
            taint.push(format!(
                "the workflow sets {name}, which the quod runner would pin"
            ));
        } else {
            env.insert(name.into(), value);
        }
    }
    // Install-time bytecode that does not depend on when it was installed.
    // pip (and uv with UV_COMPILE_BYTECODE) compile what they install with
    // py_compile, whose default under SOURCE_DATE_EPOCH is a checked-hash
    // pyc (PEP 552): validated by its source's hash, not its mtime. Without
    // it the pyc embeds the install step's wall-clock mtime, so every
    // re-run of the install (pip install is never replayed: the network)
    // gave new pyc bytes, and every step that imported them missed. The
    // value is the CLI's Python profile's (`quod_key::PYTHON`), 1980-01-01,
    // the earliest a zip can hold. A workflow's own value is kept (and
    // keyed, as every variable is), never a taint: it is the same knob.
    if !env.contains_key("SOURCE_DATE_EPOCH") {
        env.insert("SOURCE_DATE_EPOCH".into(), SOURCE_DATE_EPOCH.into());
    }
    if offline {
        env.insert("GOPROXY".into(), "off".into());
    }
    (env, taint)
}

/// The env as a list, with the warm-state knobs merged; a step that sets
/// them otherwise is tainted (it keeps its own values).
fn finish_env(env: BTreeMap<String, String>, taint: &mut Vec<String>) -> Vec<(String, String)> {
    let env: Vec<(String, String)> = env.into_iter().collect();
    let mut merged = env.clone();
    // The Go profile's warm-state knobs (GOFLAGS -count=1, GODEBUG
    // goindex=0), the ones `quod run --warm` merges.
    let knobs = quod_key::Toolchain::Go
        .profile()
        .warm
        .as_ref()
        .map_or(&[][..], |w| w.knobs);
    match knobs
        .iter()
        .try_for_each(|k| quod_key::merge_knob(&mut merged, k))
    {
        Ok(()) => merged,
        Err(e) => {
            taint.push(format!("{e}"));
            env
        }
    }
}

/// RUNNER_TEMP inside the container.
pub fn runner_temp(job: &Job, args: &ScriptStepArgs) -> String {
    args.environment_variables
        .get("RUNNER_TEMP")
        .cloned()
        .or_else(|| step::temp_dir(job))
        .unwrap_or_else(|| "/__w/_temp".into())
        .trim_end_matches('/')
        .to_string()
}

/// The hermetic surface (see the module docs). `step_dir` holds the step's
/// own RUNNER_TEMP and the fixed name-resolution files.
pub fn hermetic(
    job: &mut Job,
    args: &ScriptStepArgs,
    step: &Step,
    step_dir: &Path,
    cas: &Cas,
    salt: &[u8; 32],
) -> anyhow::Result<Surface> {
    let (env, taint) = base_env(job, args, true);
    sealed(
        job,
        args,
        step,
        step_dir,
        cas,
        salt,
        env,
        taint,
        SurfaceKind::Hermetic,
    )
}

/// The network surface (see the module docs).
pub fn network(
    job: &mut Job,
    args: &ScriptStepArgs,
    step: &Step,
    step_dir: &Path,
    cas: &Cas,
    salt: &[u8; 32],
) -> anyhow::Result<Surface> {
    let (mut env, taint) = base_env(job, args, false);
    // No secrets reach the network: what this step writes may be stored.
    // Strip by a telling name or a secret-looking value alike, and run
    // online without them. This is the fetch surface: a step here failed
    // offline and is being retried for the network only, so a token it
    // merely carries (setup-go's github.token, which the public Go download
    // never uses) is dropped, not a reason to taint. If the step truly
    // needed the secret it will fail online, and the caller reports it a
    // tainted failure rather than run it again (never a repeated effect).
    let c = secrets::classify_with(
        &env,
        job.provenance(),
        job.known_secrets(),
        &job.set_by_steps,
        &job.image_env(),
    );
    env.retain(|k, _| !c.is_secret(k));
    sealed(
        job,
        args,
        step,
        step_dir,
        cas,
        salt,
        env,
        taint,
        SurfaceKind::Network,
    )
}

#[allow(clippy::too_many_arguments)]
fn sealed(
    job: &mut Job,
    args: &ScriptStepArgs,
    step: &Step,
    step_dir: &Path,
    cas: &Cas,
    salt: &[u8; 32],
    mut env: BTreeMap<String, String>,
    mut taint: Vec<String>,
    kind: SurfaceKind,
) -> anyhow::Result<Surface> {
    let rt = runner_temp(job, args);
    let temp = step_dir.join("temp");
    let cmd_dir = temp.join(FILE_COMMANDS_DIR);
    std::fs::create_dir_all(&cmd_dir).with_context(|| format!("create {}", cmd_dir.display()))?;

    // Command files: the runner's paths carry a fresh GUID per step; the
    // step gets fixed ones, so they can be keyed and named freely.
    let prefix = format!("{rt}/{FILE_COMMANDS_DIR}/");
    let mut files = Vec::new();
    let mut key_extra = Vec::new();
    for (k, v) in env.clone() {
        if !v.starts_with(&prefix) {
            continue;
        }
        let fixed = k.to_ascii_lowercase();
        let host = cmd_dir.join(&fixed);
        let runner = job.command_file(&v);
        let dir = runner.as_ref().is_some_and(|p| p.is_dir());
        let mut initial = Vec::new();
        if dir {
            std::fs::create_dir_all(&host)?;
            if runner
                .as_ref()
                .and_then(|r| std::fs::read_dir(r).ok())
                .is_some_and(|mut d| d.next().is_some())
            {
                anyhow::bail!(
                    "the runner gave {k} as a non-empty directory; the quod runner cannot key it"
                );
            }
        } else {
            let given = match &runner {
                Some(r) => std::fs::read(r).unwrap_or_default(),
                None => Vec::new(),
            };
            if !given.is_empty() {
                key_extra.push(format!("quod:given {k} {}", Hash::of(&given)));
            }
            std::fs::write(&host, &given)?;
            initial = given;
        }
        env.insert(k.clone(), format!("{prefix}{fixed}"));
        files.push(PinnedFile {
            name: k,
            host,
            runner,
            dir,
            initial,
        });
    }

    // Withheld names keep their name with a value that does not work
    // (`WITHHELD`), as ACTIONS_* service variables get placeholders: a step
    // that checks whether one is set behaves as on GitHub (a test that
    // skips without GITHUB_TOKEN runs), and a step that uses one fails or
    // reaches for the network, which taints it. Their real values never
    // reach a keyed step.
    let mut withheld: Vec<String> = Vec::new();
    let mut absent: Vec<String> = Vec::new();
    for (k, v) in env.clone() {
        if step::withheld(&k, &v) {
            env.insert(k.clone(), WITHHELD.to_string());
            withheld.push(k.clone());
            if crate::scan::scanned(&k, &v) {
                absent.push(k);
            }
        } else if let Some(p) = step::placeholder(&k) {
            env.insert(k.clone(), p.to_string());
            withheld.push(k);
        }
    }
    withheld.sort();
    absent.sort();

    // Secrets the step is given: by a telling name, a secret's shape, the
    // workflow's provenance, or holding one of the job's known secrets (the
    // protocol gives the hook no mask list). Salted in the key, masked in
    // everything stored. Placeholders are no secrets, whatever their names.
    let c = secrets::classify_with(
        &env,
        job.provenance(),
        job.known_secrets(),
        &job.set_by_steps,
        &job.image_env(),
    );
    let mut key_env = env.clone();
    for k in &c.names {
        if let Some(v) = env.get(k) {
            key_env.insert(k.clone(), secrets::salted(salt, k, v));
        }
    }

    let mut cmd = vec![args.entry_point.clone()];
    cmd.extend(args.entry_point_args.iter().cloned());
    let mut mounts = vec![Mount {
        src: temp.clone(),
        dst: PathBuf::from(&rt),
        writable: true,
    }];
    // Name resolution: fixed and unroutable offline (attempts are counted),
    // the host's online.
    let (resolv, hosts) = if kind == SurfaceKind::Hermetic {
        let r = step_dir.join("resolv.conf");
        let h = step_dir.join("hosts");
        std::fs::write(&r, RESOLV_CONF)?;
        std::fs::write(&h, HOSTS)?;
        (Some(r), Some(h))
    } else {
        let host = |f: &str| Path::new(f).is_file().then(|| PathBuf::from(f));
        (host("/etc/resolv.conf"), host("/etc/hosts"))
    };
    for (src, dst) in [(resolv, "/etc/resolv.conf"), (hosts, "/etc/hosts")] {
        if let Some(src) = src {
            mounts.push(Mount {
                src,
                dst: dst.into(),
                writable: false,
            });
        }
    }

    if let Kind::Script { arg, .. } = &step.kind {
        let runner_script = job
            .host_path(&args.entry_point_args[arg - 1])
            .context("the step's script is not under a runner mount")?;
        let ext = runner_script
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let name = format!("{SCRIPT}{ext}");
        std::fs::copy(&runner_script, temp.join(&name))
            .with_context(|| format!("copy {}", runner_script.display()))?;
        cmd[*arg] = format!("{rt}/{name}");
    }

    // node (or anything else) from the runner's externals: bound read-only,
    // keyed by content. The externals mount is read-only for every step, so
    // one hash per job is the content every step of the job sees.
    if let Some(ext) = step::externals_dir(job)
        && args.entry_point.starts_with(&format!("{ext}/"))
    {
        let host = job
            .host_path(&args.entry_point)
            .context("entry point under the externals mount without a host path")?;
        let h = job.file_hash(&host)?;
        mounts.push(Mount {
            src: host,
            dst: PathBuf::from(&args.entry_point),
            writable: false,
        });
        key_extra.push(format!("quod:file {} {h}", args.entry_point));
    }

    // The action's own directory: the JavaScript action, or the remote
    // composite action a `run:` step belongs to (GITHUB_ACTION_PATH).
    let action = match &step.kind {
        Kind::Action(a) => Some(a.clone()),
        _ => step::actions_dir(job).and_then(|d| {
            args.environment_variables
                .get("GITHUB_ACTION_PATH")
                .and_then(|p| step::action_root(&d, p))
        }),
    };
    if let Some(a) = action {
        let host = job
            .host_path(&a.root)
            .context("action directory without a host path")?;
        let tree = cas
            .import_dir(&host)
            .with_context(|| format!("hash action {}", host.display()))?;
        let h = Hash::of(&tree_to_json(&tree)?);
        job.action_trees.insert(a.root.clone(), tree);
        mounts.push(Mount {
            src: host,
            dst: PathBuf::from(&a.root),
            writable: false,
        });
        key_extra.push(format!("quod:tree {} {h}", a.root));
    }
    key_extra.push(job.identity_key());

    let env = finish_env(env, &mut taint);
    let key_env = finish_env(key_env, &mut Vec::new());
    Ok(Surface {
        kind,
        env,
        key_env,
        cmd,
        mounts,
        egress: kind == SurfaceKind::Network,
        withheld,
        absent,
        files,
        key_extra,
        masks: c.masks,
        classified: c.names,
        secret_values: c.strong,
        short_secret: c.short,
        short_strong: c.short_strong,
        temp: Some(temp),
        taint,
    })
}

/// The runner surface (see the module docs).
pub fn runner(job: &Job, args: &ScriptStepArgs) -> anyhow::Result<Surface> {
    let (env, mut taint) = base_env(job, args, false);
    // Nothing of this surface is stored; its secrets mask what it reports.
    let c = secrets::classify_with(
        &env,
        job.provenance(),
        job.known_secrets(),
        &job.set_by_steps,
        &job.image_env(),
    );
    let env = finish_env(env, &mut taint);
    let mut cmd = vec![args.entry_point.clone()];
    cmd.extend(args.entry_point_args.iter().cloned());
    let mut mounts: Vec<Mount> = Vec::new();
    for m in &job.system_mounts {
        if m.target_volume_path
            .trim_end_matches('/')
            .ends_with("docker.sock")
            || m.source_volume_path
                .trim_end_matches('/')
                .ends_with("docker.sock")
        {
            // The docker socket is root on the host: never in a sandbox.
            continue;
        }
        let src = PathBuf::from(&m.source_volume_path);
        if !src.exists() {
            tracing::info!(target: "quod", src = %src.display(), "runner mount without a source; skipped");
            continue;
        }
        mounts.push(Mount {
            src,
            dst: PathBuf::from(&m.target_volume_path),
            writable: !m.read_only,
        });
    }
    // Name resolution for the network the step now has.
    if job.rootfs.is_some() {
        for f in ["/etc/resolv.conf", "/etc/hosts"] {
            if Path::new(f).is_file() {
                mounts.push(Mount {
                    src: f.into(),
                    dst: f.into(),
                    writable: false,
                });
            }
        }
    }
    Ok(Surface {
        kind: SurfaceKind::Runner,
        key_env: env.clone(),
        env,
        cmd,
        mounts,
        egress: true,
        withheld: Vec::new(),
        absent: Vec::new(),
        files: Vec::new(),
        key_extra: Vec::new(),
        masks: c.masks,
        classified: c.names,
        secret_values: c.strong,
        short_secret: c.short,
        short_strong: c.short_strong,
        temp: None,
        taint,
    })
}

/// What a hermetic or network step left in its command files: appended to
/// the runner's files for this step (the runner reads them after the hook)
/// and, unless it holds a secret value, returned for the result, its blob
/// in `cas` (the step's staging store: it reaches the shared store only
/// with a recorded result). A directory the step wrote into, a rewritten
/// runner file, or a secret in a command file (in any encoding) is a taint.
pub fn hand_back(
    files: &[PinnedFile],
    cas: &Cas,
    masks: &[Vec<u8>],
) -> anyhow::Result<(Vec<FileCommand>, Vec<String>)> {
    let mut out = Vec::new();
    let mut taint = Vec::new();
    for f in files {
        if f.dir {
            let entries: Vec<_> = std::fs::read_dir(&f.host)?.collect::<Result<_, _>>()?;
            if !entries.is_empty() {
                taint.push(format!(
                    "wrote into {}, a directory the quod runner does not record",
                    f.name
                ));
                if let Some(r) = &f.runner {
                    let st = std::process::Command::new("cp")
                        .arg("-a")
                        .arg(format!("{}/.", f.host.display()))
                        .arg(r)
                        .status()
                        .context("cp")?;
                    if !st.success() {
                        anyhow::bail!("copying {} back to the runner failed", f.name);
                    }
                }
            }
            continue;
        }
        let all = std::fs::read(&f.host).with_context(|| format!("read {}", f.host.display()))?;
        if !all.starts_with(&f.initial) {
            taint.push(format!(
                "rewrote {}, which the runner had written to",
                f.name
            ));
            continue;
        }
        let bytes = &all[f.initial.len()..];
        if bytes.is_empty() {
            continue;
        }
        if let Some(r) = &f.runner {
            append(r, bytes)?;
        }
        if secrets::holds(bytes, masks) {
            taint.push(format!("{} carries a secret value", f.name));
            continue;
        }
        // Stored with the meaning the runner reads, not the step's random
        // heredoc delimiters, so the same output is the same blob.
        out.push(FileCommand {
            name: f.name.clone(),
            h: cas.put(&trust::canonical_commands(&f.name, bytes))?,
        });
    }
    out.sort();
    Ok((out, taint))
}

/// What a step left in its own RUNNER_TEMP besides its script and command
/// files: on GitHub, later steps would find it there.
pub fn temp_leftovers(temp: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(temp) else {
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != FILE_COMMANDS_DIR && !n.starts_with(SCRIPT))
        .collect();
    out.sort();
    out
}

/// A listing of the runner's RUNNER_TEMP on the host (path, size, mtime),
/// minus the runner's own bookkeeping: to see what a runner-surface step
/// left for later steps.
pub fn temp_snapshot(host_temp: &Path) -> BTreeMap<String, (u64, i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    const RUNNER_OWNED: &[&str] = &[
        FILE_COMMANDS_DIR,
        "_runner_hook_responses",
        "_github_workflow",
    ];
    let mut out = BTreeMap::new();
    let mut stack = vec![host_temp.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let rel = p
                .strip_prefix(host_temp)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if RUNNER_OWNED
                .iter()
                .any(|o| rel == *o || rel.starts_with(&format!("{o}/")))
            {
                continue;
            }
            let Ok(m) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            out.insert(rel, (m.size(), m.mtime(), m.mtime_nsec()));
            if m.is_dir() {
                stack.push(p);
            }
        }
    }
    out
}

/// The values a command file (GITHUB_ENV, GITHUB_OUTPUT, GITHUB_STATE)
/// sets: each `NAME=value`'s value and each heredoc's body.
pub fn command_values(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    let mut out = Vec::new();
    while let Some(line) = lines.next() {
        let line = line.trim_end_matches('\r');
        let eq = line.find('=');
        if let Some(h) = line.find("<<").filter(|h| eq.is_none_or(|e| *h < e)) {
            let d = &line[h + 2..];
            let mut body = Vec::new();
            for l in lines.by_ref() {
                let l = l.trim_end_matches('\r');
                if l == d {
                    break;
                }
                body.push(l);
            }
            out.push(body.join("\n"));
        } else if let Some(e) = eq {
            out.push(line[e + 1..].to_string());
        }
    }
    out
}

/// A pinned command file's value (GITHUB_ENV, GITHUB_OUTPUT, GITHUB_STATE)
/// of `MIN_SECRET_LEN` bytes or more that one of `secrets` holds: a part a
/// step cut out of a secret and hands on to later steps. The file's name.
pub fn part_handed_on(files: &[PinnedFile], secrets: &[&str]) -> Option<String> {
    files
        .iter()
        .filter(|f| {
            !f.dir
                && matches!(
                    f.name.as_str(),
                    "GITHUB_ENV" | "GITHUB_OUTPUT" | "GITHUB_STATE"
                )
        })
        .find(|f| {
            let all = std::fs::read(&f.host).unwrap_or_default();
            let added = all.get(f.initial.len()..).unwrap_or(&[]);
            command_values(added).iter().any(|v| {
                let v = v.trim();
                v.len() >= secrets::MIN_SECRET_LEN && secrets.iter().any(|s| s.contains(v))
            })
        })
        .map(|f| f.name.clone())
}

/// The names a GITHUB_ENV file sets, as the runner reads it: `NAME=value`
/// lines and `NAME<<DELIMITER` heredocs (skipped to their delimiter).
pub fn env_file_names(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    let mut out = Vec::new();
    while let Some(line) = lines.next() {
        let line = line.trim_end_matches('\r');
        let (name, delim) = match (line.find('='), line.find("<<")) {
            (Some(e), Some(h)) if h < e => (&line[..h], Some(&line[h + 2..])),
            (Some(e), _) => (&line[..e], None),
            (None, Some(h)) => (&line[..h], Some(&line[h + 2..])),
            (None, None) => continue,
        };
        if !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            out.push(name.to_string());
        }
        if let Some(d) = delim {
            for l in lines.by_ref() {
                if l.trim_end_matches('\r') == d {
                    break;
                }
            }
        }
    }
    out
}

pub fn append(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .with_context(|| format!("append {}", path.display()))
}

/// BLAKE3 of a file's content, streamed.
pub fn hash_file(path: &Path) -> anyhow::Result<Hash> {
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(Hash::from_blake3(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sandbox_hostname_resolves_locally() {
        assert!(
            HOSTS
                .lines()
                .any(|l| l.split_whitespace().nth(1) == Some(quod_sandbox::HOSTNAME))
        );
    }

    #[test]
    fn hand_back_appends_only_what_the_step_added() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let pinned = |name: &str, given: &[u8], wrote: &[u8]| {
            let host = tmp.path().join(format!("{name}.pinned"));
            let runner = tmp.path().join(format!("{name}.runner"));
            std::fs::write(&runner, given).unwrap();
            std::fs::write(&host, wrote).unwrap();
            PinnedFile {
                name: name.into(),
                host,
                runner: Some(runner),
                dir: false,
                initial: given.to_vec(),
            }
        };
        let files = [
            pinned("GITHUB_OUTPUT", b"", b"x=1\n"),
            pinned("GITHUB_ARTIFACTS_LIST", b"[a]\n", b"[a]\n"),
            pinned("GITHUB_ENV", b"", b""),
        ];
        let (cmds, taint) = hand_back(&files, &cas, &[]).unwrap();
        assert!(taint.is_empty());
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "GITHUB_OUTPUT");
        assert_eq!(cas.get(&cmds[0].h).unwrap().unwrap(), b"x=1\n");
        assert_eq!(
            std::fs::read(files[0].runner.as_ref().unwrap()).unwrap(),
            b"x=1\n"
        );
        assert_eq!(
            std::fs::read(files[1].runner.as_ref().unwrap()).unwrap(),
            b"[a]\n"
        );
        // A step that rewrote what the runner gave it is not recorded.
        let rewrote = [pinned("GITHUB_ARTIFACTS_LIST", b"[a]\n", b"[b]\n")];
        let (_, taint) = hand_back(&rewrote, &cas, &[]).unwrap();
        assert_eq!(taint.len(), 1, "{taint:?}");
        // A secret in a command file reaches the runner, never the store.
        let secret = [pinned("GITHUB_OUTPUT", b"", b"t=hunter2hunter2\n")];
        let (cmds, taint) =
            hand_back(&secret, &cas, &secrets::masks_for("hunter2hunter2")).unwrap();
        assert!(cmds.is_empty() && taint.len() == 1, "{taint:?}");
        assert_eq!(
            std::fs::read(secret[0].runner.as_ref().unwrap()).unwrap(),
            b"t=hunter2hunter2\n"
        );
    }

    /// A secret in a GITHUB_OUTPUT heredoc, or base64-encoded in the step
    /// summary, keeps the file out of the store like a plain one.
    #[test]
    fn a_secret_in_any_command_file_or_encoding_is_not_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let secret = "hunter2/hunter 2";
        let masks = secrets::masks_for(secret);
        let pinned = |name: &str, wrote: &str| {
            let host = tmp.path().join(format!("{name}.pinned"));
            std::fs::write(&host, wrote).unwrap();
            PinnedFile {
                name: name.into(),
                host,
                runner: None,
                dir: false,
                initial: vec![],
            }
        };
        // base64("hunter2/hunter 2") and base64("user:hunter2/hunter 2").
        for (name, wrote) in [
            ("GITHUB_OUTPUT", "pw<<EOF_1\nhunter2/hunter 2\nEOF_1\n"),
            ("GITHUB_STEP_SUMMARY", "aHVudGVyMi9odW50ZXIgMg==\n"),
            ("GITHUB_OUTPUT", "auth=dXNlcjpodW50ZXIyL2h1bnRlciAy\n"),
        ] {
            let (cmds, taint) = hand_back(&[pinned(name, wrote)], &cas, &masks).unwrap();
            assert!(
                cmds.is_empty() && taint.len() == 1,
                "{name} {wrote:?}: {taint:?}"
            );
        }
    }

    #[test]
    fn command_file_values() {
        assert_eq!(
            command_values(b"A=1\nB<<EOF_x\nl1\nl2\nEOF_x\nC=x=y\n"),
            ["1", "l1\nl2", "x=y"]
        );
    }

    #[test]
    fn env_file_names_as_the_runner_reads_them() {
        assert_eq!(
            env_file_names(b"A=1\nDB_PASS<<EOF_x\nX=not a name\nEOF_x\nB=x=y\n\nC<<D\r\nD\r\n"),
            ["A", "DB_PASS", "B", "C"]
        );
    }

    #[test]
    fn temp_leftovers_ignore_the_script_and_command_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(FILE_COMMANDS_DIR)).unwrap();
        std::fs::write(tmp.path().join("quod-step.sh"), "").unwrap();
        assert!(temp_leftovers(tmp.path()).is_empty());
        std::fs::write(tmp.path().join("go1.27.tar.gz"), "").unwrap();
        assert_eq!(temp_leftovers(tmp.path()), ["go1.27.tar.gz"]);
    }

    #[test]
    fn file_hash_is_content() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("f");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(hash_file(&p).unwrap(), Hash::of(b"abc"));
    }
}
