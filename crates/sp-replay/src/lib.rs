//! Given a matched result: print the receipt, write outputs into the
//! workspace, replay recorded stdout and stderr, return the recorded exit
//! code. Knows nothing about how the result was produced.

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use sp_cas::{Cas, CasError, Hash, Kind, remove_any};
use sp_manifest::StepResult;

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error(transparent)]
    Cas(#[from] CasError),
    #[error("replay output: {0}")]
    Io(#[from] io::Error),
}

/// The receipt, exactly as the Schemas section prints it, followed by one
/// indented line per entry of `extra` (the trust facts, which the caller
/// knows and this crate does not).
pub fn receipt(result: &StepResult, inputs: usize, replay: Duration, extra: &[String]) -> String {
    let recorded = Duration::from_millis(result.duration_ms);
    let mut s = format!(
        "STILLPASS  REPLAYED\n  proved by run {} on {}\n  step b3:{}  manifest b3:{}  {} inputs matched\n  recorded {}  replay {}  saved {}\n",
        &result.run_id[..result.run_id.len().min(8)],
        human_time(&result.recorded_at),
        result.step_key.short(),
        result.manifest_id.short(),
        group(inputs),
        fmt_duration(recorded),
        fmt_duration(replay),
        fmt_duration(recorded.saturating_sub(replay)),
    );
    for line in extra {
        s.push_str("  ");
        s.push_str(line);
        s.push('\n');
    }
    s
}

/// Writes the result's outputs into `workspace` (eagerly), applying
/// deletions first.
pub fn materialize_outputs(
    result: &StepResult,
    cas: &Cas,
    workspace: &Path,
) -> Result<(), ReplayError> {
    for e in result.outputs.iter().filter(|e| e.k == Kind::Deleted) {
        remove_any(&workspace.join(&e.p))?;
    }
    cas.materialize_entries(
        result.outputs.iter().filter(|e| e.k != Kind::Deleted),
        workspace,
    )?;
    Ok(())
}

/// Streams the recorded stdout and stderr blobs, in that order, and returns
/// the recorded exit code. The receipt is the caller's to print first.
pub fn replay_log(
    result: &StepResult,
    cas: &Cas,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<i32, ReplayError> {
    copy_blob(cas, &result.stdout, out)?;
    copy_blob(cas, &result.stderr, err)?;
    Ok(result.exit_code)
}

fn copy_blob(cas: &Cas, h: &Hash, w: &mut impl Write) -> Result<(), ReplayError> {
    let mut f = cas.open_blob(h)?;
    match io::copy(&mut f, w).and_then(|_| w.flush()) {
        // A closed pipe is the reader's choice.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        r => Ok(r?),
    }
}

/// `2026-11-14T09:12:03.007Z` -> `2026-11-14 09:12 UTC`.
pub fn human_time(rfc3339: &str) -> String {
    match (rfc3339.get(..10), rfc3339.get(11..16)) {
        (Some(d), Some(t)) => format!("{d} {t} UTC"),
        _ => rfc3339.to_string(),
    }
}

/// 2314 -> "2,314".
pub fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Durations as receipts print them: `4m12s` from a minute up, else `0.4s`.
pub fn fmt_duration(d: Duration) -> String {
    if d.as_secs() >= 60 {
        let s = d.as_secs();
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sp_cas::Entry;

    fn result(cas: &Cas) -> StepResult {
        StepResult {
            v: 1,
            step_key: "b3:4c1e9a07f3d2c1b0a9f8e7d6c5b4a3928170605f4e3d2c1b0a99887766554433"
                .parse()
                .unwrap(),
            manifest_id: "b3:9a07f2d1e0c9b8a7f6e5d4c3b2a1908f7e6d5c4b3a29180706f5e4d3c2b1a099"
                .parse()
                .unwrap(),
            run_id: "01JCK3Q9ZX5YV8W2T4R6P0N1M3".into(),
            recorded_at: "2026-11-14T09:12:03.000Z".into(),
            exit_code: 1,
            stdout: cas.put(b"ok  \tpkg/cmd\t0.412s\n").unwrap(),
            stderr: cas.put(b"warn\n").unwrap(),
            outputs: vec![
                Entry::file(
                    ".stillpass/gocache/ab/abcd",
                    cas.put(b"cache").unwrap(),
                    0o644,
                ),
                Entry::dir(".stillpass/empty", Hash::of(b"")),
                Entry::deleted("tmp/scratch"),
            ],
            root_outputs: vec![],
            duration_ms: 252_310,
        }
    }

    #[test]
    fn receipt_matches_the_golden() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path()).unwrap();
        let trust =
            "trust: proven by 3 identical executions, audited 17 times, 0 divergences".to_string();
        let text = receipt(&result(&cas), 2314, Duration::from_millis(400), &[trust]);
        let golden = include_str!("../../../fixtures/receipt.txt");
        assert_eq!(text, golden);
        let bare = receipt(&result(&cas), 2314, Duration::from_millis(400), &[]);
        assert_eq!(bare.lines().count(), 4);
        assert!(golden.starts_with(&bare));
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(Duration::from_millis(252_310)), "4m12s");
        assert_eq!(fmt_duration(Duration::from_millis(420)), "0.4s");
        assert_eq!(fmt_duration(Duration::from_secs(60)), "1m00s");
        assert_eq!(group(999), "999");
        assert_eq!(group(1_234_567), "1,234,567");
    }

    #[test]
    fn replays_log_and_outputs() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let r = result(&cas);
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(ws.join("tmp/scratch/deep")).unwrap();
        materialize_outputs(&r, &cas, &ws).unwrap();
        assert!(!ws.join("tmp/scratch").exists());
        assert_eq!(
            std::fs::read(ws.join(".stillpass/gocache/ab/abcd")).unwrap(),
            b"cache"
        );
        assert!(ws.join(".stillpass/empty").is_dir());
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(replay_log(&r, &cas, &mut out, &mut err).unwrap(), 1);
        assert_eq!(out, b"ok  \tpkg/cmd\t0.412s\n");
        assert_eq!(err, b"warn\n");
    }
}
