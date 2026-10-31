//! The stillpass command line. Thin: wiring, flags and exit codes.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod bench;
mod deps;
mod inspect;
mod profile;
mod rootfs;
mod run;
mod seal;
mod snapshot;
mod state;
mod status;
mod step;
mod trust;

/// Exit code for Stillpass's own failures. A step can exit 2 by itself, so
/// the code alone does not say whose failure it was: a Stillpass failure
/// prints the `STILLPASS  ERROR` banner on stderr and writes an event with
/// `outcome: "error"` to events.jsonl, and leaves no run row (every run that
/// produced a step result has one, with its outcome).
pub const EXIT_STILLPASS_ERROR: u8 = 2;

#[derive(Parser)]
#[command(
    name = "stillpass",
    version,
    about = "CI that refuses to repeat itself (Phase 0 spike)"
)]
struct Cli {
    /// State directory holding the CAS, the ledger, run dirs and events.
    /// Defaults to $STILLPASS_HOME, else ~/.cache/stillpass.
    #[arg(long, global = true)]
    state: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run one step: `stillpass run [--] <cmd...>` or `--step <script>`.
    /// Observe mode (default) executes and records; replay mode replays
    /// results that proved themselves.
    Run(run::RunArgs),
    /// Hits, misses, time saved and the trust state of every step key of a repo.
    Status(status::StatusArgs),
    /// Measure observer overhead (A8) and hit rate over history (A11).
    Bench(bench::BenchArgs),
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
    /// Stillpass itself inside the sandbox; see rootfs::platform).
    #[command(name = "__host-probe", hide = true)]
    HostProbe,
    /// Print the toolchain profile a repo runs under, as NAME='VALUE' lines
    /// a shell can eval: toolchain, default setup and test steps, the
    /// directories to clear after setup, the sealed prefixes.
    Profile(profile::ProfileArgs),
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
            let state = state::State::resolve(cli.state.as_deref())?;
            match cli.cmd {
                Cmd::Run(args) => run::run(&state, args),
                Cmd::Status(args) => status::status(&state, args),
                Cmd::Bench(args) => bench::bench(&state, args),
                Cmd::Inspect { id, ledger } => {
                    inspect::inspect(&state, &id, ledger.as_deref()).map(|()| 0)
                }
                Cmd::Rootfs(args) => rootfs::cli(&state, args).map(|()| 0),
                Cmd::Step(args) => step::cli(&state, args),
                Cmd::HostProbe => rootfs::host_probe(),
                Cmd::Profile(args) => profile::profile(args).map(|()| 0),
            }
        }));
    // A panic anywhere is a Stillpass failure too: exit 2 with the banner,
    // never a code a caller could mistake for the step's.
    let result = result.unwrap_or_else(|p| {
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        Err(anyhow::anyhow!("stillpass panicked: {msg}"))
    });
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            // The machine-readable marker (tracing goes to events.jsonl once
            // the state dir resolved; before that there is only the banner).
            tracing::error!(target: "stillpass", outcome = "error", exit_code = EXIT_STILLPASS_ERROR, error = %format!("{e:#}"), "stillpass error");
            eprintln!("STILLPASS  ERROR (exit {EXIT_STILLPASS_ERROR}, not the step's result)");
            for cause in e.chain() {
                eprintln!("  {cause}");
            }
            ExitCode::from(EXIT_STILLPASS_ERROR)
        }
    }
}
