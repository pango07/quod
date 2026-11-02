//! The quod command line. Thin: wiring, flags and exit codes.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// `eprintln!` that never panics. A terminal that hung up (the SIGHUP
/// `quod run` survives to clean up) fails every write to it with EIO, and
/// `eprintln!` panics on a failed write: the run would end in a panic, or
/// abort in the error banner's own `eprintln!`, before reporting its exit.
macro_rules! say {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}

/// `eprint!` that never panics (`say!`).
macro_rules! say_raw {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = write!(std::io::stderr(), $($t)*);
    }};
}

mod audit;
mod bench;
mod deps;
mod gitdir;
mod gotest;
mod hostfp;
mod inspect;
mod profile;
mod rootfs;
mod run;
mod runner;
mod seal;
mod snapshot;
mod state;
mod status;
mod step;
mod trust;
mod vitest;

/// Exit code for Quod's own failures. A step can exit 2 by itself, so
/// the code alone does not say whose failure it was: a Quod failure
/// prints the `QUOD  ERROR` banner on stderr and writes an event with
/// `outcome: "error"` to events.jsonl, and leaves no run row (every run that
/// produced a step result has one, with its outcome).
pub const EXIT_QUOD_ERROR: u8 = 2;

#[derive(Parser)]
#[command(
    name = "quod",
    version,
    about = "CI that refuses to repeat itself (Phase 0 spike)"
)]
struct Cli {
    /// State directory holding the CAS, the ledger, run dirs and events.
    /// Defaults to $QUOD_HOME, else ~/.cache/quod.
    #[arg(long, global = true)]
    state: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run one step: `quod run [--] <cmd...>` or `--step <script>`.
    /// Observe mode (default) executes and records; replay mode replays
    /// results that proved themselves.
    Run(run::RunArgs),
    /// Execute the audits deferred by replays (trust v2): re-run each
    /// step on its recorded input tree and compare with what was served.
    /// `quod run` starts one after a deferred replay; a cron job or a
    /// runner's post-job hook may run it too.
    Audit(audit::AuditArgs),
    /// Hits, misses, time saved and the trust state of every step key of a repo.
    Status(status::StatusArgs),
    /// Measure observer overhead (A8) and hit rate over history (A11).
    Bench(bench::BenchArgs),
    /// actions/runner container hook: reads one hook input from stdin
    /// (ACTIONS_RUNNER_CONTAINER_HOOKS points at scripts/runner/hooks/index.js,
    /// which execs this).
    Hooks {
        /// The policy (JSON: mode, proofs, audit, net_steps, rootfs,
        /// keep_jobs), as the installed shim passes it. Never read from the
        /// environment: the runner gives the hook the step's.
        #[arg(long)]
        policy: Option<String>,
    },
    /// A warm pool of official actions/runner processes wired to the hooks.
    Runner {
        #[command(subcommand)]
        cmd: runner::RunnerCmd,
    },
    /// Print a stored step key (with its trust state), manifest or run.
    Inspect {
        /// A step key or manifest id (b3:...) or a run id (ULID).
        id: String,
        #[arg(long)]
        ledger: Option<PathBuf>,
    },
    /// Pinned root filesystems for steps: import one, list them.
    Rootfs(rootfs::RootfsArgs),
    /// Run one step from a JSON request (the runner API; see crate::step).
    Step(step::StepArgs),
    /// Internal: print the host class as a sandboxed step sees it (run by
    /// Quod itself inside the sandbox; see rootfs::platform).
    #[command(name = "__host-probe", hide = true)]
    HostProbe,
    /// Internal: print the `.git` `quod run` serves for a checkout (see
    /// gitdir.rs) as tree entries (JSON, paths relative to `.git`), before
    /// its index is refreshed; the acceptance suite rebuilds the served
    /// tree with it.
    #[command(name = "__served-git", hide = true)]
    ServedGit {
        #[arg(long)]
        repo: PathBuf,
    },
    /// Print the toolchain profile a repo runs under, as NAME='VALUE' lines
    /// a shell can eval: toolchain, default setup and test steps, the
    /// directories to clear after setup, the sealed prefixes.
    Profile(profile::ProfileArgs),
    /// Run a vitest suite replaying, per test file, every file whose recorded
    /// inputs are unchanged: `quod vitest -- run --project unit`.
    Vitest(vitest::VitestArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<u8> {
            // The host probe runs in the sandbox with no state, no HOME and
            // nothing to log: it only reads the kernel's view and prints it.
            if matches!(cli.cmd, Cmd::HostProbe) {
                return rootfs::host_probe();
            }
            // vitest keeps its own ledger next to the project; no store.
            if let Cmd::Vitest(args) = cli.cmd {
                return vitest::cli(args);
            }
            let state = state::State::resolve(cli.state.as_deref())?;
            match cli.cmd {
                Cmd::Run(args) => {
                    // A SIGTERM, SIGINT or SIGHUP stops the step and lets
                    // this run unmount what it mounted (the next run sweeps
                    // what a SIGKILL leaves: State::claim_run_dir).
                    quod_sandbox::on_termination();
                    run::run(&state, args)
                }
                Cmd::Audit(args) => audit::audit(&state, args),
                Cmd::Status(args) => status::status(&state, args),
                Cmd::Bench(args) => bench::bench(&state, args),
                Cmd::Hooks { policy } => quod_hooks::Policy::from_json(policy.as_deref())
                    .and_then(|p| {
                        // The hooks key steps with this host's class, as
                        // `run` does (rootfs::platform), probed once a job.
                        quod_hooks::run(
                            &state.dir,
                            &mut std::io::stdin().lock(),
                            &p,
                            &rootfs::platform,
                        )
                    })
                    .map(run::exit_byte),
                Cmd::Runner { cmd } => runner::runner(&state, cmd),
                Cmd::Inspect { id, ledger } => {
                    inspect::inspect(&state, &id, ledger.as_deref()).map(|()| 0)
                }
                Cmd::Rootfs(args) => rootfs::cli(&state, args).map(|()| 0),
                Cmd::Step(args) => step::cli(&state, args),
                Cmd::HostProbe => rootfs::host_probe(),
                Cmd::ServedGit { repo } => {
                    let cas = quod_cas::Cas::open(state.cas_dir())?;
                    let repo = repo.canonicalize()?;
                    let entries: Vec<quod_cas::Entry> =
                        // As `graft`: a checkout it cannot serve gets none.
                        gitdir::served_git(&cas, &state.dir, &repo)
                            .ok()
                            .flatten()
                            .map(|t| t.iter().cloned().collect())
                            .unwrap_or_default();
                    println!("{}", serde_json::to_string(&entries)?);
                    Ok(0)
                }
                Cmd::Profile(args) => profile::profile(args).map(|()| 0),
                Cmd::Vitest(_) => unreachable!("handled before the state resolves"),
            }
        }));
    // A panic anywhere is a Quod failure too: exit 2 with the banner,
    // never a code a caller could mistake for the step's.
    let result = result.unwrap_or_else(|p| {
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        Err(anyhow::anyhow!("quod panicked: {msg}"))
    });
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            // The machine-readable marker (tracing goes to events.jsonl once
            // the state dir resolved; before that there is only the banner).
            tracing::error!(target: "quod", outcome = "error", exit_code = EXIT_QUOD_ERROR, error = %format!("{e:#}"), "quod error");
            say!("QUOD  ERROR (exit {EXIT_QUOD_ERROR}, not the step's result)");
            for cause in e.chain() {
                say!("  {cause}");
            }
            ExitCode::from(EXIT_QUOD_ERROR)
        }
    }
}
