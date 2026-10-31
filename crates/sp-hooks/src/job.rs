//! One job: created by `prepare_job`, driven step by step by
//! `run_script_step`, closed by `cleanup_job`. Lives at `<state>/jobs/<id>`.
//!
//! The workspace is a tree in the store, never a directory on disk: the
//! checkout sets it, every stored step's outputs are applied to it, and a
//! replay is `tree.with_outputs`. Between jobs the `.stillpass/` module and
//! tool caches persist per workspace path, the way a self-hosted runner's
//! `_work` directory does.
//!
//! Scope. Everything a job can be influenced by from earlier jobs (the
//! ledger, the persisted workspace, the warm build cache, the quarantine)
//! lives under `<state>/scopes/<repository>/<trust>`: `trusted` for the
//! repository's own events, `fork-<owner_repo>` for a pull request from a
//! fork (each fork apart), `unverified` when the event cannot be read. A
//! fork's job never reads what a trusted job recorded, and the reverse.
//! The store (content-addressed, append-only) and the image cache are
//! shared.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use sp_cas::{Cas, Entry, Hash, Kind as EntryKind, Tree};
use sp_key::{Image, ImageKind, Platform, StepSpec, Toolchain, step_key};
use sp_ledger::{Ledger, Observed, RunFacts};
use sp_manifest::{Manifest, StepResult};
use sp_replay::fmt_duration;

use crate::checkout::{self, tree_from_json, tree_to_json};
use crate::exec::{self, Exec, Executed, Keep};
use crate::protocol::{self, Container, Input, ScriptStepArgs, StateResponse};
use crate::rootfs::{OciRootfs, Rootfs, RootfsProvider};
use crate::scan;
use crate::secrets;
use crate::step::{self, Kind, Step};
use crate::surface::{self, Surface};
use crate::trust::{Mode, Policy};
use sp_trust::{Decision, Reason, Replayed, Step as TrustStep, Trust, Verdict};

/// Where Stillpass keeps a workspace's own state, under GITHUB_WORKSPACE:
/// HOME, GOPATH, the module cache, the tool cache. Observed like the rest
/// of the tree.
pub const STATE_SUBDIR: &str = ".stillpass";

/// The parts of `.stillpass/` that persist from job to job on a workspace
/// path (the module cache and the tool cache: what a self-hosted runner
/// keeps between jobs). HOME and GOPATH are per job, as `/github/home` is.
pub const PERSIST: &[&str] = &["gomod", "tool"];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Totals {
    pub steps: u32,
    pub executed: u32,
    pub replayed: u32,
    pub tainted: u32,
    pub would_replay: u32,
    /// Observe mode: recorded time of the steps replay mode would have
    /// replayed. Replay mode: recorded time minus replay time, summed.
    pub saved_ms: u64,
    pub audits: u32,
    pub divergences: u32,
    pub exec_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    /// The job container's image as the workflow named it.
    pub image: String,
    pub is_alpine: bool,
    /// GITHUB_WORKSPACE inside the container.
    pub working_directory: String,
    /// The container env from the workflow (`container.env`) plus what the
    /// runner adds (HOME).
    pub env: BTreeMap<String, String>,
    pub system_mounts: Vec<protocol::Mount>,
    pub user_mounts: Vec<protocol::Mount>,
    /// How many steps ran so far.
    pub steps: u32,
    /// The unpacked image; `None` in host-root mode (tests).
    pub rootfs: Option<Rootfs>,
    /// `--user UID[:GID]` from `container.options`; root otherwise, as under
    /// docker.
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    /// `owner/repo` (lowercase) from the event, and the trust scope.
    pub repository: String,
    pub scope_name: String,
    /// Set by the first step: `<state>/scopes/<repo>/<trust>`.
    pub scope: Option<PathBuf>,
    /// Earlier steps' writes outside the workspace, newest first: layers
    /// under every later step's root.
    pub root_layers: Vec<PathBuf>,
    /// Earlier secret-bearing steps' workspace writes, newest first.
    pub ws_layers: Vec<PathBuf>,
    /// Why every later step of the job is tainted.
    pub dirty: Vec<String>,
    /// The checkout fell back to the real action: its post step (credential
    /// cleanup) must run too.
    pub checkout_fallback: bool,
    pub totals: Totals,
    pub started_ms: u64,
    /// Content hashes of runner-provided files (node in the externals),
    /// once per job: the externals mount is read-only for every step.
    #[serde(default)]
    pub hashes: BTreeMap<String, Hash>,
    /// This host's class as step keys pin it (`sp_key::Platform`, probed
    /// once at prepare_job the way `stillpass run` does).
    #[serde(default)]
    pub platform: Option<Platform>,
    /// The trees of action directories bound into this step (for the scan).
    #[serde(skip)]
    pub action_trees: BTreeMap<String, Tree>,
    #[serde(skip)]
    dir: PathBuf,
    #[serde(skip)]
    state_dir: PathBuf,
}

impl Job {
    /// The job container's image, unpacked; `None` in host-root mode.
    pub fn prepare_rootfs(
        state_dir: &Path,
        image: &str,
        policy: &Policy,
    ) -> anyhow::Result<Option<Rootfs>> {
        if policy.host_root {
            return Ok(None);
        }
        let t = Instant::now();
        let r = OciRootfs::new(state_dir.join("rootfs"))
            .prepare(image)
            .with_context(|| format!("prepare image {image}"))?;
        tracing::info!(target: "stillpass", image = %image, digest = %r.digest, ms = t.elapsed().as_millis() as u64, "rootfs");
        Ok(Some(r))
    }

    pub fn create(
        state_dir: &Path,
        c: &Container,
        rootfs: Option<Rootfs>,
        platform: Platform,
    ) -> anyhow::Result<Job> {
        let id = ulid::Ulid::generate().to_string();
        let dir = jobs_dir(state_dir).join(&id);
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let image = c.image.clone().context("job container without an image")?;
        let (uid, gid) = user_option(c.create_options.as_deref())?;
        let mut job = Job {
            id,
            image,
            is_alpine: rootfs.as_ref().is_some_and(|r| r.is_alpine),
            working_directory: c
                .working_directory
                .clone()
                .context("job container without a working directory")?,
            env: c.environment_variables.clone(),
            system_mounts: c.system_mount_volumes.clone(),
            user_mounts: c.user_mount_volumes.clone(),
            steps: 0,
            rootfs,
            uid,
            gid,
            repository: String::new(),
            scope_name: String::new(),
            scope: None,
            root_layers: Vec::new(),
            ws_layers: Vec::new(),
            dirty: Vec::new(),
            checkout_fallback: false,
            totals: Totals::default(),
            started_ms: now_ms(),
            hashes: BTreeMap::new(),
            platform: Some(platform),
            action_trees: BTreeMap::new(),
            dir,
            state_dir: state_dir.to_path_buf(),
        };
        if !job.user_mounts.is_empty() {
            job.dirty.push(format!(
                "user volume {} is not observed",
                job.user_mounts[0].target_volume_path
            ));
        }
        job.save()?;
        Ok(job)
    }

