//! The hooks' side of the trust layer. Whether a recorded result may
//! replay, how a key proves itself, audits and quarantine are `sp_trust`'s
//! (the same rules as `stillpass run`); this module holds what only the
//! hooks need: the policy as the runner's shim passes it, and the command
//! files a step appends to (GITHUB_ENV, GITHUB_OUTPUT, ...) in a canonical
//! form, so that two runs that said the same thing store the same blob.

use anyhow::Context;
pub use sp_trust::Mode;

#[derive(Clone, Debug)]
pub struct Policy {
    pub mode: Mode,
    /// Identical results a manifest needs before it may replay.
    pub proofs: u32,
    /// Fraction of would-be hits (0..=1) that execute anyway and compare.
    pub audit: f64,
    /// Substrings of a `run:` script that mark it as possibly needing the
    /// network: it runs offline first, then again online, tainted.
    pub net_steps: Vec<String>,
    /// Steps run on the host's system paths instead of the job image
    /// (tests that are not about the image).
    pub host_root: bool,
    /// Keep job directories after cleanup (debugging, tests).
    pub keep_jobs: bool,
    /// Refs besides the default branch (`refs/heads/release`) whose push or
    /// dispatch jobs are writers: the operator's list of protected refs,
    /// every push to which is reviewed as the default branch's is. A ref
    /// listed here that anyone can push to reopens the poisoning a writer
    /// guards against (see `job::scope_of`).
    pub writer_refs: Vec<String>,
}

/// Scripts that fetch: the spike's `--allow-net` by pattern.
pub const DEFAULT_NET_STEPS: &[&str] = &[
    "go mod download",
    "go get ",
    "go install ",
    "npm ci",
    "npm install",
    "yarn install",
    "pnpm install",
    "pip install",
    "cargo fetch",
    "apt-get ",
    "apk add",
    "curl ",
    "wget ",
];

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            mode: Mode::Observe,
            proofs: sp_trust::DEFAULT_PROOFS,
            audit: sp_trust::DEFAULT_AUDIT,
            net_steps: DEFAULT_NET_STEPS.iter().map(|s| s.to_string()).collect(),
            host_root: false,
            keep_jobs: false,
            writer_refs: Vec::new(),
        }
    }
}

/// The policy as the runner's hook shim passes it (`stillpass hooks
/// --policy <json>`). Never read from the environment.
#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PolicyDoc {
    mode: Option<String>,
    proofs: Option<u32>,
    audit: Option<f64>,
    net_steps: Option<Vec<String>>,
    rootfs: Option<String>,
    keep_jobs: Option<bool>,
    writer_refs: Option<Vec<String>>,
}

impl Policy {
    /// `json` is the shim's policy document; `None` is the default policy.
    pub fn from_json(json: Option<&str>) -> anyhow::Result<Policy> {
        let doc: PolicyDoc = match json {
            Some(j) => serde_json::from_str(j).context("hook policy")?,
            None => PolicyDoc::default(),
        };
        let mut p = Policy::default();
        if let Some(m) = doc.mode.as_deref() {
            p.mode = m.parse().map_err(|e| anyhow::anyhow!("policy mode: {e}"))?;
        }
        if let Some(n) = doc.proofs {
            p.proofs = n;
        }
        if let Some(a) = doc.audit {
            p.audit = a;
        }
        p.trust()
            .validate()
            .map_err(|e| anyhow::anyhow!("hook policy: {e}"))?;
        if let Some(n) = doc.net_steps {
            p.net_steps = n.into_iter().filter(|x| !x.is_empty()).collect();
        }
        match doc.rootfs.as_deref() {
            Some("host") => p.host_root = true,
            Some("image") | None => {}
            Some(other) => anyhow::bail!("policy rootfs {other:?}: image or host"),
        }
        p.keep_jobs = doc.keep_jobs.unwrap_or(false);
        for r in doc.writer_refs.unwrap_or_default() {
            if !r.starts_with("refs/heads/") && !r.starts_with("refs/tags/") {
                anyhow::bail!(
                    "policy writer_refs: {r:?} is not a full ref (refs/heads/... or refs/tags/...)"
                );
            }
            p.writer_refs.push(r);
        }
        Ok(p)
    }

    /// What `sp_trust` decides by. A failure never proves itself in the
    /// hooks: an offline failure is run again and never recorded, and a
    /// step that fails online or on the runner's surface is never recorded
    /// either.
    pub fn trust(&self) -> sp_trust::Policy {
        sp_trust::Policy {
            mode: self.mode,
            proofs: self.proofs,
            audit: self.audit,
            replay_failures: false,
        }
    }

    pub fn mode_name(&self) -> &'static str {
        self.mode.as_str()
    }

    /// The rule that gives `script` the network, if any.
    pub fn net_rule(&self, script: &str) -> Option<&str> {
        self.net_steps
            .iter()
            .find(|r| script.contains(r.as_str()))
            .map(String::as_str)
    }
}

