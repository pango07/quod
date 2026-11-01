use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use anyhow::bail;
use quod_cas::Hash;
use quod_ledger::Ledger;
use quod_replay::human_time;

use crate::state::State;

pub fn inspect(state: &State, id: &str, ledger: Option<&Path>) -> anyhow::Result<()> {
    let text = render(state, id, ledger)?;
    // A closed pipe (`| head`) is the reader's choice, not an error.
    match std::io::stdout().lock().write_all(text.as_bytes()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        r => Ok(r?),
    }
}

fn render(state: &State, id: &str, ledger: Option<&Path>) -> anyhow::Result<String> {
    let mut o = String::new();
    let ledger = Ledger::open(ledger.unwrap_or(&state.ledger_path()))?;
    if let Ok(h) = id.parse::<Hash>() {
        if let Some(m) = ledger.manifest(&h)? {
            writeln!(o, "manifest {h}")?;
            writeln!(o, "{}", serde_json::to_string_pretty(&m)?)?;
            return Ok(o);
        }
        let rows = ledger.manifests_for_key(&h)?;
        if rows.is_empty() {
            bail!("{h} is neither a manifest id nor a step key with manifests");
        }
        writeln!(o, "step key {h}: {} manifest(s), newest first", rows.len())?;
        if let Some(json) = ledger.step_key_json(&h)? {
            // The environment pin (rootfs or host, arch, kernel) and the
            // rest of what the key was computed from.
            let v: serde_json::Value = serde_json::from_str(&json)?;
            writeln!(o, "{}", serde_json::to_string_pretty(&v)?)?;
        }
        for r in rows {
            writeln!(
                o,
                "  {}  {:>7} entries  run {}  {}",
                r.id, r.entries, r.run_id, r.created_at
            )?;
        }
        o += &render_trust(&ledger, &h)?;
        return Ok(o);
    }
    let Some(run) = ledger.run(id)? else {
        bail!("no run, manifest or step key {id:?}");
    };
    writeln!(
        o,
        "run {}  started {}  commit {}  host {}  observer {}  outcome {}  mode {}",
        run.id,
        run.started_at,
        run.commit_sha.as_deref().unwrap_or("-"),
        run.host.as_deref().unwrap_or("-"),
        run.observer.as_deref().unwrap_or("-"),
        run.outcome.as_deref().unwrap_or("-"),
        run.mode.as_deref().unwrap_or("-")
    )?;
    for r in ledger.results_for_run(id)? {
        writeln!(o, "{}", serde_json::to_string_pretty(&r)?)?;
    }
    Ok(o)
}

/// The trust facts of a step key: its state and counters, the proof streak
/// of every manifest, and the newest audits.
fn render_trust(ledger: &Ledger, key: &Hash) -> anyhow::Result<String> {
    let mut o = String::new();
    let Some(t) = ledger.trust(key)? else {
        writeln!(
            o,
            "trust: no state yet (recorded before the trust layer, or never executed)"
        )?;
        return Ok(o);
    };
    writeln!(
        o,
        "trust: {}  eligible {}  replayed {}  audited {}  divergences {}  cmd {}",
        t.state.as_str(),
        t.eligible,
        t.replays,
        t.audits,
        t.divergences,
        t.cmd.as_deref().unwrap_or("-")
    )?;
    if let Some(at) = &t.quarantined_at {
        writeln!(
            o,
            "  quarantined since {}: {}",
            human_time(at),
            t.quarantine_reason.as_deref().unwrap_or("")
        )?;
    }
    let trailing = ledger.audits(Some(key), quod_trust::TRAILING_AUDITS)?;
    let (diverged, total) = quod_trust::divergence_rate(&trailing);
    if total == 0 {
        writeln!(o, "  divergence rate: no audits yet")?;
    } else {
        writeln!(
            o,
            "  divergence rate: {diverged} of {total} audits ({:.1}%) over the trailing {total}",
            diverged as f64 / total as f64 * 100.0
        )?;
    }
    for p in ledger.proofs_for_key(key)? {
        writeln!(
            o,
            "  manifest {}  proofs {}  last run {}",
            p.manifest_id, p.streak, p.last_run
        )?;
    }
    for a in ledger.audits(Some(key), 10)? {
        writeln!(
            o,
            "  audit {}  {}  {}  {}  {}",
            human_time(&a.at),
            a.phase,
            if a.diverged { "DIVERGED" } else { "ok" },
            a.actual_run.as_deref().unwrap_or("-"),
            a.detail.as_deref().unwrap_or("")
        )?;
    }
    Ok(o)
}