    /// Decides the job's scope from the event the runner wrote (on the
    /// first step: the runner writes event.json before each step) and
    /// starts its workspace there: where the last job on this path in this
    /// scope left it (its persisted `.stillpass/` state), or, for the first
    /// job, the same parts from whatever is on disk (a seeded module
    /// cache), and nothing else: HOME and the rest are per job.
    fn ensure_scope(&mut self, cas: &Cas) -> anyhow::Result<()> {
        if self.scope.is_some() {
            return Ok(());
        }
        let event = step::workflow_dir(self)
            .and_then(|d| self.host_path(&format!("{d}/event.json")))
            .and_then(|p| std::fs::read(p).ok());
        let (repository, scope_name) = scope_of(event.as_deref());
        let scope = self
            .state_dir
            .join("scopes")
            .join(slug(if repository.is_empty() {
                "unknown"
            } else {
                &repository
            }))
            .join(&scope_name);
        std::fs::create_dir_all(&scope)?;
        self.repository = repository;
        self.scope_name = scope_name;
        self.scope = Some(scope);
        let tree = match std::fs::read(self.workspace_slot()) {
            Ok(b) => persisted(&tree_from_json(&b)?),
            Err(_) => match self.host_path(&self.working_directory) {
                Some(h) => import_persisted(cas, &h)?,
                None => Tree::from_leaves([]),
            },
        };
        self.save_tree(&with_state_dirs(cas, &tree)?)?;
        tracing::info!(target: "stillpass", job = %self.id, repository = %self.repository, scope = %self.scope_name, "scope");
        Ok(())
    }

    fn scope(&self) -> &Path {
        self.scope.as_deref().expect("scope set by the first step")
    }

    pub fn load(state_dir: &Path, id: &str) -> anyhow::Result<Job> {
        if id.is_empty() || id.contains('/') || id.contains("..") {
            bail!("bad job id {id:?}");
        }
        let dir = jobs_dir(state_dir).join(id);
        let p = dir.join("job.json");
        let mut job: Job = serde_json::from_slice(
            &std::fs::read(&p).with_context(|| format!("read {}", p.display()))?,
        )?;
        job.dir = dir;
        job.state_dir = state_dir.to_path_buf();
        Ok(job)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let p = self.dir.join("job.json");
        let tmp = self.dir.join("job.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn tree_path(&self) -> PathBuf {
        self.dir.join("tree.json")
    }

    /// The persisted workspace for this GITHUB_WORKSPACE path, in scope.
    fn workspace_slot(&self) -> PathBuf {
        self.scope()
            .join("workspaces")
            .join(slug(&self.working_directory))
            .join("tree.json")
    }

    pub fn load_tree(&self) -> anyhow::Result<Tree> {
        tree_from_json(&std::fs::read(self.tree_path())?)
    }

    pub fn save_tree(&self, tree: &Tree) -> anyhow::Result<()> {
        let tmp = self.dir.join("tree.json.tmp");
        std::fs::write(&tmp, tree_to_json(tree)?)?;
        std::fs::rename(&tmp, self.tree_path())?;
        Ok(())
    }

    /// End of job: the workspace persists for the next job on this path in
    /// this scope (never what a secret-bearing step wrote: that is in the
    /// job's layers), the job directory goes.
    pub fn cleanup(self, policy: &Policy) -> anyhow::Result<()> {
        if self.scope.is_some()
            && let Ok(tree) = self.load_tree()
        {
            let slot = self.workspace_slot();
            std::fs::create_dir_all(slot.parent().unwrap())?;
            let tmp = slot.with_extension("json.tmp");
            std::fs::write(&tmp, tree_to_json(&persisted(&tree))?)?;
            std::fs::rename(&tmp, &slot)?;
        }
        let total = now_ms().saturating_sub(self.started_ms);
        let t = &self.totals;
        eprintln!(
            "stillpass: job {}: {} steps, {} executed, {} replayed, {} tainted, {} would replay, saved {}, {} audits, {} divergences, job {}",
            self.id,
            t.steps,
            t.executed,
            t.replayed,
            t.tainted,
            t.would_replay,
            fmt_duration(Duration::from_millis(t.saved_ms)),
            t.audits,
            t.divergences,
            fmt_duration(Duration::from_millis(total)),
        );
        tracing::info!(
            target: "stillpass",
            job = %self.id,
            scope = %self.scope_name,
            steps = t.steps,
            executed = t.executed,
            replayed = t.replayed,
            tainted = t.tainted,
            would_replay = t.would_replay,
            saved_ms = t.saved_ms,
            audits = t.audits,
            divergences = t.divergences,
            exec_ms = t.exec_ms,
            total_ms = total,
            "job"
        );
        if !policy.keep_jobs {
            sp_cas::remove_any(&self.dir)?;
        }
        Ok(())
    }

    /// The host path for a path inside the container, through the runner's
    /// mounts (longest target prefix wins). `None` when nothing mounts it,
    /// and for anything with a `..` component: these paths come from the
    /// step's environment, which the workflow can set, and the hook writes
    /// on the host as root.
    pub fn host_path(&self, container_path: &str) -> Option<PathBuf> {
        if !container_path.starts_with('/')
            || container_path
                .split('/')
                .any(|c| c == ".." || c == "." && container_path != "/")
        {
            return None;
        }
        let mut best: Option<(&protocol::Mount, &str)> = None;
        for m in self.system_mounts.iter().chain(&self.user_mounts) {
            let t = m.target_volume_path.trim_end_matches('/');
            let rest = if container_path == t {
                Some("")
            } else {
                container_path
                    .strip_prefix(t)
                    .and_then(|r| r.strip_prefix('/'))
            };
            if let Some(rest) = rest
                && best.is_none_or(|(b, _)| b.target_volume_path.len() < m.target_volume_path.len())
            {
                best = Some((m, rest));
            }
        }
        best.map(|(m, rest)| {
            let src = PathBuf::from(&m.source_volume_path);
            if rest.is_empty() { src } else { src.join(rest) }
        })
    }

    /// The host path of one of the runner's per-step command files: only a
    /// file directly in `<RUNNER_TEMP>/_runner_file_commands/`.
    pub fn command_file(&self, container_path: &str) -> Option<PathBuf> {
        let t = step::temp_dir(self)?;
        let name = container_path.strip_prefix(&format!("{t}/_runner_file_commands/"))?;
        if name.is_empty() || name.contains('/') {
            return None;
        }
        self.host_path(container_path)
    }

    /// The step key's image pin: the image digest, or the host in
    /// host-root mode.
    pub fn image_key(&self) -> Hash {
        match &self.rootfs {
            Some(r) => r.key(),
            None => Hash::of(format!("host-root {}", self.image).as_bytes()),
        }
    }

    /// The step key's environment pin: the job image (or the host root) by
    /// digest, on this host's class. A rootfs pin, like `stillpass run
    /// --rootfs`; the host-root fallback is a host pin and never travels.
    pub fn key_image(&self) -> anyhow::Result<Image> {
        let kind = if self.rootfs.is_some() {
            ImageKind::Rootfs
        } else {
            ImageKind::Host
        };
        let platform = self
            .platform
            .clone()
            .context("the job has no host class (prepared by an older stillpass)")?;
        Ok(Image::new(kind, self.image_key(), platform))
    }

    /// Who the step runs as, for the key: uid, gid and the passwd and group
    /// it sees (the fixed ones the sandbox binds, or the image's own).
    pub fn identity_key(&self) -> String {
        match self.uid {
            Some(u) => {
                let g = self.gid.unwrap_or(u);
                let (p, gr) = sp_sandbox::identity_files(u, g);
                format!(
                    "stillpass:identity uid={u} gid={g} passwd={}",
                    Hash::of(format!("{p}\0{gr}").as_bytes())
                )
            }
            None => "stillpass:identity uid=0 gid=0 passwd=image".to_string(),
        }
    }

    /// The content hash of a runner-provided file, once per job.
    pub fn file_hash(&mut self, host: &Path) -> anyhow::Result<Hash> {
        let k = host.to_string_lossy().into_owned();
        if let Some(h) = self.hashes.get(&k) {
            return Ok(*h);
        }
        let h = surface::hash_file(host)?;
        self.hashes.insert(k, h);
        Ok(h)
    }

    /// Appends a line to the step's GITHUB_STEP_SUMMARY, if the step has one.
    fn summary(&self, env: &BTreeMap<String, String>, line: &str) {
        let Some(host) = env
            .get("GITHUB_STEP_SUMMARY")
            .and_then(|p| self.command_file(p))
        else {
            return;
        };
        let _ = surface::append(&host, format!("{line}\n").as_bytes());
    }

    fn warm_gocache(&self) -> PathBuf {
        self.scope()
            .join("warm")
            .join(self.image_key().hex())
            .join("gocache")
    }

    /// GOCACHE for steps that are not recorded: the job's own, gone at
    /// cleanup, never the scope's warm cache.
    fn scratch_gocache(&self) -> PathBuf {
        self.dir.join("gocache")
    }
}

pub(crate) fn jobs_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("jobs")
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The repository (lowercase `owner/repo`) and the trust scope of a job,
/// from its event payload.
fn scope_of(event: Option<&[u8]>) -> (String, String) {
    let Some(v) = event.and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok()) else {
        return (String::new(), "unverified".into());
    };
    let name = |v: &serde_json::Value| {
        v.get("full_name")
            .and_then(|n| n.as_str())
            .map(str::to_ascii_lowercase)
    };
    let Some(repo) = v.get("repository").and_then(name) else {
        return (String::new(), "unverified".into());
    };
    let scope = match v.get("pull_request") {
        Some(pr) => match pr.get("head").and_then(|h| h.get("repo")).and_then(name) {
            Some(head) if head == repo => "trusted".to_string(),
            Some(head) => format!("fork-{}", slug(&head)),
            // A deleted fork: no head repository to trust.
            None => "fork-unknown".to_string(),
        },
        None => "trusted".to_string(),
    };
    (repo, scope)
}

