//! The stillpass command line. Thin: wiring, flags and exit codes.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod bench;
mod inspect;
mod run;
mod snapshot;
mod state;

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = (|| -> anyhow::Result<u8> {
        let state = state::State::resolve(cli.state.as_deref())?;
        match cli.cmd {
            Cmd::Run(args) => run::run(&state, args),
            Cmd::Bench(args) => bench::bench(&state, args),
            Cmd::Inspect { id, ledger } => {
                inspect::inspect(&state, &id, ledger.as_deref()).map(|()| 0)
            }
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
