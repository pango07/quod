//! The trust policy as command-line flags, and the small bridges between a
//! step's env and the trust layer.

use std::time::Duration;

use clap::Args;
use sp_trust::{Mode, Policy, TrustError};

/// `--mode`, `--proofs`, `--audit`: shared by `run` (and read from
/// STILLPASS_MODE, STILLPASS_PROOFS, STILLPASS_AUDIT so a suite or the bench
/// sets them once).
#[derive(Args, Clone, Debug)]
pub struct PolicyArgs {
    /// observe (default): always execute and record; report what would have
    /// replayed and the time it would have saved. replay: replay proven
    /// results.
    #[arg(long, default_value = "observe", env = "STILLPASS_MODE", value_parser = parse_mode)]
    mode: Mode,
    /// Identical executions a result needs before it may replay (plus one
    /// for every past divergence of its step key).
    #[arg(long, default_value_t = sp_trust::DEFAULT_PROOFS, env = "STILLPASS_PROOFS")]
    proofs: u32,
    /// Fraction of would-be hits that replay mode re-executes and compares
    /// instead of replaying (0..=1). 0 turns audit off: benchmarks only.
    #[arg(long, default_value_t = sp_trust::DEFAULT_AUDIT, env = "STILLPASS_AUDIT")]
    audit: f64,
    /// Let failing results (non-zero exit) prove themselves and replay. Off
    /// by default: an identical failure can be the box (load, a hang to go
    /// test's timeout), not the code. Output with a kill or timeout marker
    /// never replays either way.
    #[arg(long, env = "STILLPASS_REPLAY_FAILURES")]
    replay_failures: bool,
}

impl PolicyArgs {
    pub fn policy(&self) -> Result<Policy, TrustError> {
        let p = Policy {
            mode: self.mode,
            proofs: self.proofs,
            audit: self.audit,
            replay_failures: self.replay_failures,
        };
        p.validate()?;
        Ok(p)
    }
}

fn parse_mode(s: &str) -> Result<Mode, String> {
    s.parse().map_err(|e: TrustError| e.to_string())
}

/// `30m`, `2s`, `1h`, `90` (seconds), `1.5s`; `0` means no timeout.
pub fn parse_timeout(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let (num, unit) = match s.find(|c: char| c.is_ascii_alphabetic()) {
        Some(i) => s.split_at(i),
        None => (s, "s"),
    };
    let n: f64 = num
        .parse()
        .map_err(|_| format!("{s:?} is not a duration like 30m, 2s or 1h"))?;
    let secs = match unit {
        "s" | "sec" | "secs" => n,
        "m" | "min" | "mins" => n * 60.0,
        "h" | "hr" | "hrs" => n * 3600.0,
        "ms" => n / 1000.0,
        _ => return Err(format!("{s:?}: unknown unit {unit:?} (use ms, s, m or h)")),
    };
    if !secs.is_finite() || secs < 0.0 {
        return Err(format!("{s:?} is not a non-negative duration"));
    }
    Ok(Duration::from_secs_f64(secs))
}

/// Paths under HOME that tools fill with per-execution state and that are
/// therefore not compared (nor written out by `--outputs`). Everything else
/// a step leaves in HOME is compared like any output: `go install` into a
/// HOME-relative GOPATH, a token, a generated config are results. Each entry
/// is here because cli/cli's tests write it differently on every run:
/// gh's HTTP cache (keys include random httptest ports), Go's telemetry
/// counters (date-stamped files), gh's state dir (a random device id), and
/// the SSH key pair gh generates for codespaces.
pub const HOME_VOLATILE: &[&str] = &[
    ".cache/gh",
    ".config/go/telemetry",
    ".local/state",
    ".ssh/codespaces.auto",
    ".ssh/codespaces.auto.pub",
];

/// Output prefixes the trust layer leaves out when comparing two results:
/// the directories Stillpass provides the step as scratch, which start
/// empty (or, for the module cache, sealed) on every execution and hold
/// per-execution state rather than results. The build cache (GOCACHE)
/// whole, which go stamps with times; of the module cache only its `cache/`
/// directory, where go writes (the rest is a sealed input); of HOME only
/// the `HOME_VOLATILE` paths. They are still recorded as outputs; only the
/// comparison skips them, and `--outputs` does not write them.
pub fn ignored_outputs(env: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, var) in crate::deps::cache_dirs(env) {
        match var.as_str() {
            "GOCACHE" => out.push(format!(".stillpass/{name}")),
            "GOMODCACHE" => out.push(format!(".stillpass/{name}/cache")),
            "HOME" => out.extend(
                HOME_VOLATILE
                    .iter()
                    .map(|v| format!(".stillpass/{name}/{v}")),
            ),
            _ => {}
        }
    }
    out
}

/// Quotes a command line into one `sh` script: `go test ./...` stays as is,
/// an argument with spaces or shell characters is single-quoted.
pub fn shell_join(cmd: &[String]) -> String {
    cmd.iter()
        .map(|a| {
            let plain = !a.is_empty()
                && a.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-./:=@%+,".contains(&b));
            if plain {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts() {
        assert_eq!(parse_timeout("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_timeout("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_timeout("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_timeout("90").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_timeout("1.5s").unwrap(), Duration::from_millis(1500));
        assert_eq!(parse_timeout("250ms").unwrap(), Duration::from_millis(250));
        assert_eq!(parse_timeout("0").unwrap(), Duration::ZERO);
        assert!(
            parse_timeout("-1s").is_err()
                && parse_timeout("1d").is_err()
                && parse_timeout("x").is_err()
        );
    }

    #[test]
    fn shell_join_quotes_only_what_needs_it() {
        let s = |v: &[&str]| shell_join(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(s(&["go", "test", "./..."]), "go test ./...");
        assert_eq!(
            s(&["go", "test", "-run", "TestX/sub", "-v"]),
            "go test -run TestX/sub -v"
        );
        assert_eq!(
            s(&["sh", "-c", "echo hi; exit 3"]),
            "sh -c 'echo hi; exit 3'"
        );
        assert_eq!(s(&["echo", "it's"]), "echo 'it'\\''s'");
        assert_eq!(s(&["echo", ""]), "echo ''");
        assert_eq!(s(&["env", "A=b c"]), "env 'A=b c'");
    }

    #[test]
    fn ignored_outputs_are_the_build_cache_the_module_cache_carve_out_and_volatile_home() {
        let env = sp_key::Toolchain::Go.profile().env_vec();
        let mut got = ignored_outputs(&env);
        got.sort();
        assert_eq!(
            got,
            [
                ".stillpass/gocache",
                ".stillpass/gomod/cache",
                ".stillpass/home/.cache/gh",
                ".stillpass/home/.config/go/telemetry",
                ".stillpass/home/.local/state",
                ".stillpass/home/.ssh/codespaces.auto",
                ".stillpass/home/.ssh/codespaces.auto.pub",
            ]
        );
        // HOME itself, GOPATH (go install's binaries) and the rest of HOME
        // are compared.
        let kept: Vec<&str> = [
            ".stillpass/home/tok",
            ".stillpass/home/go/bin/tool",
            ".stillpass/gopath/bin/tool",
            ".stillpass/home/.ssh/id_ed25519",
        ]
        .into_iter()
        .filter(|p| {
            !got.iter()
                .any(|g| *p == g.as_str() || p.starts_with(&format!("{g}/")))
        })
        .collect();
        assert_eq!(kept.len(), 4, "{kept:?}");
    }
}