/// `--user UID[:GID]` (or `-u`) in docker create options. Other options are
/// not honoured; they are logged so the omission is visible.
fn user_option(create_options: Option<&str>) -> anyhow::Result<(Option<u32>, Option<u32>)> {
    let Some(opts) = create_options else {
        return Ok((None, None));
    };
    let words: Vec<&str> = opts.split_whitespace().collect();
    let (mut uid, mut gid) = (None, None);
    let mut ignored = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let value = if let Some(v) = w.strip_prefix("--user=") {
            Some(v.to_string())
        } else if w == "--user" || w == "-u" {
            i += 1;
            words.get(i).map(|v| v.to_string())
        } else {
            ignored.push(w);
            None
        };
        if let Some(v) = value {
            let (u, g) = match v.split_once(':') {
                Some((u, g)) => (u, Some(g)),
                None => (v.as_str(), None),
            };
            uid = Some(u.parse::<u32>().with_context(|| {
                format!("container option --user {v:?}: only numeric uids are supported")
            })?);
            gid = g
                .map(|g| {
                    g.parse::<u32>().with_context(|| {
                        format!("container option --user {v:?}: only numeric gids are supported")
                    })
                })
                .transpose()?;
        }
        i += 1;
    }
    if !ignored.is_empty() {
        tracing::warn!(options = ?ignored, "container options not honoured by the stillpass sandbox");
    }
    Ok((uid, gid))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// What of a finished job's tree the next job on the path starts from:
/// the `PERSIST` parts of `.stillpass/`, nothing else.
fn persisted(tree: &Tree) -> Tree {
    let keep = |p: &str| {
        PERSIST.iter().any(|d| {
            let dir = format!("{STATE_SUBDIR}/{d}");
            p == dir || p.starts_with(&format!("{dir}/"))
        })
    };
    Tree::from_leaves(tree.iter().filter(|e| keep(&e.p)).cloned())
}

/// Go's telemetry mode in the pinned HOME. In its default ("local") mode
/// every go command updates date-stamped counter files under HOME, which
/// would make each Go step an input of the next and every day a miss.
const GO_TELEMETRY_MODE: &str = "home/.config/go/telemetry/mode";

/// The `PERSIST` parts of `<host>/.stillpass/`, as a workspace tree.
fn import_persisted(cas: &Cas, host: &Path) -> anyhow::Result<Tree> {
    let mut leaves = Vec::new();
    for d in PERSIST {
        let sub = host.join(STATE_SUBDIR).join(d);
        if !sub.is_dir() {
            continue;
        }
        let base = format!("{STATE_SUBDIR}/{d}");
        for e in cas.import_dir(&sub)?.iter() {
            let mut e = e.clone();
            e.p = if e.p == "." {
                base.clone()
            } else {
                format!("{base}/{}", e.p)
            };
            leaves.push(e);
        }
    }
    Ok(Tree::from_leaves(leaves))
}

/// The tree with Stillpass's state directories present (empty if new), so
/// HOME exists and the module cache can be sealed from the first step, and
/// with Go's telemetry off in HOME.
fn with_state_dirs(cas: &Cas, tree: &Tree) -> anyhow::Result<Tree> {
    let dirs = ["home", "go", "gomod", "gomod/cache", "gocache", "tool"];
    let mut missing: Vec<Entry> = dirs
        .iter()
        .map(|d| format!("{STATE_SUBDIR}/{d}"))
        .filter(|p| tree.get(p).is_none())
        .map(|p| Entry::dir(p, Hash::of(b"")))
        .collect();
    let mode = format!("{STATE_SUBDIR}/{GO_TELEMETRY_MODE}");
    if tree.get(&mode).is_none() {
        missing.push(Entry::file(mode, cas.put(b"off\n")?, 0o644));
    }
    Ok(if missing.is_empty() {
        tree.clone()
    } else {
        tree.with_outputs(&missing)
    })
}

