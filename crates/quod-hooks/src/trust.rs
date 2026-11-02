//! The hooks' side of the trust layer. Whether a recorded result may
//! replay, how a key proves itself, audits and quarantine are `quod_trust`'s
//! (the same rules as `quod run`); this module holds what only the
//! hooks need: the policy as the runner's shim passes it, and the command
//! files a step appends to (GITHUB_ENV, GITHUB_OUTPUT, ...) in a canonical
//! form, so that two runs that said the same thing store the same blob.

use anyhow::Context;
pub use quod_trust::Mode;

#[derive(Clone, Debug)]
pub struct Policy {
    pub mode: Mode,
    /// Identical results a manifest needs before it may replay.
    pub proofs: u32,
    /// Would-be hits of a key audited in full before any of it replays
    /// blind (trust v2's shadow phase); the rate then decays to `audit`.
    pub shadow: u32,
    /// Fraction of would-be hits (0..=1) that execute anyway and compare,
    /// once the shadow phase has decayed. 0 turns every audit off.
    pub audit: f64,
    /// A higher bar for steps whose `run:` script contains a substring
    /// (trust v2, "per pattern"): the first match wins.
    pub step_proofs: Vec<StepProofs>,
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
    /// Where a job image may come from for its own config env to exempt a
    /// variable from classification by name or shape
    /// (`secrets::classify_with`): `registry/namespace` prefixes, matched
    /// on path components (`docker.io/library`, the Docker Official
    /// Images, by default). An image pinned by digest qualifies wherever it
    /// is from. Either way only in a trusted scope (see `Job::image_env`).
    pub trusted_registries: Vec<String>,
}

/// The default `trusted_registries`: the Docker Official Images.
pub const DEFAULT_TRUSTED_REGISTRIES: &[&str] = &["docker.io/library"];

/// `{"contains": "go test", "proofs": 2}`: steps whose script contains the
/// substring need `proofs` identical executions before they replay.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepProofs {
    pub contains: String,
    pub proofs: u32,
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
            proofs: quod_trust::DEFAULT_PROOFS,
            shadow: quod_trust::DEFAULT_SHADOW,
            audit: quod_trust::DEFAULT_AUDIT,
            step_proofs: Vec::new(),
            net_steps: DEFAULT_NET_STEPS.iter().map(|s| s.to_string()).collect(),
            host_root: false,
            keep_jobs: false,
            writer_refs: Vec::new(),
            trusted_registries: DEFAULT_TRUSTED_REGISTRIES
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

