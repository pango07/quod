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
//!   placeholders, and values whose names say
//!   they are secrets are given but salted in the key and masked in
//!   everything stored.
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
use sp_cas::{Cas, Hash};
use sp_manifest::FileCommand;
use sp_sandbox::Mount;

use crate::checkout::tree_to_json;
use crate::job::{Job, STATE_SUBDIR};
use crate::protocol::ScriptStepArgs;
use crate::secrets;
use crate::step::{self, Kind, Step};
use crate::trust;

/// Inside the hermetic RUNNER_TEMP: the step's script.
const SCRIPT: &str = "stillpass-step";

/// The value a withheld variable has in a keyed step: set, so presence
/// checks behave as on GitHub, and useless as a token, a sha or an id.
pub const WITHHELD: &str = "stillpass-withheld";

/// What a placeholder leaves in a step's result: `WITHHELD` (withheld names,
/// ACTIONS_* tokens) and the unreachable ACTIONS_* service address. A
/// result that holds one is not what GitHub would give (`job::run_step`
/// runs the step again with the runner's environment, never records it).
pub const PLACEHOLDER_MARKS: &[&str] = &[WITHHELD, "stillpass-unreachable"];

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
const RESOLV_CONF: &str = "nameserver 192.0.2.1\noptions attempts:1 timeout:1\n";
/// The sandbox's own name (`sp_sandbox::HOSTNAME`) resolves locally, as a
/// container's does: a test that looks itself up must not reach for DNS.
const HOSTS: &str =
    "127.0.0.1\tlocalhost\n127.0.1.1\tstillpass\n::1\tlocalhost ip6-localhost ip6-loopback\n";

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
    /// `WITHHELD`): a file naming one means the step may have needed it.
    pub absent: Vec<String>,
    /// Hermetic and network: the command files at their fixed paths, to
    /// hand back to the runner after the step.
    pub files: Vec<PinnedFile>,
    /// Hermetic: the content identity of runner-provided files the step
    /// executes, appended to the key's command.
    pub key_extra: Vec<String>,
    /// Secret values the step was given: masked in stored logs; outputs or
    /// command files containing one are not stored.
    pub masks: Vec<Vec<u8>>,
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

/// The variables Stillpass pins so tool state lives in the observed tree.
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
/// image, the runner's own defaults, or the very value Stillpass pins
/// (setup-go exports GOTOOLCHAIN=local)?
fn workflow_sets(job: &Job, args: &ScriptStepArgs, name: &str, pin: &str) -> bool {
    let runner_default = |v: &str| match name {
        "HOME" => v == "/github/home",
        "RUNNER_TOOL_CACHE" => step::tool_dir(job).is_some_and(|t| t == v),
        _ => false,
    };
    [job.env.get(name), args.environment_variables.get(name)]
        .into_iter()
        .flatten()
        .any(|v| !runner_default(v) && v != pin)
}

/// The environment every surface starts from: the image's, the docker
/// defaults (GITHUB_ACTIONS, CI), the job container's, the step's, PATH
/// with the runner's prepends, and Stillpass's pins; and the taint when
/// the workflow set something Stillpass would pin.
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
    env.extend(job.env.clone());
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
                "the workflow sets {name}, which the stillpass runner would pin"
            ));
        } else {
            env.insert(name.into(), value);
        }
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
    // goindex=0), the ones `stillpass run --warm` merges.
    let knobs = sp_key::Toolchain::Go
        .profile()
        .warm
        .as_ref()
        .map_or(&[][..], |w| w.knobs);
    match knobs
        .iter()
        .try_for_each(|k| sp_key::merge_knob(&mut merged, k))
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
    env.retain(|k, v| !(step::secret_like(k) || step::looks_secret(v)));
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
                    "the runner gave {k} as a non-empty directory; the stillpass runner cannot key it"
                );
            }
        } else {
            let given = match &runner {
                Some(r) => std::fs::read(r).unwrap_or_default(),
                None => Vec::new(),
            };
            if !given.is_empty() {
                key_extra.push(format!("stillpass:given {k} {}", Hash::of(&given)));
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
            absent.push(k);
        } else if let Some(p) = step::placeholder(&k) {
            env.insert(k.clone(), p.to_string());
            withheld.push(k);
        }
    }
    withheld.sort();
    absent.sort();

    // Secrets the step is given: by a telling name, or by a value that looks
    // like a secret whatever its name (the protocol gives the hook no mask
    // list). Salted in the key, masked in everything stored.
    let mut masks = Vec::new();
    let mut key_env = env.clone();
    for (k, v) in &env {
        // The withheld placeholder is no secret, whatever its name.
        if !v.is_empty() && v != WITHHELD && (step::secret_like(k) || step::looks_secret(v)) {
            masks.extend(secrets::masks_for(v));
            key_env.insert(k.clone(), secrets::salted(salt, k, v));
        }
    }
    masks.sort_by_key(|b| std::cmp::Reverse(b.len()));
    masks.dedup();

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
        key_extra.push(format!("stillpass:file {} {h}", args.entry_point));
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
        key_extra.push(format!("stillpass:tree {} {h}", a.root));
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
        masks,
        temp: Some(temp),
        taint,
    })
}

/// The runner surface (see the module docs).
pub fn runner(job: &Job, args: &ScriptStepArgs) -> anyhow::Result<Surface> {
    let (env, mut taint) = base_env(job, args, false);
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
            tracing::info!(target: "stillpass", src = %src.display(), "runner mount without a source; skipped");
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
        masks: Vec::new(),
        temp: None,
        taint,
    })
}

/// What a hermetic or network step left in its command files: appended to
/// the runner's files for this step (the runner reads them after the hook)
/// and, unless it holds a secret value, returned for the result. A
/// directory the step wrote into, a rewritten runner file, or a secret in a
/// command file is a taint.
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
                    "wrote into {}, a directory the stillpass runner does not record",
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
        if secrets::mask(bytes, masks) != bytes {
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
                .any(|l| l.split_whitespace().nth(1) == Some(sp_sandbox::HOSTNAME))
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

    #[test]
    fn temp_leftovers_ignore_the_script_and_command_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(FILE_COMMANDS_DIR)).unwrap();
        std::fs::write(tmp.path().join("stillpass-step.sh"), "").unwrap();
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