/// One step through the hooks: classify, key, decide, execute or replay,
/// record, report.
pub fn run_script_step(state_dir: &Path, input: &Input, policy: &Policy) -> anyhow::Result<i32> {
    let step_start = Instant::now();
    let id = input
        .state_str("job")
        .context("run_script_step without a job in the hook state (prepare_job did not run?)")?;
    let mut job = Job::load(state_dir, id)?;
    let cas = Cas::open(state_dir.join("cas"))?;
    job.ensure_scope(&cas)?;
    // Trust state (proofs, audits, quarantine) lives in the scope's own
    // ledger, opened below: a fork's steps never prove a trusted key.
    let args = input.script_step_args()?;
    job.steps += 1;
    job.totals.steps += 1;
    let n = job.steps;
    let step_dir = job.dir().join(format!("step-{n:03}"));
    std::fs::create_dir_all(&step_dir)?;
    job.save()?;
    let ledger = Ledger::open(&job.scope().join("ledger.sqlite"))?;
    let salt = secrets::salt(state_dir)?;

    let mut step = step::classify(&job, &args, policy)?;
    for why in &job.dirty {
        step.taint
            .push(format!("an earlier step of the job: {why}"));
    }
    let env_in = &args.environment_variables;

    let code = match step.kind.clone() {
        Kind::CheckoutPost if job.checkout_fallback => {
            // The real action ran; its post step removes the credentials it
            // persisted.
            let tree = job.load_tree()?;
            run_tainted(
                &mut job,
                &args,
                &step,
                &cas,
                &tree,
                &step_dir,
                vec!["the post step of a checkout that fell back to the real action".into()],
            )?
        }
        Kind::CheckoutPost => {
            eprintln!("stillpass: {} is a no-op for a CAS checkout", step.label);
            0
        }
        Kind::Checkout(inputs) => {
            let t0 = Instant::now();
            let prev = job.load_tree()?;
            let c = checkout::checkout(state_dir, &cas, &inputs)
                .with_context(|| format!("checkout {}@{}", inputs.repository, inputs.reference))?;
            // The checkout replaces the repository; the workspace's own
            // state under .stillpass/ carries over (a self-hosted runner's
            // persistent workspace, minus the git history).
            let state_prefix = format!("{STATE_SUBDIR}/");
            let leaves = c
                .tree
                .iter()
                .filter(|e| e.p != STATE_SUBDIR && !e.p.starts_with(&state_prefix))
                .chain(
                    prev.iter()
                        .filter(|e| e.p == STATE_SUBDIR || e.p.starts_with(&state_prefix)),
                )
                .cloned();
            let tree = with_state_dirs(&cas, &Tree::from_leaves(leaves))?;
            let t1 = Instant::now();
            job.save_tree(&tree)?;
            host_copy(&job, &cas, &c)?;
            tracing::info!(target: "stillpass", job = %job.id, build_ms = (t1 - t0).as_millis() as u64, save_ms = t1.elapsed().as_millis() as u64, "checkout phases");
            // actions/checkout's outputs, and the state its post step keys on.
            if let Some(out) = env_in
                .get("GITHUB_OUTPUT")
                .and_then(|p| job.command_file(p))
            {
                surface::append(
                    &out,
                    format!("ref={}\ncommit={}\n", c.ref_output, c.sha).as_bytes(),
                )?;
            }
            if let Some(st) = env_in.get("GITHUB_STATE").and_then(|p| job.command_file(p)) {
                surface::append(&st, b"isPost=true\n")?;
            }
            let files = c.tree.iter().filter(|e| e.k == EntryKind::File).count();
            let line = format!(
                "[checkout] {} {} served from the CAS ({} files, {}{}); would replay: yes",
                inputs.repository,
                &c.sha[..12],
                files,
                fmt_duration(c.elapsed),
                if c.memo { ", memo" } else { ", fetched" }
            );
            println!("stillpass: {line}");
            job.summary(env_in, &format!("- {line}"));
            job.totals.would_replay += 1;
            job.totals.replayed += u32::from(policy.mode == Mode::Replay);
            tracing::info!(target: "stillpass", job = %job.id, step = n, kind = "checkout", repository = %inputs.repository, sha = %c.sha, files, memo = c.memo, ms = c.elapsed.as_millis() as u64, "step");
            0
        }
        Kind::Action(a) if step.tainted() && a.name.starts_with("actions/checkout@") => {
            job.checkout_fallback = true;
            let tree = job.load_tree()?;
            run_tainted(
                &mut job,
                &args,
                &step,
                &cas,
                &tree,
                &step_dir,
                step.taint.clone(),
            )?
        }
        Kind::Action(_) | Kind::Script { .. } | Kind::Command => run_step(
            &mut job, &args, &step, policy, &cas, &ledger, &step_dir, &salt,
        )?,
    };
    if !policy.keep_jobs {
        for f in [
            "stdout",
            "stderr",
            "online/stdout",
            "online/stderr",
            "runner/stdout",
            "runner/stderr",
        ] {
            let _ = std::fs::remove_file(step_dir.join(f));
        }
    }
    job.totals.exec_ms += step_start.elapsed().as_millis() as u64;
    let t = &job.totals;
    job.summary(
        env_in,
        &format!(
            "  (job so far: {} steps, {} would replay, {} replayed, {} tainted)",
            t.steps, t.would_replay, t.replayed, t.tainted
        ),
    );
    tracing::info!(target: "stillpass", job = %job.id, step = n, label = %step.log_label(), hook_ms = step_start.elapsed().as_millis() as u64, "hook");
    job.save()?;
    protocol::write_response(
        input.response_file.as_deref(),
        &StateResponse {
            state: input.state.clone(),
        },
    )?;
    Ok(code)
}

/// The runner itself reads the workspace on the host: local actions
/// (`uses: ./...`) and `hashFiles()`. It gets a copy of the checkout, and
/// only the checkout; steps write to the tree. Steps never see this copy
/// (the observed workspace is bound over it), so it is rewritten only when
/// the commit changes; the marker is per host directory (each runner has
/// its own `_work`).
fn host_copy(job: &Job, cas: &Cas, c: &checkout::CheckedOut) -> anyhow::Result<()> {
    let Some(host) = job.host_path(&job.working_directory) else {
        return Ok(());
    };
    let marker = job
        .state_dir
        .join("host-copies")
        .join(slug(&host.to_string_lossy()));
    let current = std::fs::read_to_string(&marker).unwrap_or_default();
    let populated = std::fs::read_dir(&host).is_ok_and(|mut d| d.next().is_some());
    if current.trim() == c.sha && populated {
        return Ok(());
    }
    let t = Instant::now();
    let _ = std::fs::remove_file(&marker);
    std::fs::create_dir_all(&host)?;
    for e in std::fs::read_dir(&host)? {
        sp_cas::remove_any(&e?.path())?;
    }
    cas.materialize(&c.tree, &host)
        .with_context(|| format!("materialize checkout at {}", host.display()))?;
    std::fs::create_dir_all(marker.parent().unwrap())?;
    std::fs::write(&marker, &c.sha)?;
    tracing::info!(target: "stillpass", job = %job.id, ms = t.elapsed().as_millis() as u64, "host checkout copy");
    Ok(())
}

/// The run of one step on one surface.
#[allow(clippy::too_many_arguments)]
fn execute(
    job: &Job,
    args: &ScriptStepArgs,
    s: &Surface,
    cas: &Cas,
    tree: &Tree,
    dir: &Path,
    keep: Keep,
    echo: bool,
) -> anyhow::Result<Executed> {
    let hermetic = s.kind == surface::SurfaceKind::Hermetic;
    let gocache = s.env.iter().any(|(k, _)| k == "GOCACHE").then(|| {
        if hermetic {
            job.warm_gocache()
        } else {
            job.scratch_gocache()
        }
    });
    exec::execute(&Exec {
        cas,
        tree,
        rootfs: job.rootfs.as_ref(),
        root_layers: &job.root_layers,
        ws_layers: &job.ws_layers,
        step_dir: dir,
        workspace: &job.working_directory,
        binds: s.mounts.clone(),
        echo,
        cmd: s.cmd.clone(),
        cwd: args.working_directory.clone(),
        env: s.env.clone(),
        egress: s.egress,
        net_probe: hermetic,
        // Not sealed: a sealed module cache is recorded whole, so the C2
        // scan would have to treat every module's sources as read (one
        // keychain.go naming GITHUB_ACTOR made every step re-run), and every
        // step's manifest would miss on any module-cache change. Observed
        // through the FUSE tree, the cache records what was read.
        seal: false,
        gocache,
        // No `--user` is the container's default, root (with the image's
        // own passwd); `None` would be the sandbox's runner uid.
        uid: Some(job.uid.unwrap_or(0)),
        gid: job.gid,
        keep,
        masks: &s.masks,
    })
}

/// After a run whose writes were not stored: the layers and leftovers it
/// leaves for the rest of the job, each of which taints every later step.
fn keep_side_effects(
    job: &mut Job,
    ex: &Executed,
    temp: Option<(&Path, Option<PathBuf>)>,
    reasons: &mut Vec<String>,
) -> anyhow::Result<()> {
    if let Some((upper, first)) = &ex.root_layer {
        job.root_layers.insert(0, upper.clone());
        let why = format!("wrote outside the workspace ({first})");
        job.dirty.push(why.clone());
        reasons.push(why);
    }
    if let Some(l) = &ex.ws_layer {
        job.ws_layers.insert(0, l.clone());
        let why = match &ex.secret_output {
            Some(f) => format!("an output held a secret value ({f})"),
            None => "a step that had the job's secrets wrote to the workspace".to_string(),
        };
        job.dirty.push(why.clone());
        reasons.push(why);
    }
    if let Some((step_temp, runner_temp)) = temp {
        let left = surface::temp_leftovers(step_temp);
        if !left.is_empty() {
            // On GitHub, later steps would find these in RUNNER_TEMP.
            if let Some(rt) = runner_temp {
                for name in &left {
                    let st = std::process::Command::new("cp")
                        .arg("-a")
                        .arg(step_temp.join(name))
                        .arg(&rt)
                        .status()
                        .context("cp")?;
                    if !st.success() {
                        bail!("copying RUNNER_TEMP leftovers back to the runner failed");
                    }
                }
            }
            let why = format!("left files in RUNNER_TEMP ({})", left.join(", "));
            job.dirty.push(why.clone());
            reasons.push(why);
        }
    }
    Ok(())
}

