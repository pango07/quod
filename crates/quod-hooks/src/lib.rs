//! `quod hooks`: actions/runner's container hooks, implemented by
//! Quod. The official runner owns the job (protocol, YAML, contexts,
//! logs); every step it would have run with `docker exec` arrives here as a
//! `run_script_step`, and Quod runs it in its own sandbox over the
//! observed workspace, records it, and replays it when it is proven.
//!
//! One process per hook invocation; the job's state lives on disk under
//! `<state>/jobs/<id>` and the runner carries the id in the hook `state`.

pub mod checkout;
pub mod exec;
pub mod job;
pub mod protocol;
pub mod provenance;
pub mod rootfs;
pub mod scan;
pub mod seal;
pub mod secrets;
pub mod snapshot;
pub mod step;
pub mod surface;
pub mod trust;

use std::path::Path;

use anyhow::{Context, bail};
use protocol::{Command, Input, PrepareJobResponse, ResponseContainer, ResponseContext};

pub use job::Job;
pub use trust::Policy;

/// Runs one hook invocation read from `stdin` under `policy` (from the
/// shim's command line, never the environment); returns the process exit
/// code (the step's own exit code for `run_script_step`).
pub fn run(
    state_dir: &Path,
    stdin: &mut impl std::io::Read,
    policy: &Policy,
    platform: &HostClass,
) -> anyhow::Result<i32> {
    let input = Input::read(stdin)?;
    check_state_format(state_dir)?;
    // Held for the whole invocation: the pool's sweep of stale jobs takes
    // it exclusively (`job::sweep_stale_jobs`).
    let _state = job::state_lock_shared(state_dir)?;
    match input.command {
        Command::PrepareJob => prepare_job(state_dir, &input, policy, platform),
        Command::CleanupJob => cleanup_job(state_dir, &input, policy),
        Command::RunScriptStep => job::run_script_step(state_dir, &input, policy),
        Command::RunContainerStep => {
            bail!("container actions (uses: docker://...) are not supported by the quod runner yet")
        }
    }
}

/// The hooks' state format, in `<state>/format`. 2: classified secrets
/// (provenance, encodings, staging; DECISIONS, MAJOR 1). A state written
/// before it may hold secrets in the clear in its store, ledgers and
/// events, so it is refused, never upgraded in place.
pub const STATE_FORMAT: u32 = 2;

/// What only the hooks write into a state: a state that has any of these
/// and no `format` was written by a Quod from before `STATE_FORMAT` 2.
const HOOKS_STATE: &[&str] = &["scopes", "jobs", "salt", "trees", "host-copies"];

/// Stamps a new state with `STATE_FORMAT`, accepts a state that has it,
/// and refuses any other with how to start over.
pub fn check_state_format(state_dir: &Path) -> anyhow::Result<()> {
    let p = state_dir.join("format");
    let wipe = || {
        format!(
            "remove it (rm -rf {}) and start over: its store, ledgers and events may hold secrets in the clear",
            state_dir.display()
        )
    };
    match std::fs::read_to_string(&p) {
        Ok(s) if s.trim() == STATE_FORMAT.to_string() => Ok(()),
        Ok(s) => bail!(
            "the Quod state {} has format {:?}; this quod reads format {STATE_FORMAT}: {}",
            state_dir.display(),
            s.trim(),
            wipe()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(old) = HOOKS_STATE.iter().find(|d| state_dir.join(d).exists()) {
                bail!(
                    "the Quod state {} was written before state format {STATE_FORMAT} (it has {old}/ and no format file): {}",
                    state_dir.display(),
                    wipe()
                );
            }
            std::fs::create_dir_all(state_dir)?;
            let tmp = state_dir.join(format!("format.tmp.{}", ulid::Ulid::generate()));
            std::fs::write(&tmp, format!("{STATE_FORMAT}\n"))?;
            std::fs::rename(&tmp, &p)?;
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("read {}", p.display())),
    }
}

/// How the caller (the quod binary) reads this host's class for step
/// keys: `quod run`'s own probe (the sandboxed `__host-probe`), given
/// the store and the directory step runs live in.
pub type HostClass = dyn Fn(&quod_cas::Cas, &Path) -> anyhow::Result<quod_key::Platform>;

