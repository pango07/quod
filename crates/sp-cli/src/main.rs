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
mod step;

/// Exit code for Stillpass's own failures, distinct from any step's.
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
    /// Run one step: replay it if a proven result matches, else execute and record.
    Run(run::RunArgs),
    /// Measure observer overhead (A8) and hit rate over history (A11).
    Bench(bench::BenchArgs),
    /// Print a stored step key, manifest or run.
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
    let result = (|| -> anyhow::Result<u8> {
        // The host probe runs in the sandbox with no state, no HOME and
        // nothing to log: it only reads the kernel's view and prints it.
        if matches!(cli.cmd, Cmd::HostProbe) {
            return rootfs::host_probe();
        }
        let state = state::State::resolve(cli.state.as_deref())?;
        match cli.cmd {
            Cmd::Run(args) => run::run(&state, args),
            Cmd::Bench(args) => bench::bench(&state, args),
            Cmd::Inspect { id, ledger } => {
                inspect::inspect(&state, &id, ledger.as_deref()).map(|()| 0)
            }
            Cmd::Rootfs(args) => rootfs::cli(&state, args).map(|()| 0),
            Cmd::Step(args) => step::cli(&state, args),
            Cmd::HostProbe => rootfs::host_probe(),
            Cmd::Profile(args) => profile::profile(args).map(|()| 0),
        }
    })();
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("STILLPASS  ERROR (exit {EXIT_STILLPASS_ERROR}, not the step's result)");
            for cause in e.chain() {
                eprintln!("  {cause}");
            }
            ExitCode::from(EXIT_STILLPASS_ERROR)
        }
    }
}
