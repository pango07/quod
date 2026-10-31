//! `stillpass status`: what the ledger says about a repo. Hits, misses,
//! time saved (and, in observe mode, time that would have been saved), and
//! every step key's trust state with its trailing divergence rate.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use clap::Args;
use sp_cas::Hash;
use sp_ledger::{Ledger, RunRow, TrustState};
use sp_replay::{fmt_duration, human_time};
use sp_trust::TRAILING_AUDITS;

use crate::deps;
use crate::state::State;

#[derive(Args)]
pub struct StatusArgs {
    /// The repo to report on (default: the git toplevel of the current
    /// directory).
    #[arg(long)]
    repo: Option<PathBuf>,
    /// Every repo in the ledger.
    #[arg(long)]
    all: bool,
    #[arg(long)]
    ledger: Option<PathBuf>,
    /// Identical executions a result needs before it may replay (for the
    /// proofs column; the same value `run` uses).
    #[arg(long, default_value_t = sp_trust::DEFAULT_PROOFS, env = "STILLPASS_PROOFS")]
    proofs: u32,
}

pub fn status(state: &State, args: StatusArgs) -> anyhow::Result<u8> {
    let ledger_path = args.ledger.clone().unwrap_or_else(|| state.ledger_path());
    let ledger = Ledger::open(&ledger_path)?;
    let (filter, label) = if args.all {
        (None, "all repos".to_string())
    } else {
        let repo = match &args.repo {
            Some(r) => r
                .canonicalize()
                .with_context(|| format!("repo {}", r.display()))?,
            None => {
                let cwd = std::env::current_dir()?;
                deps::toplevel(&cwd).unwrap_or(cwd)
            }
        };
        (Some(deps::repo_id(&repo)), repo.display().to_string())
    };
    let runs = ledger.runs(filter.as_deref())?;
    let text = render(&ledger, &ledger_path, &label, &runs, args.proofs)?;
    match std::io::stdout().lock().write_all(text.as_bytes()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(0),
        r => r.map(|()| 0).map_err(Into::into),
    }
}

fn render(
    ledger: &Ledger,
    ledger_path: &Path,
    label: &str,
    runs: &[RunRow],
    proofs: u32,
) -> anyhow::Result<String> {
    let mut o = String::new();
    writeln!(o, "STILLPASS  STATUS  {label}")?;
    writeln!(
        o,
        "  ledger {}  {} run(s)",
        ledger_path.display(),
        runs.len()
    )?;
    let mut by_outcome: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut saved, mut would_save) = (Duration::ZERO, Duration::ZERO);
    let mut executed = Duration::ZERO;
    for r in runs {
        let outcome = r.outcome.as_deref().unwrap_or("unknown");
        *by_outcome.entry(outcome).or_default() += 1;
        let ms = Duration::from_millis(r.saved_ms.unwrap_or(0));
        match outcome {
            "replayed" => saved += ms,
            "would-replay" => would_save += ms,
            "miss" | "proving" | "audit" | "diverged" => {
                executed += Duration::from_millis(r.wall_ms.unwrap_or(0))
            }
            _ => {}
        }
    }
    let n = |k: &str| by_outcome.get(k).copied().unwrap_or(0);
    writeln!(
        o,
        "  replayed {} (saved {})   would have replayed {} (observe mode: {} not saved)",
        n("replayed"),
        fmt_duration(saved),
        n("would-replay"),
        fmt_duration(would_save)
    )?;
    let executed_n = n("miss") + n("proving") + n("audit") + n("diverged");
    writeln!(
        o,
        "  executed {} ({}): {} miss, {} proving, {} audit, {} diverged; {} timed out, {} killed, {} network, {} unknown",
        executed_n,
        fmt_duration(executed),
        n("miss"),
        n("proving"),
        n("audit"),
        n("diverged"),
        n("timeout"),
        n("killed"),
        n("net"),
        n("unknown")
    )?;

    let keys: BTreeSet<Hash> = runs.iter().filter_map(|r| r.step_key).collect();
    let mut rows = Vec::new();
    for k in &keys {
        if let Some(t) = ledger.trust(k)? {
            let best = ledger.proofs_for_key(k)?.first().map_or(0, |p| p.streak);
            rows.push((t, best));
        }
    }
    let count = |s: TrustState| rows.iter().filter(|(t, _)| t.state == s).count();
    writeln!(
        o,
        "  trust: {} step key(s): {} proven, {} observing, {} quarantined",
        rows.len(),
        count(TrustState::Proven),
        count(TrustState::Observing),
        count(TrustState::Quarantined)
    )?;
    for (t, best) in &rows {
        let needed = proofs + u32::try_from(t.divergences).unwrap_or(u32::MAX);
        let mut line = format!(
            "    {:<11}  proofs {}/{}  audited {}x  {} divergence{}  replayed {}x",
            t.state.as_str(),
            best,
            needed,
            t.audits,
            t.divergences,
            if t.divergences == 1 { "" } else { "s" },
            t.replays
        );
        if let (TrustState::Quarantined, Some(at)) = (t.state, &t.quarantined_at) {
            let _ = write!(
                line,
                "  since {}: {}",
                human_time(at),
                t.quarantine_reason.as_deref().unwrap_or("")
            );
        }
        writeln!(o, "{line}")?;
        writeln!(
            o,
            "                 b3:{}  {}",
            t.step_key.short(),
            t.cmd.as_deref().unwrap_or("-")
        )?;
    }
    let audits: Vec<_> = ledger
        .audits(None, TRAILING_AUDITS * 4)?
        .into_iter()
        .filter(|a| keys.contains(&a.step_key))
        .take(TRAILING_AUDITS)
        .collect();
    let (diverged, total) = sp_trust::divergence_rate(&audits);
    if total == 0 {
        writeln!(o, "  divergence rate: no audits yet")?;
    } else {
        writeln!(
            o,
            "  divergence rate: {diverged} of {total} audits ({:.1}%) over the trailing {}",
            diverged as f64 / total as f64 * 100.0,
            TRAILING_AUDITS.min(total)
        )?;
    }
    Ok(o)
}
