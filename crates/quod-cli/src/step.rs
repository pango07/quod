//! The runner API: run one step in a pinned rootfs and get back its result,
//! manifest and outputs. For a runner adapter (exp/runner's container
//! hooks) that owns the job and hands Quod one step at a time. In
//! process it is `run_step`; from another process `quod step
//! --request R --response O` takes and writes the same shapes as JSON, and
//! the step's stdout and stderr stream to its own (recorded ones on a
//! replay), so the runner's log is the step's log either way.
//!
//! Request:
//! ```json
//! {"image": "b3:<rootfs digest>" | "sha256:<OCI digest given at import>",
//!  "workspace": {"dir": "/host/checkout"} | {"tree": "b3:<tree document>"},
//!  "cmd": ["/bin/sh", "-e", "-c", "go test ./..."],
//!  "cwd": "/workspace",
//!  "env": [["NAME", "VALUE"]],
//!  "mounts": [{"dst": "/opt/tools", "dir": "/host/tools"} | {"dst": "/opt/tools", "tree": "b3:..."}],
//!  "lookup": true, "toolchain": "go" | "node" | "python"}
//! ```
//! - `workspace` is served at /workspace through the observer: what the step
//!   reads there is the manifest. A `dir` is imported (without `.git`); a
//!   `tree` is a document in the store (`rootfs::put_tree`).
//! - `env` is the step's whole environment (nothing is inherited, volatile
//!   names are refused, as for every key). `cmd` runs as given.
//! - `mounts` are read-only trees bound at `dst`, pinned by content and
//!   served from their blobs like the rootfs (content-derived file times,
//!   sorted listings): their digests are part of the key (`image.mounts`), never
//!   observed. A `dir`
//!   is imported into the store on every call, which hashes it; a runner
//!   that reuses a mount passes its `tree` digest instead. A destination
//!   the image lacks is created by the sandbox and shows up in
//!   `root_outputs` as directories. There is no other kind of mount: a host
//!   directory bound as is would be an input nothing records.
//! - `lookup` (default true): replay instead of executing when the trust
//!   layer allows it, exactly as `quod run --mode replay`: a manifest
//!   that matches the workspace and whose result was reproduced `proofs`
//!   times in a row (default 2; failures and killed runs never, unless
//!   `replay_failures` for failures), with `audit` (default 0.05) of such
//!   hits re-executed and compared, and a key that diverged quarantined.
//!   False always executes and records (observe mode: the execution still
//!   counts as evidence).
//! - `toolchain` (optional): the profile whose sealing and output rules
//!   apply (its env does not: `env` is the whole environment). Absent, it is
//!   detected from the workspace's top level, and Go's when that finds none
//!   or several: Go's rules seal only a GOMODCACHE the env puts under the
//!   workspace and drop no outputs. Part of the key.
//!
//! Response: `{"step_key", "key" (the canonical key document, which says
//! what pinned the environment), "replayed", "recorded" (false for a step
//! killed by a signal, or one whose write into a sealed prefix failed with
//! EROFS: never memoized), "signal", "manifest", "result", "trust" (the
//! trust layer's lines: the receipt's on a replay, the proof streak or
//! divergence on an execution)}`. `result.outputs` are the workspace writes (paths relative to
//! /workspace), `result.root_outputs` the writes anywhere else but /tmp
//! (relative to /), `result.stdout`/`stderr` blobs; every hash resolves in
//! the store at
//! `<cas>/<first 2 hex>/<hash>`. Between steps the root starts again from
//! the pinned image: a runner that wants a step's root outputs visible to
//! the next step pins the derived root as a new tree, which is then the
//! next step's environment, by content.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use quod_cas::{Cas, Hash};
use quod_key::{StepSpec, Toolchain, canonical_key_json, step_key};
use quod_ledger::{Ledger, Observed, now_rfc3339};
use quod_manifest::{Manifest, StepResult};
use quod_sandbox::Mount;
use serde::{Deserialize, Serialize};

use quod_trust::{Decision, Mode, Policy, Replayed, Step, Trust};

use crate::audit;
use crate::rootfs;
use crate::run::{Phases, execute};
use crate::state::State;
use crate::trust::{ignored_outputs, shell_join};