/// A tainted step (or the last try of one): the runner's surface, never
/// recorded, nothing stored.
fn run_tainted(
    job: &mut Job,
    args: &ScriptStepArgs,
    step: &Step,
    cas: &Cas,
    tree: &Tree,
    dir: &Path,
    mut taint: Vec<String>,
) -> anyhow::Result<i32> {
    let s = surface::runner(job, args)?;
    let host_temp = step::temp_dir(job).and_then(|t| job.host_path(&t));
    let before = host_temp.as_deref().map(surface::temp_snapshot);
    let ex = execute(
        job,
        args,
        &s,
        cas,
        tree,
        &dir.join("runner"),
        Keep::Layer,
        true,
    )?;
    job.totals.executed += 1;
    job.totals.tainted += 1;
    keep_side_effects(job, &ex, None, &mut taint)?;
    if let (Some(before), Some(ht)) = (before, host_temp.as_deref()) {
        let after = surface::temp_snapshot(ht);
        let changed: Vec<&String> = after
            .iter()
            .filter(|(k, v)| before.get(*k) != Some(v))
            .map(|(k, _)| k)
            .collect();
        if !changed.is_empty() {
            let why = format!(
                "left files in RUNNER_TEMP ({})",
                changed
                    .iter()
                    .take(3)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            job.dirty.push(why.clone());
            taint.push(why);
        }
    }
    if let Some(why) = &ex.resource_failure {
        taint.push(why.clone());
    }
    let line = format!(
        "[executed] `{}`: {} in {}; would replay: no, tainted: {}",
        step.label,
        if ex.exit_code == 0 { "ok" } else { "failed" },
        fmt_duration(ex.wall),
        taint.join("; ")
    );
    eprintln!("STILLPASS  EXECUTED  {line}");
    job.summary(&args.environment_variables, &format!("- {line}"));
    tracing::info!(target: "stillpass", job = %job.id, step = job.steps, label = %step.log_label(), decision = "tainted", taint = ?taint, exit_code = ex.exit_code, exec_ms = ex.wall.as_millis() as u64, "step");
    Ok(ex.exit_code)
}

/// The network surface: a JavaScript action or a network-rule script that
/// failed offline. Its writes are stored (it held no secrets) but it is
/// never recorded. `None` only when the step was NOT run online (the
/// network surface would carry the job's secrets): the caller then goes on
/// to the runner's surface, a first and only execution there. Once the step
/// has run online — whether it succeeded or failed — this never re-runs it:
/// a network step that performed its effect and then exited nonzero (a
/// deploy, a `curl` that half-succeeded) must not be executed a second time,
/// so a failed online attempt is reported as a tainted failure, not retried
/// with the runner's environment.
#[allow(clippy::too_many_arguments)]
fn run_online(
    job: &mut Job,
    args: &ScriptStepArgs,
    step: &Step,
    cas: &Cas,
    tree: &Tree,
    dir: &Path,
    salt: &[u8; 32],
    taint: &mut Vec<String>,
) -> anyhow::Result<Option<i32>> {
    let s = surface::network(job, args, step, dir, cas, salt)?;
    if !s.taint.is_empty() {
        return Ok(None);
    }
    let ex = execute(job, args, &s, cas, tree, dir, Keep::Store, true)?;
    let ok = ex.exit_code == 0 && ex.signal.is_none() && ex.resource_failure.is_none();
    job.totals.executed += 1;
    job.totals.tainted += 1;
    if let Some(outputs) = &ex.outputs {
        job.save_tree(&tree.with_outputs(outputs))?;
    }
    let (_, file_taint) = surface::hand_back(&s.files, cas, &s.masks)?;
    taint.extend(file_taint);
    if let Some(why) = &ex.resource_failure {
        taint.push(why.clone());
    }
    if !ok {
        // It reached the network and then failed. It may already have had
        // its effect, so it is not run again; its failure is the step's.
        taint.push(format!(
            "failed with the network (exit {}); not re-run, to not repeat a side effect",
            ex.exit_code
        ));
    }
    let runner_temp = step::temp_dir(job).and_then(|t| job.host_path(&t));
    keep_side_effects(job, &ex, s.temp.as_deref().map(|t| (t, runner_temp)), taint)?;
    let line = format!(
        "[executed] `{}`: {} in {} with the network; would replay: no, tainted: {}",
        step.label,
        if ok { "ok" } else { "failed" },
        fmt_duration(ex.wall),
        taint.join("; ")
    );
    eprintln!("STILLPASS  EXECUTED  {line}");
    job.summary(&args.environment_variables, &format!("- {line}"));
    tracing::info!(target: "stillpass", job = %job.id, step = job.steps, label = %step.log_label(), decision = "online", taint = ?taint, exit_code = ex.exit_code, exec_ms = ex.wall.as_millis() as u64, "step");
    Ok(Some(ex.exit_code))
}

/// Everything but the checkout: a replay, an observed execution, an
/// audited replay, or a tainted run.
#[allow(clippy::too_many_arguments)]
fn run_step(
    job: &mut Job,
    args: &ScriptStepArgs,
    step: &Step,
    policy: &Policy,
    cas: &Cas,
    ledger: &Ledger,
    step_dir: &Path,
    salt: &[u8; 32],
) -> anyhow::Result<i32> {
    let t0 = Instant::now();
    let tree = job.load_tree()?;
    let load_ms = t0.elapsed().as_millis() as u64;
    if policy.keep_jobs {
        std::fs::write(step_dir.join("tree.json"), tree_to_json(&tree)?)?;
    }
    if step.tainted() {
        return run_tainted(job, args, step, cas, &tree, step_dir, step.taint.clone());
    }
    let workspace = job.working_directory.clone();
    let h = surface::hermetic(job, args, step, step_dir, cas, salt)?;
    if !h.taint.is_empty() {
        return run_tainted(job, args, step, cas, &tree, step_dir, h.taint.clone());
    }
    let image = job.key_image()?;
    let mut key_cmd = step.cmd.clone();
    key_cmd.extend(h.key_extra.iter().cloned());
    // Volatile names are withheld or given fixed placeholders, and
    // RUNNER_TEMP is pinned: none of them goes in the key (a key refuses the
    // DENYLIST), and the manifest lists them as present-but-unkeyed.
    let key_env: Vec<(String, String)> = h
        .key_env
        .iter()
        .filter(|(k, _)| !sp_key::unkeyed(k))
        .cloned()
        .collect();
    let mut unkeyed: Vec<String> = h
        .withheld
        .iter()
        .cloned()
        .chain(
            h.key_env
                .iter()
                .filter(|(k, _)| sp_key::unkeyed(k))
                .map(|(k, _)| k.clone()),
        )
        .collect();
    unkeyed.sort();
    unkeyed.dedup();
    let spec = StepSpec {
        image,
        // The hooks build the whole env themselves; the toolchain names the
        // profile whose warm knobs they merge (Go's).
        toolchain: Toolchain::Go,
        cmd: key_cmd,
        cwd: args.working_directory.clone(),
        env: key_env,
        roots: vec![workspace.clone()],
        warm: h.env.iter().any(|(k, _)| k == "GOCACHE"),
    };
    let key = step_key(&spec)?;
    ledger.put_step_key(&key, &sp_key::canonical_key_json(&spec)?)?;
    let keyed_env = spec.normalized_env()?.len();
    let run_id = ulid::Ulid::generate().to_string();

    let surface_ms = t0.elapsed().as_millis() as u64 - load_ms;
    let t1 = Instant::now();
    let lookup = ledger.find_match(&key, &tree)?;
    let lookup_ms = t1.elapsed().as_millis() as u64;
    tracing::info!(target: "stillpass", job = %job.id, step = job.steps, load_ms, surface_ms, lookup_ms, candidates = lookup.candidates, "step phases");
    let candidates = lookup.candidates;
    let closest = lookup
        .closest
        .as_ref()
        .map(|(_, m)| (m.path.clone(), m.to_string()));
    // The trust layer decides, as for `stillpass run`: a matching manifest
    // replays only once proven (`proofs` identical executions), never as a
    // failure, never while quarantined; replay mode audits a fraction.
    let trust = Trust::new(ledger, cas, policy.trust())?;
    let label = step.log_label();
    let ignore = ignored_outputs(&h.env, &workspace);
    let tstep = TrustStep {
        key,
        cmd: &label,
        ignore: &ignore,
    };
    let mode = policy.mode_name();
    let decision = trust.decide(&key, lookup.hit.as_ref())?;
    let replayed = match decision {
        Decision::Replay => {
            let hit = lookup.hit.as_ref().expect("a replay needs a hit");
            // Decided again as it is recorded (another job may have
            // quarantined the key or taken the audit slot meanwhile).
            ledger.atomically(|l| -> anyhow::Result<_> {
                Ok(match trust.record_replay(&tstep, hit)? {
                    Replayed::Replayed(facts) => {
                        l.record_run_facts(&run_facts(
                            &run_id,
                            &key,
                            job,
                            mode,
                            "replayed",
                            Some(&hit.1.manifest_id),
                            Some(hit.1.exit_code),
                            None,
                            None,
                        ))?;
                        Ok(facts)
                    }
                    Replayed::Execute(reason) => Err(reason),
                })
            })?
        }
        Decision::Execute(reason) => Err(reason),
    };
    let reason = match replayed {
        Ok(facts) => {
            let (manifest, result) = lookup.hit.as_ref().expect("a replay needs a hit");
            let t = Instant::now();
            // Only what the proofs vouch for: the scratch the comparison
            // skipped (HOME_VOLATILE, the build cache) is not served.
            let served: Vec<Entry> = sp_trust::comparable_outputs(&result.outputs, &ignore)
                .into_iter()
                .cloned()
                .collect();
            if !served.is_empty() {
                job.save_tree(&tree.with_outputs(&served))?;
            }
            apply_commands(job, &args.environment_variables, cas, result)?;
            let code = sp_replay::replay_log(
                result,
                cas,
                &mut std::io::stdout().lock(),
                &mut std::io::stderr().lock(),
            )?;
            let recorded = Duration::from_millis(result.duration_ms);
            let saved = recorded.saturating_sub(t.elapsed());
            job.totals.replayed += 1;
            job.totals.would_replay += 1;
            job.totals.saved_ms += saved.as_millis() as u64;
            let line = format!(
                "[replayed] `{}`: saved {} (recorded {}, replay {}, {} inputs matched; {})",
                step.label,
                fmt_duration(saved),
                fmt_duration(recorded),
                fmt_duration(t.elapsed()),
                sp_replay::group(manifest.entries.len()),
                facts.receipt_line()
            );
            eprint!(
                "{}",
                sp_replay::receipt(
                    result,
                    manifest.entries.len(),
                    t.elapsed(),
                    &[facts.receipt_line()]
                )
            );
            job.summary(&args.environment_variables, &format!("- {line}"));
            tracing::info!(target: "stillpass", job = %job.id, step = job.steps, run_id, step_key = %key, manifest_id = %result.manifest_id, label = %label, hit = true, decision = "replay", proofs = facts.streak, recorded_ms = result.duration_ms, replay_ms = t.elapsed().as_millis() as u64, exit_code = code, "step");
            return Ok(code);
        }
        Err(reason) => reason,
    };

    // Execute on the hermetic surface (observe mode, a miss, or an audit).
    // A step that may need the network is captured, not streamed, until it
    // is known to have managed without.
    let ex = execute(
        job,
        args,
        &h,
        cas,
        &tree,
        step_dir,
        Keep::Store,
        step.retry.is_none(),
    )?;

    // Reasons the offline run is not the step's result: it is run again.
    let mut again: Vec<String> = Vec::new();
    if ex.exit_code != 0 {
        again.push(format!("exited {} offline", ex.exit_code));
    }
    match ex.net_attempts {
        Some(0) => {}
        Some(n) => again.push(format!("tried to reach the network offline ({n} attempts)")),
        None => again.push("its network attempts could not be counted".to_string()),
    }
    if let Some(sig) = ex.signal {
        again.push(format!("killed by signal {sig}"));
    }
    let machine = ex.resource_failure.clone();
    if let Some(why) = &machine {
        again.push(why.clone());
    }
    if again.is_empty() {
        // A file the step read or executed names a variable it was not
        // given: on GitHub it would have had it.
        // Files only: a directory entry's hash is its listing, not a blob.
        // From the sealed module cache, which is recorded whole rather than
        // as read, only Go sources: code that could read the environment
        // (its data files, a sigstore example JSON, a go-gh workflow, name
        // GITHUB_SHA and friends without anything reading them).
        let file_hash = |e: &Entry| (e.k == EntryKind::File).then_some(e.h).flatten();
        let sealed = format!("{STATE_SUBDIR}/gomod/");
        let mut files: BTreeMap<String, Hash> = ex
            .log
            .iter()
            .filter(|a| a.kind == EntryKind::File)
            .filter(|a| !a.path.starts_with(&sealed) || sp_key::go_source_file(&a.path))
            .filter(|a| scan::may_drive_a_program(&a.path))
            .filter_map(|a| {
                tree.get(&a.path)
                    .and_then(file_hash)
                    .map(|h| (a.path.clone(), h))
            })
            .collect();
        for (root, t) in &job.action_trees {
            files.extend(
                t.iter()
                    .filter_map(|e| file_hash(e).map(|h| (format!("{root}/{}", e.p), h))),
            );
        }
        let names = scan::referenced_by(cas, &files, &h.absent)?;
        if !names.is_empty() {
            again.push(format!(
                "a file it read names {} (withheld from offline steps)",
                names
                    .iter()
                    .map(|(n, p)| format!("{n} in {p}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if !again.is_empty() {
        // A hit's recorded run was clean; this one, on the same inputs, is
        // not the step's own result. The machine's doing (killed, out of
        // memory) teaches nothing but restarts the streak; anything else is
        // the step behaving otherwise, a divergence.
        if let Some(hit) = lookup.hit.as_ref() {
            if machine.is_some() || ex.signal.is_some() {
                if let Some(line) = trust.record_incomplete(&tstep, Some(hit), &again.join("; "))? {
                    eprintln!("stillpass: `{}`: {line}", step.label);
                }
            } else {
                let v =
                    trust.record_divergence(&tstep, &run_id, hit, &reason, &again.join("; "))?;
                job.totals.audits += 1;
                job.totals.divergences += 1;
                for l in v.lines() {
                    eprintln!("{l}");
                }
            }
        }
        if step.retry.is_some() && step.retry.is_some() && !step.tainted() {
            print_captured(step_dir)?;
        }
        let note = format!(
            "{}; ran again{}",
            again.join("; "),
            if step.retry.is_some() && ex.exit_code != 0 {
                " with the network"
            } else {
                " with the runner's environment"
            }
        );
        eprintln!("stillpass: `{}`: {note}", step.label);
        let mut taint = vec![note];
        // The network surface is for fetches (a cache or tool install that
        // failed offline); a step that succeeded offline but reached for
        // the network has a side effect, which wants the job's secrets.
        if step.retry.is_some()
            && ex.exit_code != 0
            && machine.is_none()
            && let Some(code) = run_online(
                job,
                args,
                step,
                cas,
                &tree,
                &step_dir.join("online"),
                salt,
                &mut taint,
            )?
        {
            return Ok(code);
        }
        return run_tainted(job, args, step, cas, &tree, step_dir, taint);
    }

    if step.retry.is_some() {
        print_captured(step_dir)?;
    }
    let (commands, file_taint) = surface::hand_back(&h.files, cas, &h.masks)?;
    let mut taint = file_taint;
    if let Some(outputs) = &ex.outputs {
        job.save_tree(&tree.with_outputs(outputs))?;
    }
    let runner_temp = step::temp_dir(job).and_then(|t| job.host_path(&t));
    keep_side_effects(
        job,
        &ex,
        h.temp.as_deref().map(|t| (t, runner_temp)),
        &mut taint,
    )?;
    // A workspace without a .git (no checkout yet, or one that fell back):
    // a step that looked for one saw a world GitHub's runner would not
    // show it.
    if tree.get(".git").is_none()
        && let Some(a) = ex
            .log
            .iter()
            .find(|a| a.path == ".git" || a.path.starts_with(".git/"))
    {
        taint.push(format!(
            "it looked for {} (the workspace has no .git)",
            a.path
        ));
    }
    job.totals.executed += 1;

    // What a step that ran clean on the same inputs before did not do now
    // (left a taint, wrote a secret to an output): with a hit, that is the
    // step behaving otherwise, a divergence; without one, it is merely not
    // recorded.
    let not_recordable: Option<String> = match (&ex.outputs, ex.stdout, ex.stderr) {
        (Some(_), Some(_), Some(_)) if taint.is_empty() => None,
        (Some(_), Some(_), Some(_)) => Some(taint.join("; ")),
        // Kept as a layer (an output held a secret).
        _ => Some(
            ex.secret_output
                .as_ref()
                .map_or("kept as a layer".to_string(), |p| {
                    format!("wrote a secret value to {p}")
                }),
        ),
    };
    let verdict = match (&not_recordable, &ex.outputs, ex.stdout, ex.stderr) {
        (Some(why), ..) => {
            job.totals.tainted += 1;
            match lookup.hit.as_ref() {
                Some(hit) => {
                    let v = trust.record_divergence(&tstep, &run_id, hit, &reason, why)?;
                    job.totals.audits += 1;
                    job.totals.divergences += 1;
                    Some(v)
                }
                None => None,
            }
        }
        (None, Some(outputs), Some(stdout), Some(stderr)) => {
            let manifest = Manifest::from_log(key, spec.roots.clone(), &tree, ex.log.clone())?
                .with_volatile(unkeyed.clone());
            let mid = sp_manifest::manifest_id(&manifest)?;
            let result = StepResult {
                v: sp_manifest::VERSION,
                step_key: key,
                manifest_id: mid,
                run_id: run_id.clone(),
                recorded_at: sp_ledger::now_rfc3339(),
                exit_code: ex.exit_code,
                stdout,
                stderr,
                outputs: outputs.clone(),
                // Writes outside the workspace are kept as job-local layers
                // (and taint), never recorded: a recorded result has none.
                root_outputs: vec![],
                duration_ms: ex.wall.as_millis() as u64,
                commands: commands.clone(),
            };
            // One transaction: the trust verdict (proof streak, trust state,
            // audit row), the run row, the manifest and the result.
            let v = ledger.atomically(|l| -> anyhow::Result<Verdict> {
                l.put_manifest(&manifest, &run_id)?;
                let v = trust.record_execution(
                    &tstep,
                    &run_id,
                    &manifest,
                    &result,
                    lookup.hit.as_ref(),
                    &reason,
                )?;
                l.record_execution_facts(
                    &RunFacts {
                        // The hooks observer is always the classic fuse
                        // transport (sp_fs::serve, never the ring).
                        observed: Observed {
                            transport: Some("fuse"),
                            observer: Some(&ex.observer),
                        },
                        ..run_facts(
                            &run_id,
                            &key,
                            job,
                            mode,
                            v.outcome(),
                            Some(&mid),
                            Some(ex.exit_code),
                            Some(ex.wall),
                            v.would_have_saved,
                        )
                    },
                    &manifest,
                    &result,
                )?;
                Ok(v)
            })?;
            if v.divergence.is_some() {
                job.totals.divergences += 1;
            }
            Some(v)
        }
        _ => unreachable!("recordable means outputs and logs"),
    };
    if verdict.as_ref().is_some_and(|v| v.compared_with.is_some()) && not_recordable.is_none() {
        job.totals.audits += 1;
    }
    finish_report(
        job,
        args,
        step,
        &ex,
        &reason,
        verdict.as_ref(),
        not_recordable.as_deref(),
        &taint,
        &key,
        run_id,
        candidates,
        closest,
        keyed_env,
        &h,
        &label,
    )
}

/// The trust verdict, the would-replay line, the summary and the event.
#[allow(clippy::too_many_arguments)]
fn finish_report(
    job: &mut Job,
    args: &ScriptStepArgs,
    step: &Step,
    ex: &Executed,
    reason: &Reason,
    verdict: Option<&Verdict>,
    not_recordable: Option<&str>,
    taint: &[String],
    key: &Hash,
    run_id: String,
    candidates: usize,
    closest: Option<(String, String)>,
    keyed_env: usize,
    h: &Surface,
    label: &str,
) -> anyhow::Result<i32> {
    let short = |r: &str| r[..r.len().min(8)].to_string();
    let mut audit_note = String::new();
    if let Some(v) = verdict {
        if let Some(d) = &v.divergence {
            audit_note = format!(
                "; **audit DIVERGED** from run {}: key quarantined ({d})",
                v.compared_with.as_deref().map_or("?".to_string(), short)
            );
        } else if let (Reason::Audit { .. }, Some(r)) = (reason, &v.compared_with) {
            audit_note = format!("; audit: agrees with run {}", short(r));
        }
    }
    let would = verdict.is_some_and(|v| {
        v.divergence.is_none() && matches!(v.reason, Reason::WouldReplay | Reason::Audit { .. })
    });
    let verdict_line = match (verdict, not_recordable) {
        (_, Some(why)) => format!("would replay: no, tainted: {why}"),
        (Some(v), None) if v.divergence.is_some() => {
            "would replay: no, quarantined by this run's divergence".to_string()
        }
        (Some(v), None) => match &v.reason {
            Reason::WouldReplay | Reason::Audit { .. } => {
                let saved = v
                    .would_have_saved
                    .unwrap_or(Duration::ZERO)
                    .max(Duration::ZERO);
                job.totals.would_replay += 1;
                job.totals.saved_ms += saved.as_millis() as u64;
                format!(
                    "would replay: yes, saving {} ({} proofs)",
                    fmt_duration(saved),
                    v.streak
                )
            }
            Reason::Unprovable { why } => format!("would replay: no, not provable ({why})"),
            Reason::Quarantined { .. } => format!(
                "would replay: no, quarantined after a divergence (re-proof {} of {})",
                v.streak, v.needed
            ),
            Reason::Proving { .. } => format!(
                "would replay: not yet ({} of {} identical proofs){}",
                v.streak,
                v.needed,
                if v.replayable() {
                    "; replayable from the next matching run"
                } else {
                    ""
                }
            ),
            Reason::Miss if candidates == 0 => format!(
                "would replay: no, first run of this step ({} of {} identical proofs)",
                v.streak, v.needed
            ),
            Reason::Miss => {
                let (path, mm) = closest.clone().unwrap_or_default();
                // Not `path: reason`: a problem matcher (setup-go's for Go)
                // would take that for a compiler error and annotate the run.
                let why = mm.strip_prefix(&format!("{path}: ")).unwrap_or(&mm);
                format!(
                    "would replay: no, closest manifest differed at {path} [{why}] ({} of {} identical proofs)",
                    v.streak, v.needed
                )
            }
        },
        (None, None) => "would replay: no".to_string(),
    };
    let line = format!(
        "[executed] `{}`: {} in {}; {}{}",
        step.label,
        if ex.exit_code == 0 { "ok" } else { "failed" },
        fmt_duration(ex.wall),
        verdict_line,
        audit_note
    );
    eprintln!("STILLPASS  EXECUTED  {line}");
    if let Some(v) = verdict {
        for l in v.lines() {
            eprintln!("  {l}");
        }
    }
    job.summary(&args.environment_variables, &format!("- {line}"));
    tracing::info!(
        target: "stillpass",
        job = %job.id,
        step = job.steps,
        run_id,
        step_key = %key,
        label = %label,
        reason = ?reason,
        outcome = verdict.map(|v| v.outcome()),
        would_replay = would,
        taint = ?taint,
        candidates,
        first_mismatch = closest.as_ref().map(|(p, _)| p.clone()),
        keyed_env,
        withheld = h.withheld.len(),
        inputs = ex.log.len(),
        outputs = ex.outputs.as_ref().map_or(0, Vec::len),
        observer = %ex.observer,
        exit_code = ex.exit_code,
        exec_ms = ex.wall.as_millis() as u64,
        mount_ms = ex.phases.mount.as_millis() as u64,
        diff_ms = ex.phases.diff.as_millis() as u64,
        "step"
    );
    Ok(ex.exit_code)
}

/// What sp-trust leaves out when it compares two results of a hooks step:
/// `stillpass run`'s rule (sp-cli `ignored_outputs`) on the hooks' own
/// layout. Under HOME only `HOME_VOLATILE`; a directory the env points at
/// that the Go profile calls scratch (`ephemeral`, `setup_cache`: the build
/// cache) whole. Paths are workspace-relative, as outputs are.
fn ignored_outputs(env: &[(String, String)], workspace: &str) -> Vec<String> {
    let profile = Toolchain::Go.profile();
    let mut out = Vec::new();
    for (var, value) in env {
        let Some(dir) = value
            .strip_prefix(workspace)
            .and_then(|r| r.strip_prefix('/'))
            .map(|r| r.trim_end_matches('/').to_string())
            .filter(|r| !r.is_empty())
        else {
            continue;
        };
        if var == "HOME" {
            out.extend(sp_trust::HOME_VOLATILE.iter().map(|v| format!("{dir}/{v}")));
        } else if profile
            .ephemeral
            .iter()
            .chain(profile.setup_cache)
            .any(|d| *d == dir)
        {
            out.push(dir);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The run row of a hooks step: what `stillpass status` lists.
#[allow(clippy::too_many_arguments)]
fn run_facts<'a>(
    run_id: &'a str,
    key: &'a Hash,
    job: &'a Job,
    mode: &'a str,
    outcome: &'a str,
    manifest_id: Option<&'a Hash>,
    exit_code: Option<i32>,
    wall: Option<Duration>,
    saved: Option<Duration>,
) -> RunFacts<'a> {
    RunFacts {
        id: run_id,
        commit_sha: None,
        host: None,
        observed: Observed::default(),
        step_key: Some(key),
        repo: (!job.repository.is_empty()).then_some(job.repository.as_str()),
        repo_path: None,
        mode: Some(mode),
        outcome: Some(outcome),
        manifest_id,
        exit_code,
        wall_ms: wall.map(|d| d.as_millis() as u64),
        saved_ms: saved.map(|d| d.as_millis() as u64),
    }
}

/// Writes a captured (not streamed) execution's output to the log.
fn print_captured(step_dir: &Path) -> anyhow::Result<()> {
    use std::io::Write;
    let out = std::fs::read(step_dir.join("stdout")).unwrap_or_default();
    let err = std::fs::read(step_dir.join("stderr")).unwrap_or_default();
    std::io::stdout().lock().write_all(&out)?;
    std::io::stderr().lock().write_all(&err)?;
    Ok(())
}

/// Replays what the recorded step appended to its command files into this
/// step's files.
fn apply_commands(
    job: &Job,
    env: &BTreeMap<String, String>,
    cas: &Cas,
    result: &StepResult,
) -> anyhow::Result<()> {
    for c in &result.commands {
        let Some(host) = env.get(&c.name).and_then(|p| job.command_file(p)) else {
            tracing::warn!(name = %c.name, "recorded command file has no target in this step");
            continue;
        };
        let bytes = cas
            .get(&c.h)?
            .with_context(|| format!("blob {} for {}", c.h, c.name))?;
        surface::append(&host, &bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_outputs_follow_stillpass_run() {
        let ws = "/__w/cli/cli";
        let env: Vec<(String, String)> = [
            ("HOME", "/__w/cli/cli/.stillpass/home"),
            ("GOCACHE", "/__w/cli/cli/.stillpass/gocache"),
            ("GOPATH", "/__w/cli/cli/.stillpass/go"),
            ("RUNNER_TEMP", "/__w/_temp"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .to_vec();
        let ign = ignored_outputs(&env, ws);
        // HOME: only the volatile paths; the build cache whole; GOPATH (go
        // install's binaries) and anything outside the workspace compared.
        assert!(ign.contains(&".stillpass/home/.cache/gh".to_string()));
        assert!(ign.contains(&".stillpass/home/.local/state".to_string()));
        assert!(ign.contains(&".stillpass/gocache".to_string()));
        assert!(
            !ign.iter()
                .any(|p| p == ".stillpass/home" || p.starts_with(".stillpass/go/"))
        );
        assert!(!ign.iter().any(|p| p.contains("_temp")));
        assert_eq!(ign.len(), sp_trust::HOME_VOLATILE.len() + 1);
    }

    #[test]
    fn scopes_from_events() {
        assert_eq!(scope_of(None), (String::new(), "unverified".into()));
        assert_eq!(
            scope_of(Some(b"not json")),
            (String::new(), "unverified".into())
        );
        let push = br#"{"repository":{"full_name":"Pango07/Stillpass"}}"#;
        assert_eq!(
            scope_of(Some(push)),
            ("pango07/stillpass".into(), "trusted".into())
        );
        let same = br#"{"repository":{"full_name":"o/r"},"pull_request":{"head":{"repo":{"full_name":"O/R"}}}}"#;
        assert_eq!(scope_of(Some(same)), ("o/r".into(), "trusted".into()));
        let fork = br#"{"repository":{"full_name":"o/r"},"pull_request":{"head":{"repo":{"full_name":"evil/r"}}}}"#;
        assert_eq!(scope_of(Some(fork)), ("o/r".into(), "fork-evil_r".into()));
        let gone = br#"{"repository":{"full_name":"o/r"},"pull_request":{"head":{"repo":null}}}"#;
        assert_eq!(scope_of(Some(gone)), ("o/r".into(), "fork-unknown".into()));
    }

    #[test]
    fn user_options() {
        assert_eq!(user_option(None).unwrap(), (None, None));
        assert_eq!(
            user_option(Some("--user 1001:1002")).unwrap(),
            (Some(1001), Some(1002))
        );
        assert_eq!(user_option(Some("-u 7")).unwrap(), (Some(7), None));
        assert_eq!(
            user_option(Some("--user=0:0 --cpus 1")).unwrap(),
            (Some(0), Some(0))
        );
        assert!(user_option(Some("--user runner")).is_err());
    }
}
