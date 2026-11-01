//! `stillpass hooks`: actions/runner's container hooks, implemented by
//! Stillpass. The official runner owns the job (protocol, YAML, contexts,
//! logs); every step it would have run with `docker exec` arrives here as a
//! `run_script_step`, and Stillpass runs it in its own sandbox over the
//! observed workspace, records it, and replays it when it is proven.
//!
//! One process per hook invocation; the job's state lives on disk under
//! `<state>/jobs/<id>` and the runner carries the id in the hook `state`.

pub mod checkout;
pub mod exec;
pub mod job;
pub mod protocol;
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
    // Held for the whole invocation: the pool's sweep of stale jobs takes
    // it exclusively (`job::sweep_stale_jobs`).
    let _state = job::state_lock_shared(state_dir)?;
    match input.command {
        Command::PrepareJob => prepare_job(state_dir, &input, policy, platform),
        Command::CleanupJob => cleanup_job(state_dir, &input, policy),
        Command::RunScriptStep => job::run_script_step(state_dir, &input, policy),
        Command::RunContainerStep => {
            bail!(
                "container actions (uses: docker://...) are not supported by the stillpass runner yet"
            )
        }
    }
}

/// How the caller (the stillpass binary) reads this host's class for step
/// keys: `stillpass run`'s own probe (the sandboxed `__host-probe`), given
/// the store and the directory step runs live in.
pub type HostClass = dyn Fn(&sp_cas::Cas, &Path) -> anyhow::Result<sp_key::Platform>;

fn prepare_job(
    state_dir: &Path,
    input: &Input,
    policy: &Policy,
    platform: &HostClass,
) -> anyhow::Result<i32> {
    let args = input.prepare_job_args()?;
    if !args.services.is_empty() {
        bail!(
            "service containers are not supported by the stillpass runner yet ({})",
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
    let image = container
        .image
        .clone()
        .context("job container without an image")?;
    let rootfs = Job::prepare_rootfs(state_dir, &image, policy)?;
    // Probed once a job, like `stillpass run` does once a run: every step of
    // the job is keyed on this host's class.
    let cas = sp_cas::Cas::open(state_dir.join("cas"))?;
    let class = platform(&cas, &job::jobs_dir(state_dir)).context("probe the host class")?;
    let job = Job::create(state_dir, &container, rootfs, class)?;
    println!(
        "stillpass: job {} on {} ({}), mode {}",
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
            Err(e) => eprintln!("stillpass: cleanup: {e}"),
        }
    }
    Ok(0)
}
