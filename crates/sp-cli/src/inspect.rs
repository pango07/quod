use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use anyhow::bail;
use sp_cas::Hash;
use sp_ledger::Ledger;

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
        for r in rows {
            writeln!(
                o,
                "  {}  {:>7} entries  run {}  {}",
                r.id, r.entries, r.run_id, r.created_at
            )?;
        }
        return Ok(o);
    }
    let Some(run) = ledger.run(id)? else {
        bail!("no run, manifest or step key {id:?}");
    };
    writeln!(
        o,
        "run {}  started {}  commit {}  host {}  observer {}",
        run.id,
        run.started_at,
        run.commit_sha.as_deref().unwrap_or("-"),
        run.host.as_deref().unwrap_or("-"),
        run.observer.as_deref().unwrap_or("-")
    )?;
    for r in ledger.results_for_run(id)? {
        writeln!(o, "{}", serde_json::to_string_pretty(&r)?)?;
    }
    Ok(o)
}
