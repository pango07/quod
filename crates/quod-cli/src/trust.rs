//! The trust policy as command-line flags, and the small bridges between a
//! step's env and the trust layer.

use std::time::Duration;

use clap::Args;
use quod_cas::Tree;
use quod_key::Profile;
use quod_trust::{Mode, Policy, TrustError};

/// `--mode`, `--proofs`, `--audit`: shared by `run` (and read from
/// QUOD_MODE, QUOD_PROOFS, QUOD_AUDIT so a suite or the bench
/// sets them once).
#[derive(Args, Clone, Debug)]
pub struct PolicyArgs {
    /// observe (default): always execute and record; report what would have
    /// replayed and the time it would have saved. replay: replay proven
    /// results.
    #[arg(long, default_value = "observe", env = "QUOD_MODE", value_parser = parse_mode)]
    mode: Mode,
    /// Identical executions a result needs before it may replay (plus one
    /// for every recent divergence of its step key). 1 (trust v2): a result
    /// recorded once replays on the next identical tree.
    #[arg(long, default_value_t = quod_trust::DEFAULT_PROOFS, env = "QUOD_PROOFS")]
    proofs: u32,
    /// Would-be hits of a step key that replay mode re-executes and compares
    /// in full before any of the key replays blind (trust v2's shadow
    /// phase), counted from the key's last quarantine; the rate then decays
    /// as 1/k to --audit. 0: no shadow phase.
    #[arg(long, default_value_t = quod_trust::DEFAULT_SHADOW, env = "QUOD_SHADOW")]
    shadow: u32,
    /// The audit floor: the fraction of would-be hits that replay mode
    /// re-executes and compares instead of replaying once the shadow phase
    /// has decayed (0..=1). 0 turns every audit off, the shadow included:
    /// benchmarks only.
    #[arg(long, default_value_t = quod_trust::DEFAULT_AUDIT, env = "QUOD_AUDIT")]
    audit: f64,
    /// Let failing results (non-zero exit) prove themselves and replay. Off
    /// by default: an identical failure can be the box (load, a hang to go
    /// test's timeout), not the code. Output with a kill or timeout marker
    /// never replays either way.
    /// QUOD_REPLAY_FAILURES takes 1/0, true/false, yes/no, on/off.
    #[arg(
        long,
        env = "QUOD_REPLAY_FAILURES",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    replay_failures: bool,
}