/// Where a tree comes from: a host directory imported now, or a tree
/// document already in the store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Dir(PathBuf),
    Tree(Hash),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountRequest {
    pub dst: String,
    #[serde(flatten)]
    pub src: Source,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub image: String,
    pub workspace: Source,
    pub cmd: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    #[serde(default)]
    pub mounts: Vec<MountRequest>,
    #[serde(default = "yes")]
    pub lookup: bool,
    #[serde(default)]
    pub toolchain: Option<Toolchain>,
    /// The trust policy for `lookup` (see the module docs): proofs, the
    /// shadow phase and the audit floor, as `quod run`'s flags.
    #[serde(default)]
    pub proofs: Option<u32>,
    #[serde(default)]
    pub shadow: Option<u32>,
    #[serde(default)]
    pub audit: Option<f64>,
    #[serde(default)]
    pub replay_failures: bool,
    /// Execute every audit inline instead of deferring decay-phase audits
    /// (trust v2, "Deferred audits"; `quod run --inline-audits`).
    #[serde(default)]
    pub inline_audits: bool,
}

impl Request {
    /// Replay mode for `lookup`, observe mode without it.
    fn policy(&self) -> Result<Policy, quod_trust::TrustError> {
        let p = Policy {
            mode: if self.lookup {
                Mode::Replay
            } else {
                Mode::Observe
            },
            proofs: self.proofs.unwrap_or(quod_trust::DEFAULT_PROOFS),
            shadow: self.shadow.unwrap_or(quod_trust::DEFAULT_SHADOW),
            audit: self.audit.unwrap_or(quod_trust::DEFAULT_AUDIT),
            replay_failures: self.replay_failures,
            defer: !self.inline_audits,
        };
        p.validate()?;
        Ok(p)
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub step_key: Hash,
    /// The canonical key document: image (kind, digest, mounts, platform),
    /// cmd, cwd, env, roots.
    pub key: serde_json::Value,
    pub replayed: bool,
    pub recorded: bool,
    pub signal: Option<i32>,
    pub manifest: Manifest,
    pub result: StepResult,
    /// The trust layer's lines about this step.
    pub trust: Vec<String>,
    /// A replay whose audit was deferred (trust v2): the pending audit's
    /// id in the ledger; `quod audit` executes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deferred_audit: Option<i64>,
}

/// Runs (or replays) one step. `run_dir` is a fresh scratch directory on the
/// store's filesystem, removed before this returns.
pub fn run_step(
    cas: &Cas,
    ledger: &Ledger,
    run_dir: &Path,
    req: &Request,
) -> anyhow::Result<Outcome> {
    let digest = rootfs::resolve(cas, &req.image)?;
    let root = rootfs::open(cas, &digest)?;
    let tree = match &req.workspace {
        Source::Dir(d) => cas
            .import_dir(d)
            .with_context(|| format!("import workspace {}", d.display()))?
            .without_top_level(".git"),
        Source::Tree(h) => rootfs::tree(cas, h)?,
    };
    // A tree with a git index (a runner's checkout) is served it refreshed
    // against the tree for the step's identity, the sandbox's default.
    let (uid, gid) = quod_sandbox::step_ids(None, None);
    let (tree, _) = quod_fs::git::refresh_index(&tree, cas, uid, gid)?;
    // Run directories live beside this one; their filesystem is part of the
    // host class.
    let runs = run_dir.parent().unwrap_or(run_dir);
    let mut image = root.image(cas, runs)?;
    for m in &req.mounts {
        let d = match &m.src {
            Source::Tree(h) => *h,
            Source::Dir(p) => {
                let t = cas
                    .import_dir(p)
                    .with_context(|| format!("import mount {}", p.display()))?;
                rootfs::put_tree(cas, &t)?
            }
        };
        image.mounts.push((m.dst.clone(), d));
    }
    let toolchain = req.toolchain.unwrap_or_else(|| {
        let top = tree
            .iter()
            .filter(|e| !e.p.contains('/'))
            .map(|e| e.p.as_str());
        Toolchain::detect(top).unwrap_or(Toolchain::Go)
    });
    let spec = StepSpec {
        image,
        toolchain,
        cmd: req.cmd.clone(),
        cwd: req.cwd.clone(),
        env: req.env.clone(),
        roots: vec![quod_key::WORKSPACE.to_string()],
        warm: false,
    };
    // Validates env and mounts before anything runs.
    let key = step_key(&spec)?;
    let key_json = canonical_key_json(&spec)?;
    let env = spec.normalized_env()?;
    ledger.put_step_key(&key, &key_json)?;
    let key_doc: serde_json::Value = serde_json::from_str(&key_json)?;

    // Replay only what the trust layer allows (module docs).
    let trust = Trust::new(ledger, cas, req.policy()?)?;
    let ignore = ignored_outputs(spec.profile(), &env, &tree);
    let cmd_text = shell_join(&req.cmd);
    let tstep = Step {
        key,
        cmd: &cmd_text,
        ignore: &ignore,
    };
    let lookup = ledger.find_match(&key, &tree)?;
    let decision = trust.decide(&key, lookup.hit.as_ref())?;
    let reason = match decision {
        Decision::Replay | Decision::Defer { .. } => {
            let hit = lookup.hit.as_ref().expect("a replay needs a hit");
            // A deferred audit (trust v2) needs the exact input tree in the
            // store; content-addressed, so a tree seen before is free.
            let deferred_tree = match decision {
                Decision::Defer { .. } => Some(rootfs::put_tree(cas, &tree)?),
                _ => None,
            };
            // The replay's run row joins the trust update (as `quod run` and
            // the hooks do): a later quarantine of the key marks it suspect
            // through that row (trust v2). A deferred audit is enqueued in
            // the same transaction.
            let replayed = ledger.atomically(|l| -> anyhow::Result<_> {
                let r = trust.record_replay(&tstep, hit)?;
                let run_id = ulid::Ulid::generate().to_string();
                if let Replayed::Replayed(_) | Replayed::Deferred(..) = &r {
                    l.record_run_facts(&quod_ledger::RunFacts {
                        id: &run_id,
                        step_key: Some(&key),
                        mode: Some("replay"),
                        outcome: Some("replayed"),
                        manifest_id: Some(&hit.1.manifest_id),
                        exit_code: Some(hit.1.exit_code),
                        ..quod_ledger::RunFacts::default()
                    })?;
                }
                let deferred = match &r {
                    Replayed::Deferred(_, nth) => {
                        let th = match deferred_tree {
                            Some(h) => h,
                            None => rootfs::put_tree(cas, &tree)?,
                        };
                        let req = audit::Request {
                            v: audit::REQUEST_VERSION,
                            kind: audit::Kind::Step,
                            spec: (&spec).into(),
                            script: None,
                            extra_env: vec![],
                            policy: (&trust.policy).into(),
                            repo: None,
                            rootfs: Some(digest),
                            warm: false,
                            guard: false,
                            ignore: ignore.clone(),
                            timeout_ms: 0,
                        };
                        Some((
                            *nth,
                            audit::enqueue(l, &key, hit, &run_id, *nth, &th, &req)?,
                        ))
                    }
                    _ => None,
                };
                Ok((r, deferred))
            })?;
            match replayed {
                (Replayed::Replayed(facts), _) | (Replayed::Deferred(facts, _), _) => {
                    let (manifest, result) = hit.clone();
                    let deferred = replayed.1;
                    let mut lines = vec![facts.receipt_line()];
                    if let Some((nth, id)) = deferred {
                        lines.push(audit::receipt_line(nth, id));
                    }
                    return Ok(Outcome {
                        step_key: key,
                        key: key_doc,
                        replayed: true,
                        recorded: true,
                        signal: None,
                        manifest,
                        result,
                        trust: lines,
                        deferred_audit: deferred.map(|(_, id)| id),
                    });
                }
                (Replayed::Execute(reason), _) => reason,
            }
        }
        Decision::Execute(reason) => reason,
    };

    let run_id = ulid::Ulid::generate().to_string();
    let ex = execute_with_mounts(cas, &tree, &spec, env, run_dir, &root);
    let _ = quod_cas::remove_any(run_dir);
    let ex = ex?;
    // A sealed dependency tree refused a write CI's would have taken: the
    // result is this sandbox's, not the step's, so it is never memoized.
    let recorded = ex.signal.is_none() && !ex.sealed_write_failed;
    let manifest_id = quod_manifest::manifest_id(&ex.manifest)?;
    let result = StepResult {
        v: quod_manifest::VERSION,
        step_key: key,
        manifest_id,
        run_id: run_id.clone(),
        recorded_at: now_rfc3339(),
        exit_code: ex.exit_code,
        stdout: ex.stdout,
        stderr: ex.stderr,
        outputs: ex.outputs,
        root_outputs: ex.root_outputs,
        duration_ms: ex.wall.as_millis() as u64,
        commands: vec![],
    };
    // Killed from outside (OOM, timeout): the step's own result is unknown,
    // so it is returned and never memoized. Otherwise the run (with the
    // transport that observed it), manifest and result go in together.
    // With the trust layer's verdict, in the same transaction.
    let mut trust_lines = Vec::new();
    if recorded {
        let verdict = ledger.atomically(|l| -> anyhow::Result<_> {
            let v = trust.record_execution(
                &tstep,
                &run_id,
                &ex.manifest,
                &result,
                lookup.hit.as_ref(),
                &reason,
            )?;
            l.record_execution(
                &run_id,
                None,
                None,
                Observed {
                    transport: Some(ex.transport),
                    observer: Some(&ex.observer),
                },
                &ex.manifest,
                &result,
            )?;
            Ok(v)
        })?;
        trust_lines = verdict.lines();
    }
    tracing::info!(target: "quod", run_id, step_key = %key, image = %digest, mounts = spec.image.mounts.len(), replayed = false, recorded, exit_code = ex.exit_code, wall_ms = ex.wall.as_millis() as u64, transport = ex.transport, observer = ex.observer, "step");
    Ok(Outcome {
        step_key: key,
        key: key_doc,
        replayed: false,
        recorded,
        signal: ex.signal,
        manifest: ex.manifest,
        result,
        trust: trust_lines,
        deferred_audit: None,
    })
}

/// Serves the pinned mounts (each its own FUSE mount under `run_dir`),
/// executes, and unmounts them, reporting a mount's I/O error as an error.
pub(crate) fn execute_with_mounts(
    cas: &Cas,
    tree: &quod_cas::Tree,
    spec: &StepSpec,
    env: Vec<(String, String)>,
    run_dir: &Path,
    root: &rootfs::Rootfs,
) -> anyhow::Result<crate::run::Executed> {
    let mut pinned = Vec::new();
    for (i, (_, d)) in spec.image.mounts.iter().enumerate() {
        pinned.push(rootfs::PinnedTree::mount(
            cas,
            d,
            &run_dir.join("pinned").join(i.to_string()),
        )?);
    }
    let mounts: Vec<Mount> = spec
        .image
        .mounts
        .iter()
        .zip(&pinned)
        .map(|((dst, _), p)| Mount {
            src: p.dir().to_path_buf(),
            dst: dst.into(),
            writable: false,
        })
        .collect();
    let mut phases = Phases::default();
    let ex = execute(
        cas,
        tree,
        spec,
        env,
        run_dir,
        None,
        Some(root),
        &mounts,
        // The runner owns the job's time limit.
        None,
        crate::gotest::GoTests::default(),
        None,
        &mut phases,
    );
    for p in pinned {
        p.unmount()?;
    }
    ex
}

#[derive(Args)]
pub struct StepArgs {
    /// The request, JSON (see the module docs of crate::step); `-` is stdin.
    #[arg(long)]
    request: PathBuf,
    /// Where to write the response, JSON. Written only when the step ran or
    /// replayed; its absence means a Quod error (exit 2).
    #[arg(long)]
    response: PathBuf,
    #[arg(long)]
    ledger: Option<PathBuf>,
    #[arg(long)]
    cas: Option<PathBuf>,
}

/// `quod step`: the process form of `run_step`. Exits with the step's
/// exit code; stdout and stderr carry the step's streams.
pub fn cli(state: &State, args: StepArgs) -> anyhow::Result<u8> {
    let text = if args.request.as_os_str() == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else {
        std::fs::read_to_string(&args.request)
            .with_context(|| format!("read {}", args.request.display()))?
    };
    let req: Request = serde_json::from_str(&text).context("parse the step request")?;
    let cas = Cas::open(args.cas.clone().unwrap_or_else(|| state.cas_dir()))?;
    let ledger = Ledger::open(&args.ledger.clone().unwrap_or_else(|| state.ledger_path()))?;
    let claim = state.claim_run_dir(&ulid::Ulid::generate().to_string(), false)?;
    let out = run_step(&cas, &ledger, &claim.path, &req)?;
    if out.replayed {
        quod_replay::replay_log(
            &out.result,
            &cas,
            &mut std::io::stdout().lock(),
            &mut std::io::stderr().lock(),
        )?;
    }
    std::fs::write(&args.response, serde_json::to_vec(&out)?)
        .with_context(|| format!("write {}", args.response.display()))?;
    if out.deferred_audit.is_some() {
        audit::spawn_worker(state, args.ledger.as_deref(), args.cas.as_deref());
    }
    Ok(u8::try_from(out.result.exit_code).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_shapes() {
        let r: Request = serde_json::from_str(
            r#"{"image":"sha256:00","workspace":{"dir":"/w"},"cmd":["true"],"cwd":"/workspace",
                "mounts":[{"dst":"/opt/t","tree":"b3:0000000000000000000000000000000000000000000000000000000000000000"},{"dst":"/e","dir":"/x"}]}"#,
        )
        .unwrap();
        assert!(r.lookup && r.env.is_empty());
        assert_eq!(r.workspace, Source::Dir("/w".into()));
        assert_eq!(r.mounts[1].src, Source::Dir("/x".into()));
        assert!(matches!(r.mounts[0].src, Source::Tree(_)));
        let back: Request = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
        // Unknown fields are refused, not ignored: a runner asking for
        // something this API does not do must hear about it.
        assert!(
            serde_json::from_str::<Request>(
                r#"{"image":"x","workspace":{"dir":"/w"},"cmd":[],"cwd":"/","warm":true}"#
            )
            .is_err()
        );
    }
}
