//! One job: created by `prepare_job`, driven step by step by
//! `run_script_step`, closed by `cleanup_job`. Lives at `<state>/jobs/<id>`.
//!
//! The workspace is a tree in the store, never a directory on disk: the
//! checkout sets it, every stored step's outputs are applied to it, and a
//! replay is `tree.with_outputs`. Between jobs the `.quod/` module and
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
use quod_cas::{Cas, Entry, Hash, Kind as EntryKind, Tree};
use quod_key::{Image, ImageKind, Platform, StepSpec, Toolchain, step_key};
use quod_ledger::{Ledger, Observed, RunFacts};
use quod_manifest::{Manifest, StepResult};
use quod_replay::fmt_duration;
use serde::{Deserialize, Serialize};

use crate::checkout::{self, tree_from_json, tree_to_json};
use crate::exec::{self, Exec, Executed, GoCache, Keep};
use crate::protocol::{self, Container, Input, ScriptStepArgs, StateResponse};
use crate::rootfs::{OciRootfs, Rootfs, RootfsProvider};
use crate::scan;
use crate::secrets;
use crate::step::{self, Kind, Step};
use crate::surface::{self, Surface};
use crate::trust::{Mode, Policy};
use quod_ledger::RunRecord;
use quod_trust::{Claimed, LocalLedger, Reason, Step as TrustStep, StepLedger, Trust, Verdict};
use quod_wire::client::{RemoteLedger, ServerConfig};
use quod_wire::{CiContext, ScopeHint};

/// Where Quod keeps a workspace's own state, under GITHUB_WORKSPACE:
/// HOME, GOPATH, the module cache, the tool cache. Observed like the rest
/// of the tree.
pub const STATE_SUBDIR: &str = ".quod";

