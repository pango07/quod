//! Canonical step key: the exact JSON form, env normalization and denylist,
//! hashing. Pure functions; no filesystem, no storage.
//!
//! The key is the identity of "this command in this environment," before any
//! inputs are known.

use serde::Serialize;
use sp_cas::{CanonError, Hash, canonical_json};

/// Schema version of the step key JSON.
pub const VERSION: u32 = 1;

/// The spike environment, exactly as the Schemas section lists it. Nothing
/// from the host is ever inherited.
pub const SPIKE_ENV: &[(&str, &str)] = &[
    ("CGO_ENABLED", "0"),
    ("GOCACHE", "/workspace/.stillpass/gocache"),
    ("GOFLAGS", "-mod=mod"),
    ("GOMODCACHE", "/workspace/.stillpass/gomod"),
    ("GOPROXY", "off"),
    ("GOTOOLCHAIN", "local"),
    ("HOME", "/workspace/.stillpass/home"),
    ("LANG", "C.UTF-8"),
    ("PATH", "/usr/local/go/bin:/usr/bin:/bin"),
    ("TZ", "UTC"),
];

/// Variables whose values change on every run. Putting one in a key would
/// make every run a miss, or, if normalized silently, hide a real input, so
/// they are refused outright.
pub const DENYLIST: &[&str] = &[
    "GITHUB_RUN_ID",
    "GITHUB_RUN_NUMBER",
    "GITHUB_RUN_ATTEMPT",
    "GITHUB_JOB",
    "GITHUB_TOKEN",
    "ACTIONS_RUNTIME_TOKEN",
    "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
    "RUNNER_TEMP",
    "STILLPASS_RUN_ID",
];

/// The working directory and root every spike step sees.
pub const WORKSPACE: &str = "/workspace";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepSpec {
    /// The environment pin: rootfs digest (0b) or host fingerprint (0a).
    pub image: Hash,
    pub cmd: Vec<String>,
    pub cwd: String,
    /// Any order on input; sorted by name in the key.
    pub env: Vec<(String, String)>,
    pub roots: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("duplicate env name {0:?}")]
    DuplicateEnv(String),
    #[error("env name {0:?} is volatile and may not be part of a step key")]
    Denylisted(String),
    #[error("invalid env name {0:?}")]
    BadEnvName(String),
    #[error(transparent)]
    Canon(#[from] CanonError),
}

#[derive(Serialize)]
struct KeyJson<'a> {
    v: u32,
    image: Hash,
    cmd: &'a [String],
    cwd: &'a str,
    env: Vec<[&'a str; 2]>,
    roots: &'a [String],
}

impl StepSpec {
    /// A spike step: `/bin/sh -e -c <script>` in `/workspace` with the
    /// Schemas env plus `extra_env`.
    pub fn spike(image: Hash, script: &str, extra_env: &[(String, String)]) -> StepSpec {
        let mut env: Vec<(String, String)> = SPIKE_ENV
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        env.extend(extra_env.iter().cloned());
        StepSpec {
            image,
            cmd: shell_cmd(script),
            cwd: WORKSPACE.to_string(),
            env,
            roots: vec![WORKSPACE.to_string()],
        }
    }

    /// Env sorted by name, after validation.
    pub fn normalized_env(&self) -> Result<Vec<(String, String)>, KeyError> {
        let mut env = self.env.clone();
        env.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        for w in env.windows(2) {
            if w[0].0 == w[1].0 {
                return Err(KeyError::DuplicateEnv(w[0].0.clone()));
            }
        }
        for (k, _) in &env {
            if k.is_empty() || k.contains('=') || k.contains('\0') {
                return Err(KeyError::BadEnvName(k.clone()));
            }
            if DENYLIST.contains(&k.as_str()) {
                return Err(KeyError::Denylisted(k.clone()));
            }
        }
        Ok(env)
    }
}

/// The command for a `run:` style step: the script text verbatim.
pub fn shell_cmd(script: &str) -> Vec<String> {
    ["/bin/sh", "-e", "-c", script].map(String::from).to_vec()
}

pub fn canonical_key_json(spec: &StepSpec) -> Result<String, KeyError> {
    let env = spec.normalized_env()?;
    let json = KeyJson {
        v: VERSION,
        image: spec.image,
        cmd: &spec.cmd,
        cwd: &spec.cwd,
        env: env.iter().map(|(k, v)| [k.as_str(), v.as_str()]).collect(),
        roots: &spec.roots,
    };
    Ok(canonical_json(&json)?)
}

pub fn step_key(spec: &StepSpec) -> Result<Hash, KeyError> {
    Ok(Hash::of(canonical_key_json(spec)?.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn spec() -> StepSpec {
        StepSpec::spike(Hash::of(b"image"), "go test ./...", &[])
    }

    #[test]
    fn json_shape() {
        let j = canonical_key_json(&spec()).unwrap();
        assert!(j.starts_with(r#"{"cmd":["/bin/sh","-e","-c","go test ./..."],"cwd":"/workspace","env":[["CGO_ENABLED","0"],"#));
        assert!(j.contains(r#"["TZ","UTC"]],"image":"b3:"#));
        assert!(j.ends_with(r#""roots":["/workspace"],"v":1}"#));
    }

    #[test]
    fn duplicate_env_is_an_error() {
        let mut s = spec();
        s.env.push(("TZ".into(), "Local".into()));
        assert!(matches!(step_key(&s), Err(KeyError::DuplicateEnv(n)) if n == "TZ"));
    }

    #[test]
    fn denylisted_env_is_an_error() {
        let s = StepSpec::spike(
            Hash::of(b"i"),
            "true",
            &[("GITHUB_RUN_ID".into(), "1".into())],
        );
        assert!(matches!(step_key(&s), Err(KeyError::Denylisted(_))));
    }

    #[test]
    fn every_field_changes_the_key() {
        let base = step_key(&spec()).unwrap();
        let mut s = spec();
        s.image = Hash::of(b"other");
        assert_ne!(step_key(&s).unwrap(), base);
        let s = StepSpec::spike(Hash::of(b"image"), "go test ./... ", &[]);
        assert_ne!(step_key(&s).unwrap(), base);
        let s = StepSpec::spike(
            Hash::of(b"image"),
            "go test ./...",
            &[("X".into(), "1".into())],
        );
        assert_ne!(step_key(&s).unwrap(), base);
        let mut s = spec();
        s.cwd = "/workspace/sub".into();
        assert_ne!(step_key(&s).unwrap(), base);
        let mut s = spec();
        s.roots.push("/other".into());
        assert_ne!(step_key(&s).unwrap(), base);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn env_order_does_not_matter(seed in any::<u64>()) {
            let mut s = spec();
            let base = step_key(&s).unwrap();
            // Deterministic Fisher-Yates from the seed.
            let mut x = seed | 1;
            for i in (1..s.env.len()).rev() {
                x ^= x << 13; x ^= x >> 7; x ^= x << 17;
                s.env.swap(i, (x % (i as u64 + 1)) as usize);
            }
            prop_assert_eq!(step_key(&s).unwrap(), base);
        }
    }
}