fn prepare_job(
    state_dir: &Path,
    input: &Input,
    policy: &Policy,
    platform: &HostClass,
) -> anyhow::Result<i32> {
    let args = input.prepare_job_args()?;
    if !args.services.is_empty() {
        bail!(
            "service containers are not supported by the quod runner yet ({})",
            args.services
                .iter()
                .map(|s| s.context_name.clone().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let container = args
        .container
        .context("this runner requires a job container (ACTIONS_RUNNER_REQUIRE_JOB_CONTAINER)")?;
    // A volume is a host path the workflow names, bound into the job: on
    // the runner's surface writable, where the step runs as root. A fork's
    // `volumes: ['/:/host']` would reach every scope, the seeds, the store
    // and this binary. Refused, like services, before anything is made.
    if !container.user_mount_volumes.is_empty() {
        bail!(
            "container volumes are not supported by the quod runner ({})",
            container
                .user_mount_volumes
                .iter()
                .map(|m| m.target_volume_path.clone())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let image = container
        .image
        .clone()
        .context("job container without an image")?;
    let rootfs = Job::prepare_rootfs(state_dir, &image, policy)?;
    // Probed once a job, like `quod run` does once a run: every step of
    // the job is keyed on this host's class.
    let cas = quod_cas::Cas::open(state_dir.join("cas"))?;
    let class = platform(&cas, &job::jobs_dir(state_dir)).context("probe the host class")?;
    let job = Job::create(state_dir, &container, rootfs, class)?;
    println!(
        "quod: job {} on {} ({}), mode {}",
        job.id,
        job.rootfs
            .as_ref()
            .map_or(job.image.clone(), |r| r.reference.clone()),
        job.rootfs
            .as_ref()
            .map_or("host root".to_string(), |r| r.digest.clone()),
        policy.mode_name(),
    );
    let response = PrepareJobResponse {
        state: serde_json::json!({ "job": job.id }),
        context: ResponseContext {
            container: Some(ResponseContainer {
                id: job.id.clone(),
                network: String::new(),
                ports: Default::default(),
            }),
            services: vec![],
        },
        is_alpine: job.is_alpine,
    };
    protocol::write_response(input.response_file.as_deref(), &response)?;
    Ok(0)
}

fn cleanup_job(state_dir: &Path, input: &Input, policy: &Policy) -> anyhow::Result<i32> {
    if let Some(id) = input.state_str("job") {
        match Job::load(state_dir, id) {
            Ok(job) => job.cleanup(policy)?,
            Err(e) => eprintln!("quod: cleanup: {e}"),
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N1: a job whose container names volumes is refused at prepare_job,
    /// before a job directory or an image exists.
    #[test]
    fn container_volumes_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let input = serde_json::json!({
            "command": "prepare_job",
            "responseFile": tmp.path().join("response.json"),
            "state": {},
            "args": {
                "container": {
                    "image": "golang@sha256:x",
                    "workingDirectory": "/__w/o/r",
                    "systemMountVolumes": [],
                    "userMountVolumes": [
                        {"sourceVolumePath": "/", "targetVolumePath": "/host", "readOnly": false}
                    ]
                },
                "services": []
            }
        });
        let probe: &HostClass = &|_, _| panic!("the host is never probed for a refused job");
        let e = run(
            tmp.path(),
            &mut input.to_string().as_bytes(),
            &Policy::default(),
            probe,
        )
        .unwrap_err();
        let e = format!("{e:#}");
        assert!(
            e.contains("container volumes are not supported") && e.contains("(/host)"),
            "{e}"
        );
        assert!(!job::jobs_dir(tmp.path()).exists(), "no job directory");
    }

    /// A new state is stamped; one from before format 2 is refused with
    /// how to start over; a stamped one is accepted.
    #[test]
    fn older_state_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        check_state_format(tmp.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("format")).unwrap(),
            "2\n"
        );
        check_state_format(tmp.path()).unwrap();
        let old = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(old.path().join("scopes/o_r/trusted")).unwrap();
        let e = format!("{:#}", check_state_format(old.path()).unwrap_err());
        assert!(
            e.contains("before state format 2") && e.contains("rm -rf"),
            "{e}"
        );
        std::fs::write(old.path().join("format"), "1\n").unwrap();
        let e = format!("{:#}", check_state_format(old.path()).unwrap_err());
        assert!(e.contains("format \"1\""), "{e}");
    }
}