/// The parts of `.quod/` that persist from job to job on a workspace
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
    /// runner adds (HOME), as job.json keeps it: classified values salted
    /// (every value until the first step has read the workflow's
    /// provenance). The values themselves are in `secrets`, outside the
    /// state (`container_env`).
    pub env: BTreeMap<String, String>,
    /// The runner's own mounts (the work directory, externals, the
    /// workflow's event and command files, HOME). The workflow's volumes
    /// are refused at prepare_job; a `user_mounts` an older job.json may
    /// carry is ignored.
    pub system_mounts: Vec<protocol::Mount>,
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
    /// A push, workflow_dispatch or schedule on the repository's default
    /// branch: the one kind of job whose steps may write the scope's warm
    /// build cache and persisted workspace (see `scope_of`).
    #[serde(default)]
    pub writer: bool,
    /// The job image's own config env may exempt a variable from
    /// classification by name or shape (`image_env_exempts`): decided with
    /// the scope.
    #[serde(default)]
    pub image_env_trusted: bool,
    /// Not a writer: the first entry a step of this job wrote into the job's
    /// own layer over the warm build cache (`exec::GoCache::Layered`). From
    /// then on a step that reads the layer reads something no key covers,
    /// written by this job's own code, so its run is not recorded.
    #[serde(default)]
    pub warm_layer_written: Option<String>,
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
    /// This host's class as step keys pin it (`quod_key::Platform`, probed
    /// once at prepare_job the way `quod run` does).
    #[serde(default)]
    pub platform: Option<Platform>,
    /// The runner's worker process this job runs under, as prepare_job
    /// found it: while it lives the job is live, and the sweep leaves it.
    #[serde(default)]
    pub owner: Option<Owner>,
    /// Where the job's secrets come from, read from its workflow at the
    /// first step (names only); `None` before that.
    #[serde(default)]
    pub provenance: Option<crate::provenance::Provenance>,
    /// The trees of action directories bound into this step (for the scan).
    #[serde(skip)]
    pub action_trees: BTreeMap<String, Tree>,
    /// The job's known secrets, kept outside the state for its life.
    #[serde(skip)]
    secrets: secrets::JobSecrets,
    /// This step's masks for what Quod writes about it (reasons, job.json,
    /// events, the ledger): the job's secrets in every encoding.
    #[serde(skip)]
    masks: Vec<Vec<u8>>,
    /// The state's salt (for what is logged of opaque jobs and scripts).
    #[serde(skip)]
    salt: [u8; 32],
    /// A step was given (or registered) a secret: at cleanup with
    /// `keep_jobs`, the step directories are scrubbed of what they hold
    /// raw (logs, upper dirs, staging).
    #[serde(default)]
    pub had_secrets: bool,
    /// A step was given, or registered, a certain secret too short to mask
    /// (`secrets::MIN_SECRET_LEN`): a later step printing it could not be
    /// told from one that does not, so no later step of the job is
    /// recorded.
    #[serde(default)]
    pub short_secret: Option<String>,
    /// The names earlier steps set in GITHUB_ENV (names only): a value
    /// under one of them that a known secret holds is a part a step cut out
    /// of it (`secrets::classify_with`).
    #[serde(default)]
    pub set_by_steps: std::collections::BTreeSet<String>,
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
        tracing::info!(target: "quod", image = %image, digest = %r.digest, ms = t.elapsed().as_millis() as u64, "rootfs");
        Ok(Some(r))
    }

    pub fn create(
        state_dir: &Path,
        c: &Container,
        rootfs: Option<Rootfs>,
        platform: Platform,
    ) -> anyhow::Result<Job> {
        // prepare_job refuses volumes first; this is the second line.
        if let Some(m) = c.user_mount_volumes.first() {
            bail!(
                "container volume {} is not supported by the quod runner",
                m.target_volume_path
            );
        }
        let id = ulid::Ulid::generate().to_string();
        let dir = jobs_dir(state_dir).join(&id);
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let image = c.image.clone().context("job container without an image")?;
        let (uid, gid) = user_option(c.create_options.as_deref())?;
        // Until the workflow's provenance is read (the first step), every
        // container env value is salted in job.json; the values are kept
        // outside the state.
        let salt = secrets::salt(state_dir)?;
        let salted_env = c
            .environment_variables
            .iter()
            .map(|(k, v)| {
                let v = if v.is_empty() {
                    v.clone()
                } else {
                    secrets::salted(&salt, k, v)
                };
                (k.clone(), v)
            })
            .collect();
        let job = Job {
            id,
            image,
            is_alpine: rootfs.as_ref().is_some_and(|r| r.is_alpine),
            working_directory: c
                .working_directory
                .clone()
                .context("job container without a working directory")?,
            env: salted_env,
            system_mounts: c.system_mount_volumes.clone(),
            steps: 0,
            rootfs,
            uid,
            gid,
            repository: String::new(),
            scope_name: String::new(),
            writer: false,
            image_env_trusted: false,
            warm_layer_written: None,
            scope: None,
            root_layers: Vec::new(),
            ws_layers: Vec::new(),
            dirty: Vec::new(),
            checkout_fallback: false,
            totals: Totals::default(),
            started_ms: now_ms(),
            hashes: BTreeMap::new(),
            platform: Some(platform),
            owner: Owner::of_this_hook(),
            provenance: None,
            action_trees: BTreeMap::new(),
            secrets: secrets::JobSecrets {
                container_env: c.environment_variables.clone(),
                values: Default::default(),
                pending_env: Vec::new(),
            },
            masks: Vec::new(),
            salt,
            had_secrets: false,
            short_secret: None,
            set_by_steps: Default::default(),
            dir,
            state_dir: state_dir.to_path_buf(),
        };
        job.save_secrets()?;
        job.save()?;
        Ok(job)
    }

    /// job.json holds classified container env values salted: a step must
    /// never be given the salted form for the value. Fails when the job's
    /// secrets kept outside the state are gone (a cleared shared memory).
    fn check_secrets(&self) -> anyhow::Result<()> {
        if let Some(k) = self.env.iter().find_map(|(k, v)| {
            (v.starts_with("quod:secret:") && !self.secrets.container_env.contains_key(k))
                .then_some(k)
        }) {
            bail!(
                "the job's secrets kept outside the state are gone (the container env value of {k} among them): re-run the job"
            );
        }
        Ok(())
    }

    /// The job container's env as the steps get it (the values, from the
    /// job's secrets outside the state).
    pub fn container_env(&self) -> BTreeMap<String, String> {
        let mut env = self.env.clone();
        env.extend(self.secrets.container_env.clone());
        env
    }

    /// The job image's own config env (empty in host-root mode): what the
    /// image, not the workflow or the runner, defines. Empty too unless the
    /// job's scope and image are trusted (`image_env_exempts`): the env
    /// exempts a variable from classification, and a pull request from a
    /// fork, or one naming an image anyone can push, controls it.
    pub fn image_env(&self) -> BTreeMap<String, String> {
        if !self.image_env_trusted {
            return BTreeMap::new();
        }
        self.rootfs
            .as_ref()
            .map(|r| r.env.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// The provenance read at the first step (empty before).
    pub fn provenance(&self) -> &crate::provenance::Provenance {
        static NONE: std::sync::OnceLock<crate::provenance::Provenance> =
            std::sync::OnceLock::new();
        self.provenance
            .as_ref()
            .unwrap_or_else(|| NONE.get_or_init(Default::default))
    }

    /// The job's secrets known so far (values; never written to the state).
    pub fn known_secrets(&self) -> &std::collections::BTreeSet<String> {
        &self.secrets.values
    }

    fn save_secrets(&self) -> anyhow::Result<()> {
        self.secrets.save(&self.state_dir, &self.dir, &self.id)
    }

    /// `s` with the job's secrets masked: anything Quod writes about a step.
    pub fn mask(&self, s: &str) -> String {
        secrets::mask_str(s, &self.masks)
    }

    fn add_masks(&mut self, masks: &[Vec<u8>]) {
        self.masks.extend(masks.iter().cloned());
        self.masks.sort_by_key(|b| std::cmp::Reverse(b.len()));
        self.masks.dedup();
    }

    /// At each step: the job's secrets as this step's environment shows
    /// them (`secrets::classify` over the container's env and the step's)
    /// become what Quod masks in everything it writes about the step, and
    /// join the known secrets; job.json keeps the container env with its
    /// classified values salted.
    fn learn_secrets(&mut self, env: &BTreeMap<String, String>) -> anyhow::Result<()> {
        let container = self.container_env();
        let c = secrets::classify_with(
            container.iter().chain(env.iter()),
            self.provenance(),
            self.known_secrets(),
            &self.set_by_steps,
            &self.image_env(),
        );
        self.add_masks(&c.masks);
        let salt = secrets::salt(&self.state_dir)?;
        self.salt = salt;
        let shown: BTreeMap<String, String> = self
            .secrets
            .container_env
            .iter()
            .map(|(k, v)| {
                let v = if c.is_secret(k) && !v.is_empty() {
                    secrets::salted(&salt, k, v)
                } else {
                    v.clone()
                };
                (k.clone(), v)
            })
            .collect();
        // Only the certain ones join the known secrets (see
        // `secrets::Classified::strong`).
        let new = !c.strong.is_subset(&self.secrets.values);
        // Before the provenance is read every container env value stays
        // salted in job.json (as prepare_job wrote it): which are secrets is
        // not known yet.
        if self.provenance.is_some() {
            self.env = shown;
        }
        if new {
            self.secrets.values.extend(c.strong);
            self.save_secrets()?;
        }
        Ok(())
    }

    /// A script's label for events and the trust table (salted hash).
    pub fn log_label(&self, step: &Step) -> String {
        step.log_label(&self.salt)
    }

    /// A step key as events and run rows show it: an opaque job's keys may
    /// hash a secret no name showed, unsalted, so they are shown keyed with
    /// the state's salt.
    pub fn log_key(&self, key: &Hash) -> Hash {
        if self.provenance().opaque.is_some() {
            Hash::from_blake3(blake3::keyed_hash(&self.salt, key.as_bytes()))
        } else {
            *key
        }
    }

    /// With `keep_jobs`, what a job that had secrets leaves in its step
    /// directories raw goes at cleanup: everything but each step's tree.json
    /// (the starting tree, kept for inspection).
    fn scrub_steps(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for step in rd.flatten() {
            let p = step.path();
            if !p.is_dir() || !step.file_name().to_string_lossy().starts_with("step-") {
                continue;
            }
            for e in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                if e.file_name() != "tree.json" {
                    let _ = quod_cas::remove_any(&e.path());
                }
            }
        }
    }

    /// Reads where the job's secrets come from (`provenance`), once: the
    /// workflow at GITHUB_WORKFLOW_SHA from the repository's mirror, local
    /// reusable workflows and actions from the same commit, remote actions
    /// as the runner downloaded them. Anything that cannot be read makes the
    /// job opaque, with one exception: a private repository's workflow is
    /// fetched with the job's token, which a step before actions/checkout
    /// (`run:` steps carry none) does not have. Such a step does not make
    /// the job opaque: it runs as GitHub runs it (the runner's surface,
    /// never recorded; returned here as the reason), its environment is
    /// kept with the job's secrets outside the state, and the read is tried
    /// again at the next step. Once it succeeds, those environments are
    /// classified with the provenance, so a secret such a step was given
    /// (and could have handed on through GITHUB_ENV under a plain name) is a
    /// known secret of the job before any step is recorded. Only when a
    /// step that carries a token still cannot read it is the job opaque.
    fn ensure_provenance(
        &mut self,
        env: &BTreeMap<String, String>,
    ) -> anyhow::Result<Option<String>> {
        if self.provenance.is_some() {
            return Ok(None);
        }
        let wref = env.get("GITHUB_WORKFLOW_REF").cloned().unwrap_or_default();
        let carries_token = env.values().any(|v| step::holds_github_token(v));
        let p = match self.read_provenance(env, &wref) {
            Ok(mut p) => {
                p.workflow = format!(
                    "{wref} at {}",
                    env.get("GITHUB_WORKFLOW_SHA").map_or("?", String::as_str)
                );
                p
            }
            Err(e) if !carries_token => {
                let keep =
                    secrets::pending_entries(env, &self.container_env(), &self.secrets.pending_env);
                if !keep.is_empty() {
                    self.secrets.pending_env.push(keep);
                    self.save_secrets()?;
                }
                let why = format!(
                    "the workflow is not read yet (it needs the job's token, which no step has carried so far: {e:#}); until a step does, steps run as GitHub runs them"
                );
                tracing::info!(target: "quod", job = %self.id, workflow = %wref, pending = self.secrets.pending_env.len(), "provenance deferred");
                return Ok(Some(why));
            }
            Err(e) => crate::provenance::Provenance::opaque(
                wref.clone(),
                format!("the workflow cannot be read ({e:#})"),
            ),
        };
        tracing::info!(target: "quod", job = %self.id, workflow = %p.workflow, secret_names = p.names.len(), opaque = ?p.opaque, "provenance");
        if let Some(why) = &p.opaque {
            eprintln!("quod: this job is opaque, nothing it runs is recorded: {why}");
        }
        self.provenance = Some(p);
        // What the steps before saw, classified now, and gone from the
        // sidecar in the same write (below): it is kept no longer than it
        // takes to learn which of it are secrets.
        let pending = std::mem::take(&mut self.secrets.pending_env);
        if !pending.is_empty() {
            let container = self.container_env();
            let image = self.image_env();
            for e in &pending {
                let c = secrets::classify_with(
                    container.iter().chain(e.iter()),
                    self.provenance(),
                    self.known_secrets(),
                    &self.set_by_steps,
                    &image,
                );
                self.add_masks(&c.masks);
                if !c.names.is_empty() {
                    self.had_secrets = true;
                }
                self.secrets.values.extend(c.strong);
                if c.short_strong && self.short_secret.is_none() {
                    self.short_secret = c
                        .short
                        .map(|n| format!("in {n}, given to a step before the workflow was read"));
                }
            }
            self.save_secrets()?;
        }
        Ok(None)
    }

    fn read_provenance(
        &self,
        env: &BTreeMap<String, String>,
        wref: &str,
    ) -> anyhow::Result<crate::provenance::Provenance> {
        let (repo, path, git_ref) = crate::provenance::workflow_ref(wref)
            .with_context(|| format!("GITHUB_WORKFLOW_REF {wref:?}"))?;
        let sha = env
            .get("GITHUB_WORKFLOW_SHA")
            .context("no GITHUB_WORKFLOW_SHA")?;
        let server = env
            .get("GITHUB_SERVER_URL")
            .map_or("https://github.com", String::as_str);
        // The job's token, if the step carries one (checkout's input): a
        // private repository's workflow needs it.
        let token = env
            .values()
            .map(|v| v.trim())
            .find(|v| step::holds_github_token(v) && !v.contains(char::is_whitespace));
        let commit = checkout::Commit::fetch(&self.state_dir, server, &repo, sha, &git_ref, token)?;
        let text = commit
            .read(&path)
            .with_context(|| format!("{path} not found at {sha}"))?;
        let local = |p: &str| commit.read(p);
        let actions = step::actions_dir(self).and_then(|d| self.host_path(&d));
        let remote = |uses: &str| -> Option<String> {
            let (name, r) = uses.rsplit_once('@')?;
            let mut parts = name.splitn(3, '/');
            let (owner, repo) = (parts.next()?, parts.next()?);
            let sub = parts.next().unwrap_or("");
            let mut dir = actions.clone()?.join(owner).join(repo).join(r);
            for c in sub.split('/').filter(|c| !c.is_empty()) {
                if c == ".." {
                    return None;
                }
                dir.push(c);
            }
            ["action.yml", "action.yaml"]
                .iter()
                .find_map(|f| std::fs::read_to_string(dir.join(f)).ok())
        };
        Ok(crate::provenance::analyze(
            &path,
            &text,
            &crate::provenance::Sources {
                local: &local,
                remote_action: &remote,
            },
            env.get("GITHUB_JOB").map(String::as_str),
        ))
    }

    /// Decides the job's scope from the event (its name, GITHUB_EVENT_NAME,
    /// and the event.json the runner writes before each step; see
    /// `scope_of`) and starts its workspace there: where the last job on
    /// this path in this scope left it (its persisted `.quod/` state),
    /// or for the scope's first job the operator's seed for the repository,
    /// if any. HOME and the rest are per job.
    fn ensure_scope(
        &mut self,
        cas: &Cas,
        event_name: Option<&str>,
        writer_refs: &[String],
        trusted_registries: &[String],
    ) -> anyhow::Result<()> {
        if self.scope.is_some() {
            return Ok(());
        }
        let event = step::workflow_dir(self)
            .and_then(|d| self.host_path(&format!("{d}/event.json")))
            .and_then(|p| std::fs::read(p).ok());
        let ScopeOf {
            repository,
            name: scope_name,
            writer,
        } = scope_of(event_name, event.as_deref(), writer_refs);
        let repo_slug = slug(if repository.is_empty() {
            "unknown"
        } else {
            &repository
        });
        let scope = self
            .state_dir
            .join("scopes")
            .join(&repo_slug)
            .join(&scope_name);
        std::fs::create_dir_all(&scope)?;
        self.repository = repository;
        self.image_env_trusted = image_env_exempts(&scope_name, &self.image, trusted_registries);
        self.scope_name = scope_name;
        self.writer = writer;
        self.scope = Some(scope);
        // A scope's first job starts from the operator's seed for the
        // repository (`<state>/seeds/<repo>/.quod/{gomod,tool}`, a
        // module cache filled ahead of time), or empty. Never from the
        // runner's work directory on the host: a runner-surface step of any
        // scope can write there, so a fork could seed a trusted scope's
        // module cache or tool cache through it. The state directory is
        // mounted into no step.
        let tree = match std::fs::read(self.workspace_slot()) {
            Ok(b) => persisted(&tree_from_json(&b)?),
            Err(_) => {
                let seed = self.state_dir.join("seeds").join(&repo_slug);
                if seed.is_dir() {
                    import_persisted(cas, &seed)?
                } else {
                    Tree::from_leaves([])
                }
            }
        };
        self.save_tree(&with_state_dirs(cas, &tree)?)?;
        tracing::info!(target: "quod", job = %self.id, repository = %self.repository, scope = %self.scope_name, writer = self.writer, "scope");
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
        job.secrets = secrets::JobSecrets::load(state_dir, &dir, id)?;
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

    /// The tree a step of this job is served, looked up against and
    /// recorded against: the job's tree with its git index refreshed for
    /// the step's identity (`quod_fs::git`), so `git status` takes the
    /// served stat data as a fresh checkout's and reads no unchanged file.
    pub fn step_tree(&self, cas: &Cas) -> anyhow::Result<Tree> {
        let tree = self.load_tree()?;
        let (uid, gid) = quod_sandbox::step_ids(Some(self.uid.unwrap_or(0)), self.gid);
        let (tree, r) =
            quod_fs::git::refresh_index(&tree, cas, uid, gid).context("refresh the git index")?;
        tracing::info!(target: "quod", job = %self.id, refresh = ?r, "git index");
        Ok(tree)
    }

    pub fn save_tree(&self, tree: &Tree) -> anyhow::Result<()> {
        let tmp = self.dir.join("tree.json.tmp");
        std::fs::write(&tmp, tree_to_json(tree)?)?;
        std::fs::rename(&tmp, self.tree_path())?;
        Ok(())
    }

    /// End of job: a writer's workspace persists for the next job on this
    /// path in this scope (never what a secret-bearing step wrote: that is
    /// in the job's layers); any other job's is dropped, as its layer over
    /// the warm build cache is: a pull request, even the repository's own,
    /// never leaves a module cache or a tool for the default branch to
    /// build with. The job directory goes.
    pub fn cleanup(self, policy: &Policy) -> anyhow::Result<()> {
        if self.scope.is_some()
            && self.writer
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
            "quod: job {}: {} steps, {} executed, {} replayed, {} tainted, {} would replay, saved {}, {} audits, {} divergences, job {}",
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
            target: "quod",
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
        secrets::JobSecrets::remove(&self.state_dir, &self.dir, &self.id);
        if policy.keep_jobs && self.had_secrets {
            self.scrub_steps();
        }
        if !policy.keep_jobs {
            quod_cas::remove_any(&self.dir)?;
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
        // The runner's mounts only: event.json, RUNNER_TEMP, the tool cache
        // and the workspace are where the runner put them.
        let mut best: Option<(&protocol::Mount, &str)> = None;
        for m in &self.system_mounts {
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
    /// digest, on this host's class. A rootfs pin, like `quod run
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
            .context("the job has no host class (prepared by an older quod)")?;
        Ok(Image::new(kind, self.image_key(), platform))
    }

    /// Who the step runs as, for the key: uid, gid and the passwd and group
    /// it sees (the fixed ones the sandbox binds, or the image's own).
    pub fn identity_key(&self) -> String {
        match self.uid {
            Some(u) => {
                let g = self.gid.unwrap_or(u);
                let (p, gr) = quod_sandbox::identity_files(u, g);
                format!(
                    "quod:identity uid={u} gid={g} passwd={}",
                    Hash::of(format!("{p}\0{gr}").as_bytes())
                )
            }
            None => "quod:identity uid=0 gid=0 passwd=image".to_string(),
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

    /// The job's copy-on-write layer over the warm build cache (every job
    /// that is not a writer), gone at cleanup.
    fn warm_layer(&self) -> PathBuf {
        self.dir.join("warm-layer")
    }

    /// GOCACHE for steps that are not recorded: the job's own, gone at
    /// cleanup, never the scope's warm cache.
    fn scratch_gocache(&self) -> PathBuf {
        self.dir.join("gocache")
    }
}

/// The `PERSIST` parts of `<dir>/.quod/` (an operator's seed), as a
/// workspace tree.
fn import_persisted(cas: &Cas, dir: &Path) -> anyhow::Result<Tree> {
    let mut leaves = Vec::new();
    for d in PERSIST {
        let sub = dir.join(STATE_SUBDIR).join(d);
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

/// A process, by pid and start time (so a reused pid is not it), in the
/// pid namespace of the hook that recorded it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub pid: u32,
    /// Clock ticks after boot (field 22 of /proc/<pid>/stat).
    pub start: u64,
    /// `/proc/self/ns/pid` of the recording hook (`pid:[4026531836]`): the
    /// pid means something only there.
    #[serde(default)]
    pub pid_ns: String,
}

/// How long a job directory with no owner the sweep can check (none
/// recorded, or one in another pid namespace) must sit untouched before the
/// sweep takes it: longer than GitHub lets a self-hosted job run (5 days).
const UNOWNED_STALE: Duration = Duration::from_secs(6 * 24 * 3600);

fn pid_ns() -> String {
    std::fs::read_link("/proc/self/ns/pid")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Owner {
    /// The runner's worker process this hook runs under: the nearest
    /// ancestor named `Runner.Worker` (the shim's node process is in
    /// between). None if there is none (a test harness, a runner that names
    /// its worker otherwise): the parent, the shim, dies with each hook,
    /// so a live job would look dead between its hooks.
    pub fn of_this_hook() -> Option<Owner> {
        let mut pid = std::os::unix::process::parent_id();
        for _ in 0..8 {
            if pid <= 1 {
                return None;
            }
            if std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .is_ok_and(|c| c.trim_end() == "Runner.Worker")
            {
                return Owner::of(pid);
            }
            pid = proc_stat(pid)?.ppid;
        }
        None
    }

    pub fn of(pid: u32) -> Option<Owner> {
        proc_stat(pid).map(|st| Owner {
            pid,
            start: st.start,
            pid_ns: pid_ns(),
        })
    }

    /// Whether this owner is still running, as far as this process can
    /// tell: `None` when it cannot (recorded in another pid namespace, or
    /// with none).
    pub fn alive(&self) -> Option<bool> {
        if self.pid_ns.is_empty() || self.pid_ns != pid_ns() {
            return None;
        }
        Some(proc_stat(self.pid).is_some_and(|st| st.start == self.start && st.state != 'Z'))
    }
}

struct ProcStat {
    state: char,
    ppid: u32,
    start: u64,
}

/// State, parent and start time from /proc/<pid>/stat (the fields after the
/// parenthesised comm, which may itself hold spaces and parentheses).
fn proc_stat(pid: u32) -> Option<ProcStat> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let f: Vec<&str> = s[s.rfind(')')? + 1..].split_whitespace().collect();
    Some(ProcStat {
        state: f.first()?.chars().next()?,
        ppid: f.get(1)?.parse().ok()?,
        start: f.get(19)?.parse().ok()?,
    })
}

/// The state directory's lock. Every hook invocation holds it shared
/// (`lib::run`), the sweep exclusively: a sweep never runs while a hook is
/// mid-way (a prepare_job that has made its job directory but not yet said
/// who owns it, a step with its mounts up). Between a live job's hooks no
/// lock is held; its owner (`Owner`) is what keeps it from the sweep.
pub fn state_lock_shared(state_dir: &Path) -> anyhow::Result<std::fs::File> {
    let f = open_state_lock(state_dir)?;
    f.lock_shared()
        .context("lock the state directory (shared)")?;
    Ok(f)
}

fn open_state_lock(state_dir: &Path) -> anyhow::Result<std::fs::File> {
    std::fs::create_dir_all(state_dir)?;
    let p = state_dir.join("hooks.lock");
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&p)
        .with_context(|| format!("open {}", p.display()))
}

/// A path field of /proc/self/mountinfo, whose space, tab, newline and
/// backslash are written as octal escapes (`\040`, `\011`, `\012`, `\134`).
fn unescape_mountinfo(field: &str) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let b = field.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let octal = b
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|c| (b'0'..=b'7').contains(c)));
        match octal {
            Some(d) if b[i] == b'\\' => {
                let v = d.iter().fold(0u32, |n, c| n * 8 + u32::from(c - b'0'));
                match u8::try_from(v) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 4;
                    }
                    Err(_) => {
                        out.push(b[i]);
                        i += 1;
                    }
                }
            }
            _ => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

/// Removes the job directories under `state_dir` whose owner (the runner's
/// worker process that ran prepare_job) is gone, unmounting what is still
/// mounted in them first (deepest first, lazily): a runner killed with
/// SIGKILL never reaches cleanup_job, and its job directory keeps the
/// steps' raw output, its job.json and the overlay and FUSE mounts. A job
/// whose worker still runs (another pool on this state, a worker that
/// outlived its listener) is left alone, and so is one whose owner it
/// cannot check (none recorded, or recorded in another pid namespace: the
/// sweep assumes the hooks share its pid namespace, and spares what they
/// do not) until it has sat untouched longer than any job may run. Takes the state lock exclusively,
/// waiting up to a minute for running hooks; if they do not finish, sweeps
/// nothing. Returns how many job directories it removed.
pub fn sweep_stale_jobs(state_dir: &Path) -> anyhow::Result<usize> {
    sweep_stale_jobs_waiting(state_dir, Duration::from_secs(60))
}

fn sweep_stale_jobs_waiting(state_dir: &Path, wait: Duration) -> anyhow::Result<usize> {
    let jobs = jobs_dir(state_dir);
    let Ok(rd) = std::fs::read_dir(&jobs) else {
        return Ok(0);
    };
    let lock = open_state_lock(state_dir)?;
    let t = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if t.elapsed() < wait => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                eprintln!(
                    "quod: hooks of this state are still running after {}s; stale jobs not swept",
                    wait.as_secs()
                );
                return Ok(0);
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(e).context("lock the state directory (exclusive)");
            }
        }
    }
    let owner = |dir: &Path| -> Option<Owner> {
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("job.json")).ok()?).ok()?;
        serde_json::from_value(v.get("owner")?.clone()).ok()
    };
    // Untouched since: job.json is saved by every hook of the job.
    let idle = |dir: &Path| -> Duration {
        std::fs::metadata(dir.join("job.json"))
            .or_else(|_| std::fs::metadata(dir))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .unwrap_or(Duration::ZERO)
    };
    let stale: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|d| match owner(d).and_then(|o| o.alive()) {
            Some(alive) => !alive,
            None => idle(d) > UNOWNED_STALE,
        })
        .collect();
    let mut mounts: Vec<PathBuf> = std::fs::read_to_string("/proc/self/mountinfo")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split(' ').nth(4).map(unescape_mountinfo))
        .filter(|m| stale.iter().any(|d| m.starts_with(d)))
        .collect();
    mounts.sort_by_key(|m| std::cmp::Reverse(m.as_os_str().len()));
    for m in &mounts {
        let st = std::process::Command::new("umount")
            .arg("-l")
            .arg(m)
            .status();
        if !st.is_ok_and(|s| s.success()) {
            tracing::warn!(target: "quod", mount = %m.display(), "stale job mount not unmounted");
        }
    }
    for d in &stale {
        if let Some(id) = d.file_name().and_then(|n| n.to_str()) {
            secrets::JobSecrets::remove(state_dir, d, id);
        }
        quod_cas::remove_any(d).with_context(|| format!("remove stale job {}", d.display()))?;
    }
    // A job's secrets whose job directory is gone (removed by hand, or a
    // crash between the two removals): kept nowhere else, needed by no one.
    let shm = secrets::runtime_dir(state_dir);
    for e in std::fs::read_dir(&shm).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let id = name.split('.').next().unwrap_or("");
        if !id.is_empty() && !jobs.join(id).is_dir() {
            let _ = std::fs::remove_file(e.path());
        }
    }
    Ok(stale.len())
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