/// The policy as the runner's hook shim passes it (`quod hooks
/// --policy <json>`). Never read from the environment.
#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PolicyDoc {
    mode: Option<String>,
    proofs: Option<u32>,
    shadow: Option<u32>,
    audit: Option<f64>,
    step_proofs: Option<Vec<StepProofs>>,
    net_steps: Option<Vec<String>>,
    rootfs: Option<String>,
    keep_jobs: Option<bool>,
    writer_refs: Option<Vec<String>>,
    trusted_registries: Option<Vec<String>>,
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
        if let Some(n) = doc.shadow {
            p.shadow = n;
        }
        if let Some(a) = doc.audit {
            p.audit = a;
        }
        p.trust()
            .validate()
            .map_err(|e| anyhow::anyhow!("hook policy: {e}"))?;
        for s in doc.step_proofs.unwrap_or_default() {
            if s.contains.is_empty() {
                anyhow::bail!("policy step_proofs: an empty `contains` would match every step");
            }
            quod_trust::Policy {
                proofs: s.proofs,
                ..p.trust()
            }
            .validate()
            .map_err(|e| anyhow::anyhow!("hook policy step_proofs {:?}: {e}", s.contains))?;
            p.step_proofs.push(s);
        }
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
        if let Some(t) = doc.trusted_registries {
            for r in &t {
                if r.is_empty() || r.contains("://") || r.ends_with('/') || r.contains('@') {
                    anyhow::bail!(
                        "policy trusted_registries: {r:?} is not a registry or registry/namespace (docker.io/library, ghcr.io/owner)"
                    );
                }
            }
            p.trusted_registries = t;
        }
        Ok(p)
    }

    /// What `quod_trust` decides by. A failure never proves itself in the
    /// hooks: an offline failure is run again and never recorded, and a
    /// step that fails online or on the runner's surface is never recorded
    /// either. Audits stay inline in the hooks: a deferred audit needs the
    /// step's hermetic surface (its masks, layers and command files) kept
    /// past the job, which the hooks do not store (docs/trust-v2.md,
    /// "Deferred audits", the hooks follow-up).
    pub fn trust(&self) -> quod_trust::Policy {
        quod_trust::Policy {
            mode: self.mode,
            proofs: self.proofs,
            shadow: self.shadow,
            audit: self.audit,
            replay_failures: false,
            defer: false,
        }
    }

    /// `trust()` for one step: its `step_proofs` bar when a pattern matches
    /// the script, the policy's otherwise.
    pub fn trust_for(&self, script: &str) -> quod_trust::Policy {
        let mut p = self.trust();
        if let Some(s) = self
            .step_proofs
            .iter()
            .find(|s| script.contains(s.contains.as_str()))
        {
            p.proofs = s.proofs;
        }
        p
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
                "ghadelimiter_quod_{}",
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
        assert!(String::from_utf8_lossy(&stored).starts_with("notes<<ghadelimiter_quod_"));
        // Not key-value, or not parseable: kept as written.
        assert_eq!(canonical_commands("GITHUB_PATH", b"/x\n"), b"/x\n");
        assert_eq!(
            canonical_commands("GITHUB_OUTPUT", b"k<<D\nunterminated\n"),
            b"k<<D\nunterminated\n"
        );
    }

    #[test]
    fn policy_documents() {
        let p = Policy::from_json(Some(r#"{"mode":"replay","proofs":3,"shadow":5,"audit":1}"#))
            .unwrap();
        assert_eq!(
            (p.mode, p.proofs, p.shadow, p.audit),
            (Mode::Replay, 3, 5, 1.0)
        );
        assert!(!p.trust().replay_failures);
        assert_eq!(p.trust().shadow, 5);
        assert!(
            p.writer_refs.is_empty(),
            "no writer refs but the default branch"
        );
        assert_eq!(p.trusted_registries, ["docker.io/library"]);
        let t = Policy::from_json(Some(r#"{"trusted_registries":["ghcr.io/acme"]}"#)).unwrap();
        assert_eq!(t.trusted_registries, ["ghcr.io/acme"]);
        let w = Policy::from_json(Some(r#"{"writer_refs":["refs/heads/release"]}"#)).unwrap();
        assert_eq!(w.writer_refs, ["refs/heads/release"]);
        let d = Policy::from_json(None).unwrap();
        assert_eq!(
            (d.mode, d.proofs, d.shadow, d.audit),
            (
                Mode::Observe,
                quod_trust::DEFAULT_PROOFS,
                quod_trust::DEFAULT_SHADOW,
                quod_trust::DEFAULT_AUDIT
            )
        );
        // Trust v2: a higher bar per step pattern; the first match wins.
        let s = Policy::from_json(Some(
            r#"{"step_proofs":[{"contains":"go test","proofs":2},{"contains":"test","proofs":4}]}"#,
        ))
        .unwrap();
        assert_eq!(s.trust_for("go test ./...").proofs, 2);
        assert_eq!(s.trust_for("npm test").proofs, 4);
        assert_eq!(s.trust_for("go build ./...").proofs, 1);
        assert_eq!(s.trust_for("go build ./...").shadow, s.shadow);
        for bad in [
            r#"{"proofs":0}"#,
            r#"{"audit":2}"#,
            r#"{"shadow":100000}"#,
            r#"{"mode":"fast"}"#,
            r#"{"rootfs":"vm"}"#,
            r#"{"unknown":1}"#,
            r#"{"writer_refs":["release"]}"#,
            r#"{"trusted_registries":["https://ghcr.io"]}"#,
            r#"{"trusted_registries":[""]}"#,
            r#"{"step_proofs":[{"contains":"","proofs":2}]}"#,
            r#"{"step_proofs":[{"contains":"x","proofs":0}]}"#,
            r#"{"step_proofs":[{"pattern":"x","proofs":2}]}"#,
        ] {
            assert!(Policy::from_json(Some(bad)).is_err(), "{bad}");
        }
    }
}