/// What a command file says, as the runner reads it: `NAME=VALUE` lines and
/// `NAME<<DELIM` heredocs (the runner picks the form by whichever of `=`
/// and `<<` comes first on the line; it splits on LF). `None` for a file
/// that is not key-value (GITHUB_PATH, the step summary) or does not parse.
pub fn command_meaning(name: &str, bytes: &[u8]) -> Option<Vec<(String, String)>> {
    if !matches!(name, "GITHUB_ENV" | "GITHUB_OUTPUT" | "GITHUB_STATE") {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.split('\n');
    let mut out = Vec::new();
    while let Some(line) = lines.next() {
        if line.is_empty() {
            continue;
        }
        let eq = line.find('=');
        let heredoc = line.find("<<");
        match (eq, heredoc) {
            (Some(e), h) if h.is_none_or(|h| e < h) => {
                out.push((line[..e].to_string(), line[e + 1..].to_string()));
            }
            (_, Some(h)) => {
                let (k, delim) = (&line[..h], &line[h + 2..]);
                if k.is_empty() || delim.is_empty() {
                    return None;
                }
                let mut value: Vec<&str> = Vec::new();
                loop {
                    let l = lines.next()?;
                    if l == delim {
                        break;
                    }
                    value.push(l);
                }
                out.push((k.to_string(), value.join("\n")));
            }
            _ => return None,
        }
    }
    Some(out)
}

/// The bytes stored (and replayed) for what a step appended to command file
/// `name`. A key-value file is rewritten in a canonical form with the same
/// meaning: `NAME=VALUE` for a one-line value, else a heredoc whose
/// delimiter is derived from the value, never from the step (the runner's
/// own `ghadelimiter_<uuid>` differs on every run, which would make every
/// result look different). Anything else is stored as written.
pub fn canonical_commands(name: &str, bytes: &[u8]) -> Vec<u8> {
    let Some(pairs) = command_meaning(name, bytes) else {
        return bytes.to_vec();
    };
    let mut out = String::new();
    for (k, v) in pairs {
        if v.contains('\n') || v.contains('\r') {
            let mut delim = format!(
                "ghadelimiter_stillpass_{}",
                &blake3::hash(v.as_bytes()).to_hex()[..16]
            );
            while v.split('\n').any(|l| l == delim) {
                delim.push('_');
            }
            out.push_str(&format!("{k}<<{delim}\n{v}\n{delim}\n"));
        } else {
            out.push_str(&format!("{k}={v}\n"));
        }
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_files_compare_by_meaning() {
        let a = "go-version<<ghadelimiter_1\n1.27.0\nghadelimiter_1\nx=1\n";
        let b = "go-version<<ghadelimiter_2\n1.27.0\nghadelimiter_2\nx=1\n";
        let c = "go-version<<ghadelimiter_2\n1.27.3\nghadelimiter_2\nx=1\n";
        let m = |s: &str| command_meaning("GITHUB_OUTPUT", s.as_bytes());
        assert_eq!(m(a), m(b));
        assert_ne!(m(a), m(c));
        assert_eq!(
            m(a).unwrap(),
            [
                ("go-version".to_string(), "1.27.0".to_string()),
                ("x".into(), "1".into())
            ]
        );
        assert!(
            m("go-version<<D\n1.27.0\n").is_none(),
            "unterminated heredoc"
        );
        assert!(command_meaning("GITHUB_PATH", b"/x\n").is_none());
        // Stored canonically: the random delimiters are gone, the meaning
        // stays, and a multi-line value keeps a heredoc that parses back.
        let canon = |s: &str| canonical_commands("GITHUB_OUTPUT", s.as_bytes());
        assert_eq!(canon(a), canon(b));
        assert_ne!(canon(a), canon(c));
        assert_eq!(canon(a), b"go-version=1.27.0\nx=1\n");
        let multi = "notes<<EOF_9\nline one\nline two\nEOF_9\n";
        let stored = canonical_commands("GITHUB_ENV", multi.as_bytes());
        assert_eq!(
            command_meaning("GITHUB_ENV", &stored),
            command_meaning("GITHUB_ENV", multi.as_bytes())
        );
        assert!(String::from_utf8_lossy(&stored).starts_with("notes<<ghadelimiter_stillpass_"));
        // Not key-value, or not parseable: kept as written.
        assert_eq!(canonical_commands("GITHUB_PATH", b"/x\n"), b"/x\n");
        assert_eq!(
            canonical_commands("GITHUB_OUTPUT", b"k<<D\nunterminated\n"),
            b"k<<D\nunterminated\n"
        );
    }

    #[test]
    fn policy_documents() {
        let p = Policy::from_json(Some(r#"{"mode":"replay","proofs":3,"audit":1}"#)).unwrap();
        assert_eq!((p.mode, p.proofs, p.audit), (Mode::Replay, 3, 1.0));
        assert!(!p.trust().replay_failures);
        assert!(
            p.writer_refs.is_empty(),
            "no writer refs but the default branch"
        );
        let w = Policy::from_json(Some(r#"{"writer_refs":["refs/heads/release"]}"#)).unwrap();
        assert_eq!(w.writer_refs, ["refs/heads/release"]);
        let d = Policy::from_json(None).unwrap();
        assert_eq!(
            (d.mode, d.proofs, d.audit),
            (
                Mode::Observe,
                sp_trust::DEFAULT_PROOFS,
                sp_trust::DEFAULT_AUDIT
            )
        );
        for bad in [
            r#"{"proofs":0}"#,
            r#"{"audit":2}"#,
            r#"{"mode":"fast"}"#,
            r#"{"rootfs":"vm"}"#,
            r#"{"unknown":1}"#,
            r#"{"writer_refs":["release"]}"#,
        ] {
            assert!(Policy::from_json(Some(bad)).is_err(), "{bad}");
        }
    }
}