/// A job's repository (lowercase `owner/repo`), its trust scope, and
/// whether it may write the scope's shared state.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopeOf {
    repository: String,
    name: String,
    writer: bool,
}

/// The job's repository and trust scope, from the event payload (the
/// runner's event.json) and the event name (GITHUB_EVENT_NAME, which comes
/// from the step's environment and so is never trusted alone).
///
/// Trusted: an event that runs the repository's own code by its own hand
/// (push, workflow_dispatch, schedule), or a pull request or workflow_run
/// whose head repository is proven the base repository. The name must fit
/// the payload: a payload carrying `pull_request` or `workflow_run` is
/// never a push, dispatch or schedule; a push's has `pusher`, `after` and
/// `ref`, a schedule's `schedule`, a dispatch's `inputs`. A pull request or
/// workflow_run from another repository: that fork's own scope
/// (`fork-<owner_repo>`, `fork-unknown` when the head is gone). Anything
/// else (issue_comment, release, a name the payload does not fit, a
/// payload that does not name its repository): `unverified`. Untrusted
/// scopes never share a ledger, warm cache, module cache or workspace with
/// the trusted one.
///
/// Writer: a push or workflow_dispatch whose `ref` is the repository's
/// default branch or one of the operator's `writer_refs` (protected refs),
/// or a schedule (which GitHub runs only on the default branch). Only a writer's steps write the scope's warm build cache and
/// persisted workspace; every other job, a same-repository pull request's
/// included, works on a copy of them that is discarded.
fn scope_of(event_name: Option<&str>, event: Option<&[u8]>, writer_refs: &[String]) -> ScopeOf {
    let unverified = |repository: String| ScopeOf {
        repository,
        name: "unverified".into(),
        writer: false,
    };
    let Some(v) = event.and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok()) else {
        return unverified(String::new());
    };
    let name = |v: &serde_json::Value| {
        v.get("full_name")
            .and_then(|n| n.as_str())
            .map(str::to_ascii_lowercase)
    };
    let Some(repo) = v.get("repository").and_then(name) else {
        return unverified(String::new());
    };
    let has = |k: &str| v.get(k).is_some();
    let object = |k: &str| v.get(k).is_some_and(|x| x.is_object());
    let string = |k: &str| v.get(k).is_some_and(|x| x.is_string());
    let by_head = |head: Option<String>| ScopeOf {
        name: match head {
            Some(h) if h == repo => "trusted".to_string(),
            Some(h) => format!("fork-{}", slug(&h)),
            // A deleted fork: no head repository to trust.
            None => "fork-unknown".to_string(),
        },
        repository: repo.clone(),
        writer: false,
    };
    let on_default_branch = || {
        let default = v
            .get("repository")
            .and_then(|r| r.get("default_branch"))
            .and_then(|b| b.as_str())
            .filter(|b| !b.is_empty());
        let r#ref = v.get("ref").and_then(|r| r.as_str());
        matches!((default, r#ref), (Some(d), Some(r)) if r.strip_prefix("refs/heads/") == Some(d))
            || r#ref.is_some_and(|r| writer_refs.iter().any(|w| w == r))
    };
    let trusted = |writer: bool| ScopeOf {
        repository: repo.clone(),
        name: "trusted".into(),
        writer,
    };
    let (pr, run) = (has("pull_request"), has("workflow_run"));
    match event_name {
        Some(
            "pull_request"
            | "pull_request_target"
            | "pull_request_review"
            | "pull_request_review_comment",
        ) if object("pull_request") && !run => by_head(
            v.get("pull_request")
                .and_then(|pr| pr.get("head"))
                .and_then(|h| h.get("repo"))
                .and_then(name),
        ),
        Some("workflow_run") if object("workflow_run") && !pr => by_head(
            v.get("workflow_run")
                .and_then(|w| w.get("head_repository"))
                .and_then(name),
        ),
        // Never a push, dispatch or schedule if the payload is a pull
        // request's or a workflow run's.
        _ if pr || run => unverified(repo.clone()),
        Some("push") if object("pusher") && string("after") && string("ref") => {
            trusted(on_default_branch())
        }
        Some("workflow_dispatch") if has("inputs") => trusted(on_default_branch()),
        Some("schedule") if string("schedule") => trusted(true),
        _ => unverified(repo.clone()),
    }
}

/// May the job image's own config env exempt a variable from classification
/// by name or shape (`secrets::classify_with`)? An image's env is whatever
/// its builder wrote, so this trusts the builder: only for a job in a
/// trusted scope (a push, dispatch or schedule of the repository, or a
/// pull request from a branch of the repository itself; never a fork's or
/// an unverified event), and only for an image pinned by digest (the
/// workflow names exactly that config) or from one of the operator's
/// `trusted_registries` (`registry/namespace` prefixes, matched on path
/// components; the Docker Official Images by default). Anywhere else a
/// secret-shaped variable is classified whatever the image says.
pub fn image_env_exempts(scope_name: &str, image: &str, trusted_registries: &[String]) -> bool {
    if scope_name != "trusted" {
        return false;
    }
    let Ok(r) = crate::rootfs::ImageRef::parse(image) else {
        return false;
    };
    if r.reference.starts_with("sha256:") {
        return true;
    }
    let name = format!("{}/{}", r.registry, r.repository);
    trusted_registries.iter().any(|t| {
        name.strip_prefix(t.as_str())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
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
        tracing::warn!(options = ?ignored, "container options not honoured by the quod sandbox");
    }
    Ok((uid, gid))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// What of a finished job's tree the next job on the path starts from:
/// the `PERSIST` parts of `.quod/`, nothing else.
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

/// The tree with Quod's state directories present (empty if new), so
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
    let args = input.script_step_args()?;
    job.ensure_scope(
        &cas,
        args.environment_variables
            .get("GITHUB_EVENT_NAME")
            .map(String::as_str),
        &policy.writer_refs,
        &policy.trusted_registries,
    )?;
    // Trust state (proofs, audits, quarantine) lives in the scope's own
    // ledger, opened below: a fork's steps never prove a trusted key.
    job.check_secrets()?;
    let deferred = job.ensure_provenance(&args.environment_variables)?;
    job.learn_secrets(&args.environment_variables)?;
    job.steps += 1;
    job.totals.steps += 1;
    let n = job.steps;
    let step_dir = job.dir().join(format!("step-{n:03}"));
    std::fs::create_dir_all(&step_dir)?;
    job.save()?;
    let ledger = Ledger::open(&job.scope().join("ledger.sqlite"))?;
    let salt = secrets::salt(state_dir)?;

    let mut step = step::classify(&job, &args, policy)?;
    step.taint.extend(deferred);
    for why in &job.dirty {
        step.taint
            .push(format!("an earlier step of the job: {why}"));
    }
    let env_in = &args.environment_variables;

    let code = match step.kind.clone() {
        Kind::CheckoutPost if job.checkout_fallback => {
            // The real action ran; its post step removes the credentials it
            // persisted.
            let tree = job.step_tree(&cas)?;
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
            eprintln!("quod: {} is a no-op for a CAS checkout", step.label);
            0
        }
        Kind::Checkout(inputs) => {
            let t0 = Instant::now();
            let prev = job.load_tree()?;
            let c = checkout::checkout(state_dir, &cas, &inputs)
                .with_context(|| format!("checkout {}@{}", inputs.repository, inputs.reference))?;
            // The checkout replaces the repository; the workspace's own
            // state under .quod/ carries over (a self-hosted runner's
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
            tracing::info!(target: "quod", job = %job.id, build_ms = (t1 - t0).as_millis() as u64, save_ms = t1.elapsed().as_millis() as u64, "checkout phases");
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
            println!("quod: {line}");
            job.summary(env_in, &format!("- {line}"));
            job.totals.would_replay += 1;
            job.totals.replayed += u32::from(policy.mode == Mode::Replay);
            tracing::info!(target: "quod", job = %job.id, step = n, kind = "checkout", repository = %inputs.repository, sha = %c.sha, files, memo = c.memo, ms = c.elapsed.as_millis() as u64, "step");
            0
        }
        Kind::Action(a) if step.tainted() && a.name.starts_with("actions/checkout@") => {
            job.checkout_fallback = true;
            let tree = job.step_tree(&cas)?;
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
    // The names this step set in GITHUB_ENV, as the runner will read them
    // (whatever surface wrote or replayed them into its file).
    if let Some(f) = env_in.get("GITHUB_ENV").and_then(|p| job.command_file(p))
        && let Ok(b) = std::fs::read(&f)
    {
        job.set_by_steps.extend(surface::env_file_names(&b));
    }
    // What a run staged and did not record is not kept.
    for d in ["stage", "online/stage"] {
        let _ = quod_cas::remove_any(&step_dir.join(d));
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
    tracing::info!(target: "quod", job = %job.id, step = n, label = %job.log_label(&step), hook_ms = step_start.elapsed().as_millis() as u64, "hook");
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
    let stamp = host_copy_stamp(job, &c.sha);
    let current = std::fs::read_to_string(&marker).unwrap_or_default();
    let populated = std::fs::read_dir(&host).is_ok_and(|mut d| d.next().is_some());
    if current.trim() == stamp && populated {
        return Ok(());
    }
    let t = Instant::now();
    let _ = std::fs::remove_file(&marker);
    std::fs::create_dir_all(&host)?;
    for e in std::fs::read_dir(&host)? {
        quod_cas::remove_any(&e?.path())?;
    }
    cas.materialize(&c.tree, &host)
        .with_context(|| format!("materialize checkout at {}", host.display()))?;
    std::fs::create_dir_all(marker.parent().unwrap())?;
    std::fs::write(&marker, &stamp)?;
    tracing::info!(target: "quod", job = %job.id, ms = t.elapsed().as_millis() as u64, "host checkout copy");
    Ok(())
}

/// The key's line naming the variables a keyed step does not get as they
/// are. Versioned by what the step sees of them: v1 removed them, v2 sets
/// them to `surface::WITHHELD`, so a result recorded under v1 (a step that
/// found no token) never answers a v2 key; v3 is the rename to Quod, the
/// placeholder's value (`quod-withheld`) changed with it.
pub const WITHHELD_KEY: &str = "quod:withheld v3";

fn withheld_key_line(names: &[String]) -> String {
    format!("{WITHHELD_KEY} {}", names.join(","))
}

/// A file entry's blob.
fn file_blob(e: &Entry) -> Option<Hash> {
    (e.k == EntryKind::File).then_some(e.h).flatten()
}

/// Moves blobs a run staged (`exec::Exec::stage`) into the shared store,
/// once the run is recordable or its outputs carry the job. Durable before
/// anything names them (`Cas::take_files`, one filesystem sync).
fn promote(cas: &Cas, stage: &Cas, hashes: impl IntoIterator<Item = Hash>) -> anyhow::Result<()> {
    let mut want = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    for h in hashes {
        if cas.has(&h) || !seen.insert(h) {
            continue;
        }
        let p = stage.blob_path(&h);
        if !p.is_file() {
            bail!("blob {h} is neither staged nor in the store");
        }
        want.push(h);
        files.push(p);
    }
    if files.is_empty() {
        return Ok(());
    }
    let taken = cas.take_files(&files, quod_fs::syncfs)?;
    for (w, (got, _)) in want.iter().zip(&taken) {
        if w != got {
            bail!("staged blob {w} hashed as {got}");
        }
    }
    Ok(())
}

/// Values the run registered with `::add-mask::`: the job's secrets from
/// now on (masked in everything written about it and every later step).
fn learn_add_masks(job: &mut Job, ex: &Executed) -> anyhow::Result<()> {
    if ex.add_masks.is_empty() {
        return Ok(());
    }
    let mut masks = Vec::new();
    job.had_secrets = true;
    for v in &ex.add_masks {
        if v.len() < secrets::MIN_SECRET_LEN {
            job.short_secret
                .get_or_insert_with(|| "a value registered with ::add-mask::".to_string());
        } else {
            job.secrets.values.insert(v.clone());
        }
        masks.extend(secrets::masks_for(v));
    }
    job.add_masks(&masks);
    job.save_secrets()
}

/// Whose copy of the checkout the host directory holds. It is reused only
/// by a job of the scope and kind that made it: a runner-surface step of
/// another scope (a fork's) or of a job that is not a writer (a pull
/// request of the repository itself) could have changed it, and the runner
/// reads local actions from it.
fn host_copy_stamp(job: &Job, sha: &str) -> String {
    format!(
        "{} {} {} {sha}",
        job.repository,
        job.scope_name,
        if job.writer { "writer" } else { "reader" }
    )
}

/// The run of one step on one surface.
#[allow(clippy::too_many_arguments)]
fn execute(
    job: &Job,
    args: &ScriptStepArgs,
    s: &Surface,
    cas: &Cas,
    stage: &Cas,
    tree: &Tree,
    dir: &Path,
    keep: Keep,
    echo: bool,
    warm: bool,
) -> anyhow::Result<Executed> {
    let hermetic = s.kind == surface::SurfaceKind::Hermetic;
    let has_gocache = s.env.iter().any(|(k, _)| k == "GOCACHE");
    // The scope's warm build cache only for a keyed step that runs go (the
    // hooks' `--warm`), checked after the step: in place for a writer, under
    // the job's own layer for any other job. The runner's and the network's
    // surfaces (never recorded) get the job's scratch cache; any other keyed
    // step none (its GOCACHE is in the observed workspace).
    let gocache = match (hermetic, warm && has_gocache, has_gocache) {
        (true, true, _) if job.writer => GoCache::Shared(job.warm_gocache()),
        (true, true, _) => GoCache::Layered {
            lower: job.warm_gocache(),
            layer: job.warm_layer(),
        },
        (true, false, _) | (false, _, false) => GoCache::None,
        (false, _, true) => GoCache::Scratch(job.scratch_gocache()),
    };
    exec::execute(&Exec {
        cas,
        stage,
        // A step given a classified secret, or any step of an opaque job
        // (whose secrets Quod cannot name), stores no workspace outputs.
        secret_given: !s.classified.is_empty() || job.provenance().opaque.is_some(),
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
        let why = job.mask(&format!("wrote outside the workspace ({first})"));
        job.dirty.push(why.clone());
        reasons.push(why);
    }
    if let Some(l) = &ex.ws_layer {
        job.ws_layers.insert(0, l.clone());
        let why = job.mask(
            ex.secret_output
                .as_deref()
                .unwrap_or("a step that had the job's secrets wrote to the workspace"),
        );
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
            let why = job.mask(&format!("left files in RUNNER_TEMP ({})", left.join(", ")));
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
    if !s.classified.is_empty() {
        job.had_secrets = true;
    }
    let host_temp = step::temp_dir(job).and_then(|t| job.host_path(&t));
    let before = host_temp.as_deref().map(surface::temp_snapshot);
    // Keep::Layer: nothing is stored, the staging store is never written.
    let ex = execute(
        job,
        args,
        &s,
        cas,
        cas,
        tree,
        &dir.join("runner"),
        Keep::Layer,
        true,
        false,
    )?;
    job.totals.executed += 1;
    job.totals.tainted += 1;
    learn_add_masks(job, &ex)?;
    keep_side_effects(job, &ex, None, &mut taint)?;
    if let (Some(before), Some(ht)) = (before, host_temp.as_deref()) {
        let after = surface::temp_snapshot(ht);
        let changed: Vec<&String> = after
            .iter()
            .filter(|(k, v)| before.get(*k) != Some(v))
            .map(|(k, _)| k)
            .collect();
        if !changed.is_empty() {
            let why = job.mask(&format!(
                "left files in RUNNER_TEMP ({})",
                changed
                    .iter()
                    .take(3)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            job.dirty.push(why.clone());
            taint.push(why);
        }
    }
    if let Some(why) = &ex.resource_failure {
        taint.push(why.clone());
    }
    let taint: Vec<String> = taint.iter().map(|t| job.mask(t)).collect();
    let line = job.mask(&format!(
        "[executed] `{}`: {} in {}; would replay: no, tainted: {}",
        step.label,
        if ex.exit_code == 0 { "ok" } else { "failed" },
        fmt_duration(ex.wall),
        taint.join("; ")
    ));
    eprintln!("QUOD  EXECUTED  {line}");
    job.summary(&args.environment_variables, &format!("- {line}"));
    tracing::info!(target: "quod", job = %job.id, step = job.steps, label = %job.log_label(step), decision = "tainted", taint = ?taint, exit_code = ex.exit_code, exec_ms = ex.wall.as_millis() as u64, "step");
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
    let stage = Cas::open(dir.join("stage"))?;
    let ex = execute(
        job,
        args,
        &s,
        cas,
        &stage,
        tree,
        dir,
        Keep::Store,
        true,
        false,
    )?;
    let ok = ex.exit_code == 0 && ex.signal.is_none() && ex.resource_failure.is_none();
    job.totals.executed += 1;
    job.totals.tainted += 1;
    learn_add_masks(job, &ex)?;
    // Its writes carry the rest of the job (it held no secrets); its logs
    // and command files are never stored.
    if let Some(outputs) = &ex.outputs {
        promote(cas, &stage, outputs.iter().filter_map(file_blob))?;
        job.save_tree(&tree.with_outputs(outputs))?;
    }
    let (_, file_taint) = surface::hand_back(&s.files, &stage, &s.masks)?;
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
    for t in taint.iter_mut() {
        *t = job.mask(t);
    }
    let line = job.mask(&format!(
        "[executed] `{}`: {} in {} with the network; would replay: no, tainted: {}",
        step.label,
        if ok { "ok" } else { "failed" },
        fmt_duration(ex.wall),
        taint.join("; ")
    ));
    eprintln!("QUOD  EXECUTED  {line}");
    job.summary(&args.environment_variables, &format!("- {line}"));
    tracing::info!(target: "quod", job = %job.id, step = job.steps, label = %job.log_label(step), decision = "online", taint = ?taint, exit_code = ex.exit_code, exec_ms = ex.wall.as_millis() as u64, "step");
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
    let tree = job.step_tree(cas)?;
    let load_ms = t0.elapsed().as_millis() as u64;
    if policy.keep_jobs {
        std::fs::write(step_dir.join("tree.json"), tree_to_json(&tree)?)?;
    }
    if step.tainted() {
        return run_tainted(job, args, step, cas, &tree, step_dir, step.taint.clone());
    }
    let workspace = job.working_directory.clone();
    let h = surface::hermetic(job, args, step, step_dir, cas, salt)?;
    // What this step was given joins the job's known secrets: later steps
    // mask it, and treat a copy of it under another name as one.
    job.add_masks(&h.masks);
    if !h.classified.is_empty() || job.provenance().opaque.is_some() {
        job.had_secrets = true;
    }
    if !h.secret_values.is_subset(job.known_secrets()) {
        job.secrets.values.extend(h.secret_values.iter().cloned());
        job.save_secrets()?;
    }
    if !h.taint.is_empty() {
        return run_tainted(job, args, step, cas, &tree, step_dir, h.taint.clone());
    }
    let image = job.key_image()?;
    // The scope's warm build cache, only for a step that runs go.
    // Never for a step given a classified secret, nor in an opaque job (whose
    // secrets Quod may not name): the warm cache outlives the job, and a
    // derived form of a secret (compressed, in an object file) is not caught
    // by the masks. Such a step builds with the observed cache in the
    // workspace, whose writes become a job layer.
    let warm = step.runs_go()
        && h.env.iter().any(|(k, _)| k == "GOCACHE")
        && h.classified.is_empty()
        && job.provenance().opaque.is_none();
    // A job that is not a writer whose earlier step wrote its layer over the
    // warm cache: this step reads what that step's code put there, which no
    // key covers. It may replay what a clean run recorded; its own run is
    // neither recorded nor compared (a pull request must not be able to
    // prove, or quarantine, a key the default branch uses).
    let unrecorded: Option<String> = (warm && !job.writer)
        .then(|| job.warm_layer_written.clone())
        .flatten()
        .map(|p| {
            format!(
                "not recorded: an earlier step of this job wrote {p} into its own layer over the warm build cache (only a default-branch push, dispatch or schedule writes the cache)"
            )
        })
        // The workflow uses a secret in a way no name follows: no step of
        // the job is recorded (each may still replay a clean job's result).
        .or_else(|| {
            job.provenance()
                .opaque
                .as_ref()
                .map(|why| format!("not recorded: the job is opaque ({why})"))
        })
        .or_else(|| {
            h.short_secret.as_ref().map(|n| {
                format!(
                    "not recorded: the secret in {n} is shorter than {} bytes, too short to mask reliably",
                    secrets::MIN_SECRET_LEN
                )
            })
        })
        .or_else(|| {
            job.short_secret.as_ref().map(|w| {
                format!(
                    "not recorded: an earlier step of the job had a secret too short to mask ({w}), which this one could print unmasked"
                )
            })
        })
        .map(|u| job.mask(&u));
    if h.short_strong
        && job.short_secret.is_none()
        && let Some(n) = &h.short_secret
    {
        job.short_secret = Some(format!("in {n}"));
    }
    let mut key_cmd = step.cmd.clone();
    key_cmd.extend(h.key_extra.iter().cloned());
    // Volatile names are withheld or given fixed placeholders, and
    // RUNNER_TEMP is pinned: none of them goes in the key (a key refuses the
    // DENYLIST), and the manifest lists them as present-but-unkeyed.
    let key_env: Vec<(String, String)> = h
        .key_env
        .iter()
        .filter(|(k, _)| !quod_key::unkeyed(k))
        .cloned()
        .collect();
    let mut unkeyed: Vec<String> = h
        .withheld
        .iter()
        .cloned()
        .chain(
            h.key_env
                .iter()
                .filter(|(k, _)| quod_key::unkeyed(k))
                .map(|(k, _)| k.clone()),
        )
        .collect();
    unkeyed.sort();
    unkeyed.dedup();
    // Their values stay out, but which of them the job has does not: a
    // step can branch on a variable's presence (a test that skips without
    // GITHUB_TOKEN, an action that asks for an OIDC token only when
    // ACTIONS_ID_TOKEN_REQUEST_URL is set), so a result recorded without
    // one must not replay for a job that has it. Names only, never values.
    key_cmd.push(withheld_key_line(&unkeyed));
    let spec = StepSpec {
        image,
        // The hooks build the whole env themselves; the toolchain names the
        // profile whose warm knobs they merge (Go's).
        toolchain: Toolchain::Go,
        cmd: key_cmd,
        cwd: args.working_directory.clone(),
        env: key_env,
        roots: vec![workspace.clone()],
        warm,
    };
    let key = step_key(&spec)?;
    let keyed_env = spec.normalized_env()?.len();
    let run_id = ulid::Ulid::generate().to_string();

    let surface_ms = t0.elapsed().as_millis() as u64 - load_ms;
    // The trust layer decides, as for `quod run`: a matching manifest
    // replays only once proven (`proofs` identical executions, the policy's
    // or the step pattern's), never as a failure, never while quarantined;
    // replay mode audits on the schedule (shadow, decay, floor).
    let trust = Trust::new(ledger, cas, policy.trust_for(step.pattern_text()))?;
    let label = job.log_label(step);
    let ignore = ignored_outputs(&h.env, &workspace);
    let tstep = TrustStep {
        key,
        cmd: &label,
        ignore: &ignore,
    };
    let mode = policy.mode_name();
    // The scope's own ledger, or a Quod server's when the runner's
    // environment sets QUOD_SERVER (the same calls, on the server's
    // stores, in the scope this job decided).
    let local = LocalLedger {
        ledger,
        trust: &trust,
    };
    let remote = remote_ledger(job, &args.environment_variables, cas, policy.trust());
    let store: &dyn StepLedger = match &remote {
        Some(r) => r,
        None => &local,
    };
    // Looked up and decided; a replay is decided again as it is recorded
    // (another job may have quarantined the key or taken the audit slot
    // meanwhile), with its run row, in one transaction.
    let t1 = Instant::now();
    let (lookup, claimed) = {
        let job: &Job = job;
        // The hooks' policy never defers an audit (`Policy::trust`): no
        // deferral callback, which `LocalLedger` checks against the policy.
        store.claim(
            &tstep,
            &tree,
            &mut |r| {
                RunRecord::from_facts(&run_facts(
                    &run_id,
                    &key,
                    job,
                    mode,
                    "replayed",
                    Some(&r.manifest_id),
                    Some(r.exit_code),
                    None,
                    None,
                ))
            },
            None,
        )?
    };
    let lookup_ms = t1.elapsed().as_millis() as u64;
    tracing::info!(target: "quod", job = %job.id, step = job.steps, load_ms, surface_ms, lookup_ms, candidates = lookup.candidates, "step phases");
    let candidates = lookup.candidates;
    let closest = lookup
        .closest
        .as_ref()
        .map(|(_, m)| (m.path.clone(), m.to_string()));
    let replayed = match claimed {
        Claimed::Replay { facts, .. } => Ok(facts),
        Claimed::Execute { reason } => Err(reason),
    };
    let reason = match replayed {
        Ok(facts) => {
            let (manifest, result) = lookup.hit.as_ref().expect("a replay needs a hit");
            let t = Instant::now();
            // Only what the proofs vouch for: the scratch the comparison
            // skipped (HOME_VOLATILE, the build cache) is not served.
            let served: Vec<Entry> = quod_trust::comparable_outputs(&result.outputs, &ignore)
                .into_iter()
                .cloned()
                .collect();
            if !served.is_empty() {
                job.save_tree(&tree.with_outputs(&served))?;
            }
            apply_commands(job, &args.environment_variables, cas, result)?;
            let code = quod_replay::replay_log(
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
                quod_replay::group(manifest.entries.len()),
                facts.receipt_line()
            );
            eprint!(
                "{}",
                quod_replay::receipt(
                    result,
                    manifest.entries.len(),
                    t.elapsed(),
                    &[facts.receipt_line()]
                )
            );
            job.summary(&args.environment_variables, &format!("- {line}"));
            tracing::info!(target: "quod", job = %job.id, step = job.steps, run_id, step_key = %job.log_key(&key), manifest_id = %result.manifest_id, label = %label, hit = true, decision = "replay", proofs = facts.streak, recorded_ms = result.duration_ms, replay_ms = t.elapsed().as_millis() as u64, exit_code = code, "step");
            return Ok(code);
        }
        Err(reason) => reason,
    };

    // A step that reached for the network on an earlier offline try runs
    // where it ended then, without that try (it would only be set aside
    // again, after costing its whole run): never recorded, as before. Only
    // while what that try read is unchanged (`same_inputs`): a step's need
    // for the network can depend on its inputs (a test that reaches out only
    // for some fixtures), so on other inputs it tries offline again. Every
    // `NETWORK_PROBE_EVERY`th run tries offline anyway, so a step that no
    // longer needs the network records again. A run that is not recorded
    // (`unrecorded`) reads this memory but never writes it.
    let network = ledger.network(&key)?;
    if let Some(n) = network
        .as_ref()
        .filter(|n| unrecorded.is_some() || (skips_offline(n) && same_inputs(n, &tree)))
    {
        if unrecorded.is_none() {
            ledger.network_skipped(&key)?;
        }
        let online = n.surface == NETWORK_SURFACE && step.retry.is_some();
        let note = job.mask(&format!(
            "needed the network on an earlier run ({}); ran {} without trying offline first",
            n.reason,
            if online {
                "with the network"
            } else {
                "with the runner's environment"
            }
        ));
        eprintln!("quod: `{}`: {note}", step.label);
        let mut taint = vec![note];
        if online
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

    // Execute on the hermetic surface (observe mode, a miss, or an audit).
    // A step that may need the network is captured, not streamed, until it
    // is known to have managed without. What it writes and prints is staged
    // in the step's own store until the run is judged.
    let stage = Cas::open(step_dir.join("stage"))?;
    let ex = execute(
        job,
        args,
        &h,
        cas,
        &stage,
        &tree,
        step_dir,
        Keep::Store,
        step.retry.is_none(),
        warm,
    )?;
    if let Some(why) = &ex.warm_violation {
        // A writer's step wrote the cache itself, shared with later steps
        // and jobs of the scope: start it over rather than keep whatever the
        // step put there. Any other job's step wrote only the job's layer:
        // that goes, and the cache is untouched.
        let (dir, what) = if job.writer {
            (job.warm_gocache(), "the scope's warm build cache was wiped")
        } else {
            job.warm_layer_written = None;
            (
                job.warm_layer(),
                "the job's layer over the warm build cache was discarded",
            )
        };
        if let Err(e) = std::fs::remove_dir_all(&dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(target: "quod", dir = %dir.display(), error = %e, "warm build cache not wiped");
        }
        eprintln!("quod: `{}`: {}; {what}", step.label, job.mask(why));
    } else if job.warm_layer_written.is_none()
        && let Some(p) = &ex.warm_layer_written
    {
        job.warm_layer_written = Some(p.clone());
    }

    learn_add_masks(job, &ex)?;
    // Reasons the offline run is not the step's result: it is run again.
    let mut again: Vec<String> = Vec::new();
    if ex.exit_code != 0 {
        again.push(format!("exited {} offline", ex.exit_code));
    }
    match ex.net_attempts {
        Some(0) => {
            if network.is_some() && unrecorded.is_none() {
                ledger.clear_network(&key)?;
            }
        }
        Some(n) => {
            let why = format!("tried to reach the network offline ({n} attempts)");
            // Where it runs next (below): the network surface for a fetch
            // that failed offline, the runner's otherwise.
            if unrecorded.is_none() && ex.resource_failure.is_none() && ex.signal.is_none() {
                let surface = if step.retry.is_some() && ex.exit_code != 0 {
                    NETWORK_SURFACE
                } else {
                    "runner"
                };
                let paths = network_inputs(&ex.log);
                ledger.mark_network(
                    &key,
                    surface,
                    &why,
                    &serde_json::to_string(&paths)?,
                    &inputs_digest(&tree, &paths),
                )?;
            }
            again.push(why);
        }
        None => again.push("its network attempts could not be counted".to_string()),
    }
    if let Some(sig) = ex.signal {
        again.push(format!("killed by signal {sig}"));
    }
    let machine = ex.resource_failure.clone();
    if let Some(why) = &machine {
        again.push(why.clone());
    }
    // A result that holds a placeholder (a withheld variable printed, put
    // in GITHUB_OUTPUT or a file, by a name no scan sees) is not what
    // GitHub would give: the job gets the step run with its real
    // environment, and nothing is recorded.
    if again.is_empty()
        && let Some(w) = ex
            .placeholder_in
            .clone()
            .or_else(|| surface::placeholder_in_files(&h.files).map(|n| format!("its {n}")))
    {
        again.push(format!(
            "{w} holds a withheld variable's placeholder ({}), not the value GitHub would give",
            surface::WITHHELD
        ));
    }
    if again.is_empty() {
        // A file the step read or executed names a variable it was not
        // given: on GitHub it would have had it.
        // Files only: a directory entry's hash is its listing, not a blob.
        // From the sealed module cache, which is recorded whole rather than
        // as read, only Go sources: code that could read the environment
        // (its data files, a sigstore example JSON, a go-gh workflow, name
        // GITHUB_SHA and friends without anything reading them). Not the
        // files whose content the step was never served (`ex.unopened`:
        // only stat()ed, as `git status` does every tracked file over a
        // served index, or over-recorded and never opened): a stat shows
        // no names.
        let file_hash = |e: &Entry| (e.k == EntryKind::File).then_some(e.h).flatten();
        let sealed = format!("{STATE_SUBDIR}/gomod/");
        let mut files: BTreeMap<String, Hash> = ex
            .log
            .iter()
            .filter(|a| a.kind == EntryKind::File && !ex.unopened.contains(&a.path))
            .filter(|a| !a.path.starts_with(&sealed) || quod_key::go_source_file(&a.path))
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
        // the step behaving otherwise, a divergence. Not for a run that is
        // not recorded: it read what its own job wrote.
        let again_why = job.mask(&again.join("; "));
        if let Some(hit) = lookup.hit.as_ref().filter(|_| unrecorded.is_none()) {
            if machine.is_some() || ex.signal.is_some() {
                if let Some(line) = store.commit_incomplete(&tstep, Some(hit), &again_why, None)? {
                    eprintln!("quod: `{}`: {line}", step.label);
                }
            } else {
                let v = record_divergence(
                    store, &tstep, job, mode, &run_id, hit, &reason, &again_why, &ex,
                )?;
                job.totals.audits += 1;
                job.totals.divergences += 1;
                for l in v.lines() {
                    eprintln!("{l}");
                }
            }
        }
        if step.retry.is_some() && !step.tainted() {
            print_captured(step_dir)?;
        }
        let note = format!(
            "{again_why}; ran again{}",
            if step.retry.is_some() && ex.exit_code != 0 {
                " with the network"
            } else {
                " with the runner's environment"
            }
        );
        eprintln!("quod: `{}`: {note}", step.label);
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
    let (commands, file_taint) = surface::hand_back(&h.files, &stage, &job.masks)?;
    let mut taint = file_taint;
    // A part of a secret it hands on to later steps (a password cut out of
    // a known DATABASE_URL into GITHUB_ENV): no encoding of the secret
    // matches it, so the file is checked against the secrets themselves.
    let whole: Vec<&str> = job
        .known_secrets()
        .iter()
        .chain(h.secret_values.iter())
        .map(String::as_str)
        .collect();
    if let Some(f) = surface::part_handed_on(&h.files, &whole) {
        taint.push(format!("{f} carries a part of a secret value"));
    }
    if let Some(why) = &ex.warm_violation {
        taint.push(why.clone());
    }
    if !ex.add_masks.is_empty() {
        taint.push("it registered a secret (::add-mask::)".to_string());
    }
    // A workflow command line (`::set-output`, `::save-state`, ...) that
    // holds a secret: replayed from the masked log it would set `***`
    // where a fresh run sets the value.
    let raw = |f: &str| std::fs::read(step_dir.join(f)).unwrap_or_default();
    if secrets::command_line_holds(&raw("stdout"), &job.masks)
        || secrets::command_line_holds(&raw("stderr"), &job.masks)
    {
        taint.push("a workflow command line it printed holds a secret".to_string());
    }
    // A path it looked up (an absent one too) or wrote that holds a
    // secret: the manifest would store it.
    if ex
        .log
        .iter()
        .any(|a| secrets::holds(a.path.as_bytes(), &job.masks))
    {
        taint.push("a path it looked up holds a secret value".to_string());
    }
    if let Some(outputs) = &ex.outputs {
        if outputs.iter().any(|e| {
            secrets::holds(e.p.as_bytes(), &job.masks)
                || e.t
                    .as_ref()
                    .is_some_and(|t| secrets::holds(t.as_bytes(), &job.masks))
        }) {
            taint.push("an output's name or link target holds a secret value".to_string());
        }
        // Its writes carry the rest of the job: into the store now.
        promote(cas, &stage, outputs.iter().filter_map(file_blob))?;
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
    for t in taint.iter_mut() {
        *t = job.mask(t);
    }
    let not_recordable: Option<String> = match (&ex.outputs, ex.stdout, ex.stderr) {
        (Some(_), Some(_), Some(_)) if taint.is_empty() => None,
        (Some(_), Some(_), Some(_)) => Some(taint.join("; ")),
        // Kept as a layer (a step given secrets wrote, or an output held one).
        _ => Some(job.mask(ex.secret_output.as_deref().unwrap_or("kept as a layer"))),
    };
    let verdict = match (&not_recordable, &ex.outputs, ex.stdout, ex.stderr) {
        // Not recorded, not compared: no verdict.
        _ if unrecorded.is_some() => {
            if not_recordable.is_some() {
                job.totals.tainted += 1;
            }
            None
        }
        (Some(why), ..) => {
            job.totals.tainted += 1;
            match lookup.hit.as_ref() {
                Some(hit) => {
                    let v = record_divergence(
                        store, &tstep, job, mode, &run_id, hit, &reason, why, &ex,
                    )?;
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
            let mid = quod_manifest::manifest_id(&manifest)?;
            let result = StepResult {
                v: quod_manifest::VERSION,
                step_key: key,
                manifest_id: mid,
                run_id: run_id.clone(),
                recorded_at: quod_ledger::now_rfc3339(),
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
            // Recordable: what it printed and wrote to its command files
            // into the store, then one transaction: the trust verdict (proof
            // streak, trust state, audit row), the run row, the manifest and
            // the result.
            promote(
                cas,
                &stage,
                [stdout, stderr]
                    .into_iter()
                    .chain(commands.iter().map(|c| c.h)),
            )?;
            // The key's JSON, written with the result only (a run that is
            // not recorded may have shown a secret, `::add-mask::`, only as
            // it ran), masked: its secrets are salted already; this catches
            // one in the command. An opaque job records nothing, so never
            // writes one.
            store.put_step_key(&key, &job.mask(&quod_key::canonical_key_json(&spec)?))?;
            // One transaction (record_execution_facts inserts the
            // manifest); the run row's outcome and would-be saving come
            // from the verdict.
            let v: Verdict = store.commit_execution(
                &tstep,
                &run_id,
                &manifest,
                &result,
                lookup.hit.as_ref(),
                &reason,
                RunRecord::from_facts(&RunFacts {
                    // The hooks observer is always the classic fuse
                    // transport (quod_fs::serve, never the ring).
                    observed: Observed {
                        transport: Some("fuse"),
                        observer: Some(&ex.observer),
                    },
                    ..run_facts(
                        &run_id,
                        &key,
                        job,
                        mode,
                        "",
                        Some(&mid),
                        Some(ex.exit_code),
                        Some(ex.wall),
                        None,
                    )
                }),
            )?;
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
        unrecorded.as_deref(),
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

/// How often a step that needs the network tries offline again: every
/// this many runs of its key (the others skip the offline try).
pub const NETWORK_PROBE_EVERY: u32 = 10;

/// The ledger's name for the network surface (`Ledger::mark_network`).
const NETWORK_SURFACE: &str = "network";

/// Does this run of a step that needed the network skip its offline try?
/// All but every `NETWORK_PROBE_EVERY`th.
fn skips_offline(n: &quod_ledger::NetworkRow) -> bool {
    n.skipped + 1 < NETWORK_PROBE_EVERY
}

/// The workspace paths an offline try read (sorted, unique): what the
/// network memory keeps of it (`Ledger::mark_network`).
fn network_inputs(log: &[quod_manifest::Access]) -> Vec<String> {
    let paths: std::collections::BTreeSet<&str> = log.iter().map(|a| a.path.as_str()).collect();
    paths.into_iter().map(String::from).collect()
}

/// `paths` as `tree` has them (each entry, or its absence), hashed.
fn inputs_digest(tree: &Tree, paths: &[String]) -> String {
    let mut h = blake3::Hasher::new();
    for p in paths {
        h.update(p.as_bytes());
        h.update(b"\0");
        match tree.get(p) {
            Some(e) => h.update(&serde_json::to_vec(e).expect("an entry serializes")),
            None => h.update(b"absent"),
        };
        h.update(b"\n");
    }
    h.finalize().to_hex().to_string()
}

/// Did the offline try that set `n` read what this run's tree holds? A row
/// without inputs never matches: the run tries offline.
fn same_inputs(n: &quod_ledger::NetworkRow, tree: &Tree) -> bool {
    let (Some(inputs), Some(digest)) = (&n.inputs, &n.digest) else {
        return false;
    };
    serde_json::from_str::<Vec<String>>(inputs).is_ok_and(|p| inputs_digest(tree, &p) == *digest)
}

/// `; replayable from the next matching run (its next 2 would-be hits are
/// audited)` when this execution's manifest may replay from now on (trust
/// v2: after its first record), else nothing.
fn replayable_note(v: &Verdict) -> String {
    if !v.replayable() {
        return String::new();
    }
    match v.shadow_left {
        0 => "; replayable from the next matching run".to_string(),
        1 => {
            "; replayable from the next matching run (its next would-be hit is audited)".to_string()
        }
        n => format!(
            "; replayable from the next matching run (its next {n} would-be hits are audited)"
        ),
    }
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
    unrecorded: Option<&str>,
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
                "; **audit DIVERGED** from run {}: key quarantined ({d}){}",
                v.compared_with.as_deref().map_or("?".to_string(), short),
                match v.suspects {
                    0 => String::new(),
                    1 => "; 1 earlier replay of this step may have been wrong: re-run it".into(),
                    n => format!(
                        "; {n} earlier replays of this step may have been wrong: re-run them"
                    ),
                }
            );
        } else if let (Reason::Audit { shadow, .. }, Some(r)) = (reason, &v.compared_with) {
            audit_note = format!(
                "; {}audit: agrees with run {}",
                if *shadow { "shadow " } else { "" },
                short(r)
            );
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
            Reason::WouldReplay | Reason::Audit { .. } | Reason::DeferredAudit { .. } => {
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
                replayable_note(v)
            ),
            Reason::Miss if candidates == 0 => format!(
                "would replay: no, first run of this step ({} of {} identical proofs){}",
                v.streak,
                v.needed,
                replayable_note(v)
            ),
            Reason::Miss => {
                let (path, mm) = closest.clone().unwrap_or_default();
                // Not `path: reason`: a problem matcher (setup-go's for Go)
                // would take that for a compiler error and annotate the run.
                let why = mm.strip_prefix(&format!("{path}: ")).unwrap_or(&mm);
                format!(
                    "would replay: no, closest manifest differed at {path} [{why}] ({} of {} identical proofs){}",
                    v.streak,
                    v.needed,
                    replayable_note(v)
                )
            }
        },
        (None, None) => unrecorded.map_or("would replay: no".to_string(), |u| {
            format!("would replay: no, {u}")
        }),
    };
    let line = job.mask(&format!(
        "[executed] `{}`: {} in {}; {}{}",
        step.label,
        if ex.exit_code == 0 { "ok" } else { "failed" },
        fmt_duration(ex.wall),
        verdict_line,
        audit_note
    ));
    eprintln!("QUOD  EXECUTED  {line}");
    if let Some(v) = verdict {
        for l in v.lines() {
            eprintln!("  {}", job.mask(&l));
        }
    }
    job.summary(&args.environment_variables, &format!("- {line}"));
    tracing::info!(
        target: "quod",
        job = %job.id,
        step = job.steps,
        run_id,
        step_key = %job.log_key(key),
        label = %label,
        reason = ?reason,
        outcome = verdict.map(|v| v.outcome()),
        would_replay = would,
        taint = ?taint,
        candidates,
        first_mismatch = closest.as_ref().map(|(p, _)| job.mask(p)),
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

/// What quod-trust leaves out when it compares two results of a hooks step:
/// `quod run`'s rule (quod-cli `ignored_outputs`) on the hooks' own
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
            out.extend(
                quod_trust::HOME_VOLATILE
                    .iter()
                    .map(|v| format!("{dir}/{v}")),
            );
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

/// `Trust::record_divergence` and the run's row (outcome `diverged`, with
/// the transport that observed it), in one transaction, so `quod
/// status` counts the divergence.
#[allow(clippy::too_many_arguments)]
fn record_divergence(
    store: &dyn StepLedger,
    tstep: &TrustStep,
    job: &Job,
    mode: &str,
    run_id: &str,
    hit: &(Manifest, StepResult),
    reason: &Reason,
    why: &str,
    ex: &Executed,
) -> anyhow::Result<Verdict> {
    // The run row's outcome comes from the verdict.
    Ok(store.commit_divergence(
        tstep,
        run_id,
        hit,
        reason,
        why,
        RunRecord::from_facts(&RunFacts {
            observed: Observed {
                transport: Some("fuse"),
                observer: Some(&ex.observer),
            },
            ..run_facts(
                run_id,
                &tstep.key,
                job,
                mode,
                "",
                None,
                Some(ex.exit_code),
                Some(ex.wall),
                None,
            )
        }),
    )?)
}

/// A Quod server's step ledger, when the runner's own environment (the
/// hook process's, which a workflow cannot set) has QUOD_SERVER. The
/// runner token must be a delegated one: the job's scope is the one this
/// job decided from its event payload, as with the local ledger.
fn remote_ledger<'a>(
    job: &Job,
    step_env: &std::collections::BTreeMap<String, String>,
    cas: &'a Cas,
    policy: quod_trust::Policy,
) -> Option<RemoteLedger<'a>> {
    Some(match ServerConfig::from_env(|k| std::env::var(k).ok())? {
        Ok(mut cfg) => {
            cfg.scope = Some(ScopeHint {
                repository: job.repository.clone(),
                trust: job.scope_name.clone(),
                writer: job.writer,
            });
            // For the dashboard only; the server keys nothing on it.
            cfg.ci = Some(CiContext::from_env(|k| step_env.get(k).cloned()));
            RemoteLedger::new(cfg, cas, policy)
        }
        Err(why) => RemoteLedger::unusable(why, cas, policy),
    })
}

/// The run row of a hooks step: what `quod status` lists.
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
    use quod_trust::Decision;

    /// A step that needed the network skips its offline try, except every
    /// NETWORK_PROBE_EVERY-th run.
    #[test]
    fn a_network_step_tries_offline_every_so_often() {
        let row = |skipped| quod_ledger::NetworkRow {
            surface: "runner".into(),
            reason: "r".into(),
            skipped,
            inputs: None,
            digest: None,
        };
        let tries: Vec<u32> = (0..3 * NETWORK_PROBE_EVERY)
            .scan(0, |skipped, run| {
                let skip = skips_offline(&row(*skipped));
                // A skipped run counts; an offline try that reaches for the
                // network again marks the key anew (the count starts over).
                *skipped = if skip { *skipped + 1 } else { 0 };
                Some((run, skip))
            })
            .filter(|(_, skip)| !skip)
            .map(|(run, _)| run)
            .collect();
        let n = NETWORK_PROBE_EVERY;
        assert_eq!(tries, [n - 1, 2 * n - 1, 3 * n - 1]);
    }

    /// N1 (Fable's review of 75ebbea): the image's env exempts a variable
    /// from classification only in a trusted scope, for an image pinned by
    /// digest or from a trusted registry. Before, any job's image did.
    #[test]
    fn the_image_env_exempts_only_trusted_images_in_trusted_scopes() {
        let official = ["docker.io/library".to_string()];
        let digest = "ghcr.io/anyone/img@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(image_env_exempts("trusted", "python:3.12-slim", &official));
        assert!(image_env_exempts(
            "trusted",
            "docker.io/library/python",
            &official
        ));
        assert!(image_env_exempts("trusted", digest, &official));
        assert!(!image_env_exempts(
            "trusted",
            "ghcr.io/anyone/img:v1",
            &official
        ));
        assert!(!image_env_exempts(
            "trusted",
            "anyone/python:3.12",
            &official
        ));
        // A prefix matches whole components only.
        let acme = ["ghcr.io/acme".to_string()];
        assert!(image_env_exempts("trusted", "ghcr.io/acme/base:1", &acme));
        assert!(!image_env_exempts(
            "trusted",
            "ghcr.io/acme-evil/base:1",
            &acme
        ));
        assert!(!image_env_exempts("trusted", "python:3.12-slim", &[]));
        // Never for a fork's pull request or an unverified event.
        for scope in ["fork-evil_r", "fork-unknown", "unverified"] {
            assert!(!image_env_exempts(scope, "python:3.12-slim", &official));
            assert!(!image_env_exempts(scope, digest, &official));
        }
    }

    /// M3 (Fable's review of 75ebbea): the network memory holds only for
    /// the inputs its offline try read. A change to one of them (or a path
    /// it looked for that now exists) makes the next run try offline; a
    /// change elsewhere does not. Before, only the step key was looked at.
    #[test]
    fn a_network_row_holds_for_the_inputs_it_was_set_on() {
        let f = |p: &str, c: &[u8]| Entry::file(p, Hash::of(c), 0o644);
        let t1 = Tree::from_leaves([f("net-mode.txt", b"online"), f("other", b"x")]);
        let log = [
            quod_manifest::Access {
                path: "net-mode.txt".into(),
                kind: EntryKind::File,
            },
            quod_manifest::Access {
                path: "fixtures/extra".into(),
                kind: EntryKind::File,
            },
            quod_manifest::Access {
                path: "net-mode.txt".into(),
                kind: EntryKind::File,
            },
        ];
        let paths = network_inputs(&log);
        assert_eq!(paths, ["fixtures/extra", "net-mode.txt"]);
        let row = quod_ledger::NetworkRow {
            surface: "runner".into(),
            reason: "r".into(),
            skipped: 0,
            inputs: Some(serde_json::to_string(&paths).unwrap()),
            digest: Some(inputs_digest(&t1, &paths)),
        };
        assert!(same_inputs(&row, &t1));
        let t2 = Tree::from_leaves([f("net-mode.txt", b"online"), f("other", b"y")]);
        assert!(same_inputs(&row, &t2), "a file it did not read changed");
        let t3 = Tree::from_leaves([f("net-mode.txt", b"offline"), f("other", b"x")]);
        assert!(!same_inputs(&row, &t3), "a file it read changed");
        let t4 = Tree::from_leaves([
            f("net-mode.txt", b"online"),
            f("other", b"x"),
            f("fixtures/extra", b"new"),
        ]);
        assert!(!same_inputs(&row, &t4), "a path it found absent exists");
        let unknown = quod_ledger::NetworkRow {
            inputs: None,
            digest: None,
            ..row
        };
        assert!(
            !same_inputs(&unknown, &t1),
            "a row without inputs tries offline"
        );
    }

    #[test]
    fn ignored_outputs_follow_quod_run() {
        let ws = "/__w/cli/cli";
        let env: Vec<(String, String)> = [
            ("HOME", "/__w/cli/cli/.quod/home"),
            ("GOCACHE", "/__w/cli/cli/.quod/gocache"),
            ("GOPATH", "/__w/cli/cli/.quod/go"),
            ("RUNNER_TEMP", "/__w/_temp"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .to_vec();
        let ign = ignored_outputs(&env, ws);
        // HOME: only the volatile paths; the build cache whole; GOPATH (go
        // install's binaries) and anything outside the workspace compared.
        assert!(ign.contains(&".quod/home/.cache/gh".to_string()));
        assert!(ign.contains(&".quod/home/.local/state/gh".to_string()));
        assert!(!ign.contains(&".quod/home/.local/state".to_string()));
        assert!(ign.contains(&".quod/gocache".to_string()));
        assert!(
            !ign.iter()
                .any(|p| p == ".quod/home" || p.starts_with(".quod/go/"))
        );
        assert!(!ign.iter().any(|p| p.contains("_temp")));
        assert_eq!(ign.len(), quod_trust::HOME_VOLATILE.len() + 1);
    }

    fn age(dir: &Path, by: Duration) {
        let t = std::time::SystemTime::now() - by;
        for p in [dir.join("job.json"), dir.to_path_buf()] {
            if let Ok(f) = std::fs::File::open(&p) {
                f.set_modified(t).unwrap();
            }
        }
    }

    #[test]
    fn stale_jobs_are_swept() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(sweep_stale_jobs(tmp.path()).unwrap(), 0, "no jobs dir");
        let job = jobs_dir(tmp.path()).join("01JOB");
        std::fs::create_dir_all(job.join("step-001")).unwrap();
        std::fs::write(job.join("job.json"), "{}").unwrap();
        std::fs::write(job.join("step-001/stdout"), "raw output").unwrap();
        // No owner to check: spared while it may still be running.
        assert_eq!(sweep_stale_jobs(tmp.path()).unwrap(), 0);
        age(&job, UNOWNED_STALE + Duration::from_secs(60));
        assert_eq!(sweep_stale_jobs(tmp.path()).unwrap(), 1);
        assert!(!job.exists());
    }

    /// Minor 3 (review of 5f0afd6): a job's secrets in /dev/shm whose job
    /// directory is gone are removed by the sweep; a live job's are kept.
    #[test]
    fn the_sweep_removes_orphaned_job_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let shm = secrets::runtime_dir(tmp.path());
        if std::fs::create_dir_all(&shm).is_err() {
            return; // no /dev/shm here
        }
        let live = jobs_dir(tmp.path()).join("01LIVE");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::write(live.join("job.json"), "{}").unwrap();
        std::fs::write(shm.join("01LIVE.json"), "{}").unwrap();
        std::fs::write(shm.join("01GONE.json"), "{\"values\":[\"x\"]}").unwrap();
        sweep_stale_jobs(tmp.path()).unwrap();
        assert!(
            !shm.join("01GONE.json").exists(),
            "orphaned secrets removed"
        );
        assert!(shm.join("01LIVE.json").exists(), "a live job's kept");
        let _ = std::fs::remove_dir_all(&shm);
    }

    fn job_with_workflow_mount(tmp: &Path) -> Job {
        let work = tmp.join("_work");
        for d in ["_temp/_github_workflow", "o/r"] {
            std::fs::create_dir_all(work.join(d)).unwrap();
        }
        let m = |s: &str, t: &str| protocol::Mount {
            source_volume_path: work.join(s).to_string_lossy().into_owned(),
            target_volume_path: t.into(),
            read_only: false,
        };
        let c = Container {
            image: Some("golang@sha256:x".into()),
            working_directory: Some("/__w/o/r".into()),
            system_mount_volumes: vec![
                m("", "/__w"),
                m("_temp", "/__w/_temp"),
                m("_temp/_github_workflow", "/github/workflow"),
            ],
            ..Container::default()
        };
        Job::create(&tmp.join("state"), &c, None, Platform::test_aarch64()).unwrap()
    }

    /// N1: a volume never reaches a job. Job::create refuses one (behind
    /// prepare_job's refusal), and paths resolve through the runner's own
    /// mounts only: a `user_mounts` onto /github/workflow/event.json that a
    /// job.json carries cannot replace the payload the scope is read from.
    #[test]
    fn volumes_never_reach_a_job() {
        let tmp = tempfile::tempdir().unwrap();
        let c = Container {
            image: Some("golang@sha256:x".into()),
            working_directory: Some("/__w/o/r".into()),
            user_mount_volumes: vec![protocol::Mount {
                source_volume_path: "/".into(),
                target_volume_path: "/host".into(),
                read_only: false,
            }],
            ..Container::default()
        };
        let Err(e) = Job::create(
            &tmp.path().join("state"),
            &c,
            None,
            Platform::test_aarch64(),
        ) else {
            panic!("a container volume is refused");
        };
        assert!(e.to_string().contains("/host"), "{e}");
        assert!(!jobs_dir(&tmp.path().join("state")).exists());

        let job = job_with_workflow_mount(tmp.path());
        let forged = tmp.path().join("forged");
        std::fs::create_dir_all(&forged).unwrap();
        std::fs::write(forged.join("event.json"), "{}").unwrap();
        let p = job.dir().join("job.json");
        let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        v["user_mounts"] = serde_json::json!([{
            "source_volume_path": forged.join("event.json"),
            "target_volume_path": "/github/workflow/event.json",
            "read_only": false
        }]);
        std::fs::write(&p, v.to_string()).unwrap();
        let job = Job::load(&tmp.path().join("state"), &job.id).unwrap();
        assert_eq!(
            job.host_path("/github/workflow/event.json").unwrap(),
            tmp.path().join("_work/_temp/_github_workflow/event.json")
        );
        assert_eq!(job.host_path("/host/etc/passwd"), None);
    }

    /// N3: the withheld line is versioned, so keys recorded when withheld
    /// names were removed (v1) never match keys under the placeholder.
    #[test]
    fn the_withheld_line_is_versioned() {
        let names = ["GITHUB_SHA".to_string(), "GITHUB_TOKEN".to_string()];
        assert_eq!(
            withheld_key_line(&names),
            "quod:withheld v3 GITHUB_SHA,GITHUB_TOKEN"
        );
        assert_ne!(
            withheld_key_line(&names),
            format!("quod:withheld {}", names.join(",")),
            "not the v1 line"
        );
    }

    /// Trust v2 replays a result recorded once. A fork's record lands in the
    /// fork's own scope ledger, so the trusted scope of the same repository
    /// never finds it: an untrusted fork's first record is never a trusted
    /// replay, under v2 as under v1.
    #[test]
    fn a_forks_record_is_never_a_trusted_hit() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("state").join("cas")).unwrap();
        let event = tmp.path().join("_work/_temp/_github_workflow/event.json");
        let scope_for = |name: &str, payload: &[u8]| {
            // (Job::create makes the runner's directories the event lives in.)
            let mut job = job_with_workflow_mount(tmp.path());
            std::fs::write(&event, payload).unwrap();
            job.ensure_scope(&cas, Some(name), &[], &[]).unwrap();
            job.scope().to_path_buf()
        };
        let fork = scope_for(
            "pull_request",
            br#"{"repository":{"full_name":"o/r"},"pull_request":{"head":{"repo":{"full_name":"evil/r"}}}}"#,
        );
        let trusted = scope_for(
            "push",
            br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","after":"abc","pusher":{}}"#,
        );
        assert_ne!(fork, trusted);
        assert!(fork.ends_with("fork-evil_r"), "{}", fork.display());
        assert!(trusted.ends_with("trusted"), "{}", trusted.display());
        // The fork records a result for a key on a tree.
        let key = Hash::of(b"go test ./...");
        let tree = Tree::from_leaves([Entry::file("go.mod", Hash::of(b"m"), 0o644)]);
        let m = Manifest::from_log(
            key,
            vec!["/workspace".into()],
            &tree,
            [quod_manifest::Access {
                path: "go.mod".into(),
                kind: EntryKind::File,
            }],
        )
        .unwrap();
        let fl = Ledger::open(&fork.join("ledger.sqlite")).unwrap();
        let mid = fl.put_manifest(&m, "01FORK").unwrap();
        fl.put_result(&StepResult {
            v: quod_manifest::VERSION,
            step_key: key,
            manifest_id: mid,
            run_id: "01FORK".into(),
            recorded_at: quod_ledger::now_rfc3339(),
            exit_code: 0,
            stdout: Hash::of(b""),
            stderr: Hash::of(b""),
            outputs: vec![],
            root_outputs: vec![],
            duration_ms: 1,
            commands: vec![],
        })
        .unwrap();
        assert!(fl.find_match(&key, &tree).unwrap().hit.is_some());
        // The trusted scope's ledger has never heard of it: a miss, and the
        // trust layer under v2's defaults has nothing to replay.
        let tl = Ledger::open(&trusted.join("ledger.sqlite")).unwrap();
        let lookup = tl.find_match(&key, &tree).unwrap();
        assert!(lookup.hit.is_none() && lookup.candidates == 0);
        let trust = Trust::new(&tl, &cas, Policy::default().trust()).unwrap();
        assert_eq!(
            trust.decide(&key, None).unwrap(),
            Decision::Execute(Reason::Miss)
        );
        assert!(tl.trust(&key).unwrap().is_none());
    }

    /// N1: the host copy of the checkout is not shared between a writer and
    /// a job that is not one, nor between scopes.
    #[test]
    fn the_host_copy_stamp_names_the_kind_of_job() {
        let tmp = tempfile::tempdir().unwrap();
        let mut job = job_with_workflow_mount(tmp.path());
        job.repository = "o/r".into();
        job.scope_name = "trusted".into();
        job.writer = true;
        let writer = host_copy_stamp(&job, "abc");
        job.writer = false;
        let reader = host_copy_stamp(&job, "abc");
        job.scope_name = "fork-evil_r".into();
        let fork = host_copy_stamp(&job, "abc");
        assert!(writer != reader && reader != fork && writer != fork);
        assert!(writer.ends_with("abc") && writer.contains("writer"));
    }

    /// MINOR 6 and N5: the sweep removes a job only when it can tell its
    /// owner is gone (or the job has sat longer than any job may run), and
    /// never while a hook holds the state.
    #[test]
    fn the_sweep_spares_live_jobs() {
        let tmp = tempfile::tempdir().unwrap();
        let me = Owner::of(std::process::id()).expect("this process");
        assert_eq!(me.alive(), Some(true));
        assert!(!me.pid_ns.is_empty());
        let job = |id: &str, owner: Option<Owner>| {
            let d = jobs_dir(tmp.path()).join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("job.json"),
                serde_json::json!({ "owner": owner }).to_string(),
            )
            .unwrap();
            d
        };
        let live = job("01LIVE", Some(me.clone()));
        // The same pid started at another time: a reused pid, gone.
        let reused = job(
            "01REUSED",
            Some(Owner {
                start: me.start + 1,
                ..me.clone()
            }),
        );
        // No owner (no Runner.Worker above the hook), or one in another pid
        // namespace: cannot be checked, spared until long idle.
        let unknown = job("01UNKNOWN", None);
        let elsewhere = job(
            "01ELSEWHERE",
            Some(Owner {
                pid_ns: "pid:[1]".into(),
                start: me.start + 1,
                ..me.clone()
            }),
        );
        let abandoned = job("01ABANDONED", None);
        age(&abandoned, UNOWNED_STALE + Duration::from_secs(60));
        // A hook holding the state: the sweep waits, then sweeps nothing.
        let held = state_lock_shared(tmp.path()).unwrap();
        assert_eq!(
            sweep_stale_jobs_waiting(tmp.path(), Duration::from_millis(300)).unwrap(),
            0
        );
        assert!(reused.exists() && abandoned.exists());
        drop(held);
        assert_eq!(sweep_stale_jobs(tmp.path()).unwrap(), 2);
        assert!(live.exists() && unknown.exists() && elsewhere.exists());
        assert!(!reused.exists() && !abandoned.exists());
    }

    /// N5: a hook with no Runner.Worker above it records no owner, rather
    /// than its parent, which dies with each hook. Under GitHub Actions this
    /// test itself runs below a Runner.Worker, which is then the owner.
    /// Whether one is above is read from the process tree, not from
    /// GITHUB_ACTIONS: `sudo` (ci.yml's root integration job) drops that
    /// variable while the test still runs under the worker.
    #[test]
    fn no_worker_no_owner() {
        let mut worker = None;
        let mut pid = std::os::unix::process::parent_id();
        for _ in 0..8 {
            if pid <= 1 {
                break;
            }
            if std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .is_ok_and(|c| c.trim_end() == "Runner.Worker")
            {
                worker = Some(pid);
                break;
            }
            match proc_stat(pid) {
                Some(st) => pid = st.ppid,
                None => break,
            }
        }
        match worker {
            Some(w) => assert_eq!(Owner::of_this_hook().map(|o| o.pid), Some(w)),
            None => assert_eq!(Owner::of_this_hook(), None),
        }
    }

    #[test]
    fn mountinfo_paths_are_unescaped() {
        assert_eq!(
            unescape_mountinfo(r"/state/jobs/a\040b/step\011x\134y"),
            PathBuf::from("/state/jobs/a b/step\tx\\y")
        );
        assert_eq!(
            unescape_mountinfo(r"/plain/path"),
            PathBuf::from("/plain/path")
        );
        // Not an escape: left as written.
        assert_eq!(
            unescape_mountinfo(r"/a\9xy\04"),
            PathBuf::from(r"/a\9xy\04")
        );
        assert_eq!(unescape_mountinfo(r"/a\777"), PathBuf::from(r"/a\777"));
    }

    #[test]
    fn scopes_from_events() {
        let scope = |name: Option<&str>, payload: &[u8]| {
            let s = scope_of(name, Some(payload), &[]);
            (s.name, s.writer)
        };
        let push = Some("push");
        assert_eq!(
            scope_of(push, None, &[]),
            ScopeOf {
                repository: String::new(),
                name: "unverified".into(),
                writer: false
            }
        );
        assert_eq!(scope_of(push, Some(b"not json"), &[]).name, "unverified");
        let on_main = br#"{"repository":{"full_name":"Pango07/Quod","default_branch":"main"},"ref":"refs/heads/main","after":"abc","pusher":{"name":"p"}}"#;
        assert_eq!(
            scope_of(push, Some(on_main), &[]),
            ScopeOf {
                repository: "pango07/quod".into(),
                name: "trusted".into(),
                writer: true
            }
        );
        // Trusted but not a writer: another branch, a tag, no default branch named.
        let branch = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/feature","after":"abc","pusher":{}}"#;
        assert_eq!(scope(push, branch), ("trusted".into(), false));
        let tag = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/tags/main","after":"abc","pusher":{}}"#;
        assert_eq!(scope(push, tag), ("trusted".into(), false));
        let nameless = br#"{"repository":{"full_name":"o/r"},"ref":"refs/heads/main","after":"abc","pusher":{}}"#;
        assert_eq!(scope(push, nameless), ("trusted".into(), false));
        let dispatch_main = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","inputs":null,"workflow":".github/workflows/ci.yml"}"#;
        let dispatch_branch = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/runner-lab","inputs":{"x":"1"}}"#;
        assert_eq!(
            scope(Some("workflow_dispatch"), dispatch_main),
            ("trusted".into(), true)
        );
        assert_eq!(
            scope(Some("workflow_dispatch"), dispatch_branch),
            ("trusted".into(), false)
        );
        let schedule = br#"{"repository":{"full_name":"o/r"},"schedule":"0 4 * * *"}"#;
        assert_eq!(scope(Some("schedule"), schedule), ("trusted".into(), true));
        // An event that does not run the repository's own code by its own
        // hand, or no event name at all.
        for other in [Some("issue_comment"), Some("release"), Some("issues"), None] {
            assert_eq!(
                scope(other, on_main),
                ("unverified".into(), false),
                "{other:?}"
            );
        }
        // A comment on a pull request: unverified, whatever it carries.
        let comment = br#"{"repository":{"full_name":"o/r"},"issue":{"pull_request":{"url":"x"}},"comment":{}}"#;
        assert_eq!(
            scope(Some("issue_comment"), comment),
            ("unverified".into(), false)
        );

        let same = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","pull_request":{"head":{"repo":{"full_name":"O/R"}}}}"#;
        assert_eq!(
            scope_of(Some("pull_request"), Some(same), &[]),
            ScopeOf {
                repository: "o/r".into(),
                name: "trusted".into(),
                writer: false
            },
            "a same-repository pull request is trusted, never a writer"
        );
        let fork = br#"{"repository":{"full_name":"o/r"},"pull_request":{"head":{"repo":{"full_name":"evil/r"}}}}"#;
        assert_eq!(
            scope(Some("pull_request"), fork),
            ("fork-evil_r".into(), false)
        );
        assert_eq!(
            scope(Some("pull_request_target"), fork),
            ("fork-evil_r".into(), false)
        );
        let gone = br#"{"repository":{"full_name":"o/r"},"pull_request":{"head":{"repo":null}}}"#;
        assert_eq!(scope(Some("pull_request"), gone).0, "fork-unknown");
        // workflow_run: by the head repository of the run that triggered it.
        let run_fork = br#"{"repository":{"full_name":"o/r"},"workflow_run":{"head_repository":{"full_name":"evil/r"}}}"#;
        assert_eq!(scope(Some("workflow_run"), run_fork).0, "fork-evil_r");
        let run_same = br#"{"repository":{"full_name":"o/r"},"workflow_run":{"head_repository":{"full_name":"o/r"}}}"#;
        assert_eq!(
            scope(Some("workflow_run"), run_same),
            ("trusted".into(), false)
        );
        let run_none = br#"{"repository":{"full_name":"o/r"},"workflow_run":{}}"#;
        assert_eq!(scope(Some("workflow_run"), run_none).0, "fork-unknown");

        // The operator's protected refs write too; a pull request never.
        let release = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/release","after":"abc","pusher":{},"inputs":null}"#;
        let refs = ["refs/heads/release".to_string()];
        assert!(!scope_of(Some("push"), Some(release), &[]).writer);
        assert!(scope_of(Some("push"), Some(release), &refs).writer);
        assert!(scope_of(Some("workflow_dispatch"), Some(release), &refs).writer);
        let pr_release = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/release","pull_request":{"head":{"repo":{"full_name":"o/r"}}}}"#;
        assert!(!scope_of(Some("pull_request"), Some(pr_release), &refs).writer);
    }

    /// MAJOR 4: GITHUB_EVENT_NAME is in the step's environment, which a
    /// workflow's `env:` may set; the payload is the runner's. A forged name
    /// never trusts a payload it does not fit.
    #[test]
    fn a_forged_event_name_trusts_nothing() {
        let scope = |name: &str, payload: &[u8]| {
            let s = scope_of(Some(name), Some(payload), &[]);
            (s.name, s.writer)
        };
        let unverified = ("unverified".to_string(), false);
        // A same-repository pull request on main, a fork's, and a workflow
        // run, each carrying push-looking fields too.
        let pr_same = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","after":"abc","pusher":{},"inputs":{},"schedule":"x","pull_request":{"head":{"repo":{"full_name":"o/r"}}}}"#;
        let pr_fork = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","after":"abc","pusher":{},"pull_request":{"head":{"repo":{"full_name":"evil/r"}}}}"#;
        let run = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","after":"abc","pusher":{},"workflow_run":{"head_repository":{"full_name":"o/r"}}}"#;
        for forged in ["push", "workflow_dispatch", "schedule"] {
            for payload in [&pr_same[..], &pr_fork[..], &run[..]] {
                assert_eq!(
                    scope(forged, payload),
                    unverified,
                    "{forged} over {}",
                    String::from_utf8_lossy(payload)
                );
            }
        }
        // A name the payload's shape does not fit.
        let push_main = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","after":"abc","pusher":{}}"#;
        let dispatch_main = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","inputs":{}}"#;
        let schedule =
            br#"{"repository":{"full_name":"o/r","default_branch":"main"},"schedule":"0 4 * * *"}"#;
        let comment = br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","issue":{},"comment":{}}"#;
        assert_eq!(
            scope("schedule", push_main),
            unverified,
            "a push is no schedule"
        );
        assert_eq!(
            scope("workflow_dispatch", push_main),
            unverified,
            "a push is no dispatch"
        );
        assert_eq!(
            scope("push", dispatch_main),
            unverified,
            "a dispatch is no push"
        );
        assert_eq!(
            scope("schedule", dispatch_main),
            unverified,
            "a dispatch is no schedule"
        );
        assert_eq!(scope("push", schedule), unverified, "a schedule is no push");
        assert_eq!(
            scope("workflow_dispatch", schedule),
            unverified,
            "a schedule is no dispatch"
        );
        for forged in ["push", "workflow_dispatch", "schedule"] {
            assert_eq!(
                scope(forged, comment),
                unverified,
                "{forged} over an issue comment"
            );
        }
        // A push missing one of its fields.
        for partial in [
            &br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","after":"abc"}"#[..],
            br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","pusher":{}}"#,
            br#"{"repository":{"full_name":"o/r","default_branch":"main"},"after":"abc","pusher":{}}"#,
            br#"{"repository":{"full_name":"o/r","default_branch":"main"},"ref":"refs/heads/main","after":"abc","pusher":"p"}"#,
        ] {
            assert_eq!(scope("push", partial), unverified, "{}", String::from_utf8_lossy(partial));
        }
        // A pull request name over a push, or over a payload that is both.
        assert_eq!(scope("pull_request", push_main), unverified);
        let both = br#"{"repository":{"full_name":"o/r"},"pull_request":{"head":{"repo":{"full_name":"o/r"}}},"workflow_run":{"head_repository":{"full_name":"o/r"}}}"#;
        assert_eq!(scope("pull_request", both), unverified);
        assert_eq!(scope("workflow_run", both), unverified);
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