impl PolicyArgs {
    pub fn policy(&self) -> Result<Policy, TrustError> {
        let p = Policy {
            mode: self.mode,
            proofs: self.proofs,
            shadow: self.shadow,
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

/// Paths under HOME left out of comparisons (`quod_trust::HOME_VOLATILE`).
pub use quod_trust::HOME_VOLATILE;

/// Output prefixes the trust layer leaves out when comparing two results:
/// what Quod provides the step as scratch, which starts empty (or
/// sealed) on every execution and holds per-execution state rather than
/// results. A directory the env names that the profile calls scratch
/// (`ephemeral`: Go's build cache, which go stamps with times) or a
/// download cache (`setup_cache`: npm's, which holds its debug logs; pip's
/// and uv's), whole; the carve-outs of the sealed dependency trees this
/// tree has, where the tool keeps its own caches (Go's module download
/// cache, vitest's results under node_modules/.vite); of HOME only the
/// `HOME_VOLATILE` paths. Everything else is compared, GOPATH (go
/// install's binaries) and a venv included. They are still recorded as
/// outputs; only the comparison skips them, and `--outputs` does not write
/// them. A directory the env does not point at is compared: a runner's own
/// env (the runner API) decides what is scratch, not the profile's.
/// Outputs a profile forbids (`no_outputs_under`) never get this far: they
/// are dropped before a result is recorded.
pub fn ignored_outputs(profile: &Profile, env: &[(String, String)], tree: &Tree) -> Vec<String> {
    let mut out = Vec::new();
    for (var, value) in env {
        let Some(dir) = quod_key::profile::workspace_relative(value) else {
            continue;
        };
        if var == "HOME" {
            out.extend(HOME_VOLATILE.iter().map(|v| format!("{dir}/{v}")));
        } else if profile
            .ephemeral
            .iter()
            .chain(profile.setup_cache)
            .any(|d| *d == dir)
        {
            out.push(dir);
        }
    }
    for s in profile.seals(tree) {
        out.extend(s.carve_outs);
    }
    out.sort();
    out.dedup();
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
    use quod_cas::{Entry, Hash};
    use quod_key::Toolchain;

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
        let go = Toolchain::Go.profile();
        let tree = Tree::from_leaves([
            Entry::file("go.mod", Hash::of(b"m"), 0o644),
            Entry::file(".quod/gomod/a.b/c@v1/x.go", Hash::of(b"x"), 0o444),
            Entry::file(".quod/gomod/cache/download/x", Hash::of(b"d"), 0o444),
        ]);
        let got = ignored_outputs(go, &go.env_vec(), &tree);
        assert_eq!(
            got,
            [
                ".quod/gocache",
                ".quod/gomod/cache",
                ".quod/home/.cache/gh",
                ".quod/home/.config/go/telemetry",
                ".quod/home/.local/state/gh",
                ".quod/home/.ssh/codespaces.auto",
                ".quod/home/.ssh/codespaces.auto.pub",
            ]
        );
        // HOME itself, GOPATH (go install's binaries) and the rest of HOME
        // are compared.
        let kept: Vec<&str> = [
            ".quod/home/tok",
            ".quod/home/go/bin/tool",
            ".quod/gopath/bin/tool",
            ".quod/home/.ssh/id_ed25519",
        ]
        .into_iter()
        .filter(|p| {
            !got.iter()
                .any(|g| *p == g.as_str() || p.starts_with(&format!("{g}/")))
        })
        .collect();
        assert_eq!(kept.len(), 4, "{kept:?}");
        // Without a module cache in the tree there is no carve-out to skip.
        let bare = Tree::from_leaves([Entry::file("go.mod", Hash::of(b"m"), 0o644)]);
        assert!(!ignored_outputs(go, &go.env_vec(), &bare).contains(&".quod/gomod/cache".into()));
        // A runner's env that puts the build cache elsewhere gets it compared.
        let mut env = go.env_vec();
        env.retain(|(k, _)| k != "GOCACHE");
        env.push(("GOCACHE".into(), "/workspace/build-cache".into()));
        let got = ignored_outputs(go, &env, &tree);
        assert!(
            !got.iter()
                .any(|g| g == ".quod/gocache" || g == "build-cache"),
            "{got:?}"
        );
    }

    /// Node: npm's cache (its debug logs) and the carve-outs of every sealed
    /// node_modules; Python: pip's and uv's caches, the venv compared.
    #[test]
    fn ignored_outputs_follow_the_profile() {
        let node = Toolchain::Node.profile();
        let tree = Tree::from_leaves([
            Entry::file("package.json", Hash::of(b"p"), 0o644),
            Entry::file("node_modules/dep/index.js", Hash::of(b"i"), 0o644),
            Entry::file("web/node_modules/x/index.js", Hash::of(b"x"), 0o644),
        ]);
        let got = ignored_outputs(node, &node.env_vec(), &tree);
        for want in [
            ".quod/npm-cache",
            "node_modules/.vite",
            "node_modules/.vitest",
            "web/node_modules/.vite",
        ] {
            assert!(got.contains(&want.to_string()), "{want}: {got:?}");
        }
        assert!(
            !got.iter()
                .any(|g| g == "node_modules" || g == "node_modules/dep")
        );
        let py = Toolchain::Python.profile();
        let tree = Tree::from_leaves([
            Entry::file("pyproject.toml", Hash::of(b"p"), 0o644),
            Entry::file(".quod/venv/bin/python", Hash::of(b"b"), 0o755),
        ]);
        let got = ignored_outputs(py, &py.env_vec(), &tree);
        assert!(got.contains(&".quod/pip-cache".to_string()), "{got:?}");
        assert!(got.contains(&".quod/uv-cache".to_string()), "{got:?}");
        assert!(!got.iter().any(|g| g.starts_with(".quod/venv")), "{got:?}");
    }
}
