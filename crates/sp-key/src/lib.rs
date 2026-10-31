//! Canonical step key: the exact JSON form, env normalization and denylist,
//! hashing. Pure functions; no filesystem, no storage.
//!
//! The key is the identity of "this command in this environment," before any
//! inputs are known.

use serde::Serialize;
use sp_cas::{CanonError, Hash, canonical_json};

/// Schema version of the step key JSON. v2 added `warm`: a warm-state step is
/// a different key by construction, not only through the env it changes.
pub const VERSION: u32 = 2;

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

/// What a warm-state step adds to its env. Both turn off the parts of Go's
/// cache that are not keyed by content, leaving the build cache (keyed by
/// content hashes) as the warm state. Merged into whatever GOFLAGS and
/// GODEBUG the step already has; a step that sets the same knob to another
/// value is refused, never silently overridden. Part of the key like every
/// variable the step sees.
///
/// `-count=1` turns off Go's test-result cache: it decides whether a test's
/// inputs changed from their size and mtime, and the observer serves every
/// file with the same constant mtime, so with a warm cache it could replay a
/// stale result for a changed fixture of the same size.
///
/// `goindex=0` turns off Go's module index: the file lists and build
/// constraints of every module in GOMODCACHE, stored in GOCACHE keyed by the
/// module's path on the assumption that the module cache never changes. A
/// warm run using it never reads those directories, so the observer would
/// record far fewer inputs than a cold run does (7k against 21k on cli/cli).
pub const WARM_GOFLAG: &str = "-count=1";
pub const WARM_GODEBUG: &str = "goindex=0";

/// Is `name` a file the Go toolchain treats as source (go/build's extension
/// set)? Warm runs over-record every such file in every directory the step
/// listed: go test caches each package's vet verdict keyed by the files it
/// compiled, but vet's buildtag check also parses the package's ignored
/// files (excluded by a GOOS/GOARCH suffix, so nothing else ever opens
/// them), and a cached verdict is reused whatever they now contain. With an
/// empty cache the observer sees vet read them; with a full one it cannot.
pub fn go_source_file(name: &str) -> bool {
    const EXTS: &[&str] = &[
        ".go", ".c", ".cc", ".cpp", ".cxx", ".m", ".s", ".S", ".sx", ".swig", ".swigcxx", ".syso",
        ".h", ".hh", ".hpp", ".hxx", ".f", ".F", ".for", ".f90",
    ];
    EXTS.iter().any(|e| name.ends_with(e))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepSpec {
    /// The environment pin: rootfs digest (0b) or host fingerprint (0a).
    pub image: Hash,
    pub cmd: Vec<String>,
    pub cwd: String,
    /// Any order on input; sorted by name in the key.
    pub env: Vec<(String, String)>,
    pub roots: Vec<String>,
    /// Started with warm state (a build cache from earlier warm runs).
    pub warm: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("duplicate env name {0:?}")]
    DuplicateEnv(String),
    #[error("env name {0:?} is volatile and may not be part of a step key")]
    Denylisted(String),
    #[error("invalid env name {0:?}")]
    BadEnvName(String),
    #[error("warm state needs {name}={want}, but the step sets {name}={have:?}")]
    WarmConflict {
        name: String,
        have: String,
        want: &'static str,
    },
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
    warm: bool,
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
            warm: false,
        }
    }

    /// The spike step started warm: `spike`, marked warm in the key, with
    /// `WARM_GOFLAG` and `WARM_GODEBUG` merged into its env.
    pub fn spike_warm(
        image: Hash,
        script: &str,
        extra_env: &[(String, String)],
    ) -> Result<StepSpec, KeyError> {
        let mut spec = StepSpec::spike(image, script, extra_env);
        spec.warm = true;
        merge_knob(&mut spec.env, "GOFLAGS", ' ', WARM_GOFLAG)?;
        merge_knob(&mut spec.env, "GODEBUG", ',', WARM_GODEBUG)?;
        Ok(spec)
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

/// Adds `item` (a `knob=value` setting) to the `sep`-separated variable
/// `name`: appended to an existing value, set if absent, left alone if
/// already there, refused if the variable sets the same knob to something
/// else. Only the first entry named `name` is touched; a duplicate is the
/// key's error to report.
fn merge_knob(
    env: &mut Vec<(String, String)>,
    name: &str,
    sep: char,
    item: &'static str,
) -> Result<(), KeyError> {
    let knob = &item[..=item.find('=').expect("knob=value")];
    match env.iter_mut().find(|(k, _)| k == name) {
        None => env.push((name.to_string(), item.to_string())),
        Some((_, v)) => {
            let mine = v.split(sep).filter(|s| s.starts_with(knob));
            let other: Vec<&str> = mine.filter(|s| *s != item).collect();
            if !other.is_empty() {
                return Err(KeyError::WarmConflict {
                    name: name.to_string(),
                    have: v.clone(),
                    want: item,
                });
            }
            if !v.split(sep).any(|s| s == item) {
                if !v.is_empty() {
                    v.push(sep);
                }
                v.push_str(item);
            }
        }
    }
    Ok(())
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
        warm: spec.warm,
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
        assert!(j.ends_with(r#""roots":["/workspace"],"v":2,"warm":false}"#));
    }

    #[test]
    fn warm_is_marked_in_the_key_and_merges_its_knobs() {
        let cold = spec();
        let warm = StepSpec::spike_warm(Hash::of(b"image"), "go test ./...", &[]).unwrap();
        assert!(warm.warm && !cold.warm);
        assert_ne!(step_key(&warm).unwrap(), step_key(&cold).unwrap());
        assert!(
            canonical_key_json(&warm)
                .unwrap()
                .ends_with(r#""warm":true}"#)
        );
        let get =
            |s: &StepSpec, k: &str| s.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get(&warm, "GOFLAGS").as_deref(), Some("-mod=mod -count=1"));
        assert_eq!(get(&warm, "GODEBUG").as_deref(), Some("goindex=0"));
        assert_eq!(warm.env.len(), cold.env.len() + 1);
        assert!(warm.normalized_env().is_ok());

        // The marker alone separates the keys: same env, different key.
        let mut unmarked = warm.clone();
        unmarked.warm = false;
        assert_ne!(step_key(&unmarked).unwrap(), step_key(&warm).unwrap());

        // A step's own GODEBUG is kept and extended, not replaced.
        let x = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<Vec<_>>()
        };
        let merged =
            StepSpec::spike_warm(Hash::of(b"i"), "true", &x(&[("GODEBUG", "http2debug=1")]))
                .unwrap();
        assert_eq!(
            get(&merged, "GODEBUG").as_deref(),
            Some("http2debug=1,goindex=0")
        );
        let same =
            StepSpec::spike_warm(Hash::of(b"i"), "true", &x(&[("GODEBUG", "goindex=0")])).unwrap();
        assert_eq!(get(&same, "GODEBUG").as_deref(), Some("goindex=0"));
        // Asking for the opposite is refused, never silently overridden.
        let conflict =
            StepSpec::spike_warm(Hash::of(b"i"), "true", &x(&[("GODEBUG", "goindex=1")]));
        assert!(
            matches!(conflict, Err(KeyError::WarmConflict { ref name, .. }) if name == "GODEBUG"),
            "{conflict:?}"
        );
    }

    #[test]
    fn go_source_files() {
        for yes in [
            "a.go",
            "a_windows.go",
            "x_test.go",
            "asm_amd64.s",
            "t.h",
            "lib.syso",
        ] {
            assert!(go_source_file(yes), "{yes}");
        }
        for no in [
            "README.md",
            "go.mod",
            "go.sum",
            "a.txt",
            "Makefile",
            "gofile",
            ".go.swp",
        ] {
            assert!(!go_source_file(no), "{no}");
        }
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
        let mut s = spec();
        s.warm = true;
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
