//! Canonical step key: the exact JSON form, env normalization and denylist,
//! hashing. Pure functions; no filesystem, no storage.
//!
//! The key is the identity of "this command in this environment," before any
//! inputs are known. The environment comes from a toolchain profile
//! (`profile`): Go, Node and Python each define the fixed list of variables
//! a step sees, and the key names the toolchain.

pub mod profile;

use serde::{Deserialize, Serialize};
use sp_cas::{CanonError, Hash, canonical_json};

pub use profile::{Knob, Profile, Seal, Toolchain, Warm, go_source_file};

/// How the environment outside the observed tree was pinned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageKind {
    /// `digest` names a content-addressed root filesystem the step ran in
    /// (`stillpass rootfs import`); nothing of the host's userland is
    /// visible. Portable: any host with the same rootfs and platform
    /// produces the key.
    Rootfs,
    /// `digest` is the BLAKE3 of the host fingerprint
    /// (scripts/host-fingerprint.sh); the step ran on the host's /usr and
    /// /etc. The fallback for hosts without the rootfs; not portable.
    Host,
}

impl std::fmt::Display for ImageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ImageKind::Rootfs => "rootfs",
            ImageKind::Host => "host",
        })
    }
}

/// What the host's kernel and CPU show a step whatever its root filesystem.
/// The named fields are the ones programs choose code paths by, kept
/// readable; `host` is the host class: what is constant about the host and
/// observable by, or in force for, a step (sp_sandbox::host_class_inside,
/// read in a sandbox like the step's; host_class_outside: cgroup limits,
/// binfmt_misc, filesystems, the sandbox's version). An audit re-runs on the
/// same host, so it can never see a dependence on a host constant: those
/// are in the key, exact (no bucket), and a relaxation would have to be an
/// explicit opt-in recorded here. Only what varies run to run is left to
/// audit; the known gaps are listed in sp_sandbox's docs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Platform {
    /// `uname -m`: `x86_64`, `aarch64`.
    pub arch: String,
    /// `uname -r` reduced to `major.minor`, e.g. `6.8` (the full release is
    /// in `host`).
    pub kernel: String,
    /// The CPU features user space dispatches on (aarch64: the auxv HWCAP
    /// words; x86_64: also /proc/cpuinfo's flags), as the sandbox reports
    /// them.
    pub cpu_features: String,
    /// CPUs the step may run on (its affinity mask): the default
    /// parallelism of Go (GOMAXPROCS, `go test -p`) and of most build tools.
    pub cpus: u32,
    /// The kernel's page size (AT_PAGESZ): 4 KiB, 16 KiB or 64 KiB on arm64.
    pub page_size: u64,
    /// The host class, `[name, value]` pairs, sorted by name in the key;
    /// a repeated name is an error.
    pub host: Vec<(String, String)>,
}

impl Platform {
    /// `kernel_release` is the full `uname -r`; only major.minor is kept.
    pub fn new(
        arch: &str,
        kernel_release: &str,
        cpu_features: &str,
        cpus: u32,
        page_size: u64,
        host: Vec<(String, String)>,
    ) -> Platform {
        Platform {
            arch: arch.to_string(),
            kernel: kernel_major_minor(kernel_release),
            cpu_features: cpu_features.to_string(),
            cpus,
            page_size,
            host,
        }
    }
}

/// The environment pin: everything a step can read that is not in the
/// observed tree. The digest covers the userland (a rootfs, or the host's
/// fingerprint), `mounts` the read-only trees bound into it, `platform` what
/// the kernel and CPU expose.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Image {
    pub kind: ImageKind,
    pub digest: Hash,
    /// Read-only trees bound into the root, `[destination, tree digest]`:
    /// pinned by content like the rootfs, never observed. Sorted by
    /// destination in the key; a destination at or under an observed root
    /// is an error. Empty for the CLI.
    pub mounts: Vec<(String, Hash)>,
    pub platform: Platform,
}

impl Image {
    pub fn new(kind: ImageKind, digest: Hash, platform: Platform) -> Image {
        Image {
            kind,
            digest,
            mounts: Vec::new(),
            platform,
        }
    }

    /// For tests and goldens: a rootfs pin on a fixed aarch64 platform.
    pub fn rootfs(digest: Hash) -> Image {
        Image::new(ImageKind::Rootfs, digest, Platform::test_aarch64())
    }

    /// BLAKE3 of the image's canonical JSON (mounts and host class sorted):
    /// names host-side state that belongs to exactly one environment, such
    /// as the warm build cache.
    pub fn id(&self) -> Result<Hash, KeyError> {
        Ok(Hash::of(canonical_json(&self.normalized(&[])?)?.as_bytes()))
    }

    /// The image as the key spells it: mounts validated against `roots` and
    /// sorted, the host class sorted and unique.
    fn normalized(&self, roots: &[String]) -> Result<Image, KeyError> {
        let mut host = self.platform.host.clone();
        host.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        for w in host.windows(2) {
            if w[0].0 == w[1].0 {
                return Err(KeyError::DuplicateHost(w[0].0.clone()));
            }
        }
        Ok(Image {
            mounts: normalized_mounts(&self.mounts, roots)?,
            platform: Platform {
                host,
                ..self.platform.clone()
            },
            ..self.clone()
        })
    }
}

impl Platform {
    /// A fixed aarch64 platform for tests and goldens (kernel 6.8, the
    /// features of a Neoverse N1, 4 CPUs, 4 KiB pages, a small host class).
    pub fn test_aarch64() -> Platform {
        let host = [
            ("kernel release", "6.8.0-1021-azure"),
            (
                "kernel version",
                "#25-Ubuntu SMP Wed Jan 15 20:45:09 UTC 2025",
            ),
            ("memory GiB", "15"),
            ("rlimit NOFILE", "1024 65536"),
            ("sandbox", "bubblewrap 0.9.0"),
            ("sysctl kernel.yama.ptrace_scope", "1"),
        ];
        Platform::new(
            "aarch64",
            "6.8.0-1021-azure",
            "fp asimd evtstrm aes pmull sha1 sha2 crc32 atomics fphp asimdhp cpuid asimdrdm lrcpc dcpop asimddp ssbs; hwcap=0x8fff hwcap2=0x0",
            4,
            4096,
            host.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }
}

/// `6.8.0-1021-azure` -> `6.8`; `7.0.14-linuxkit` -> `7.0`. Anything that
/// is not `digits.digits...` is kept whole rather than guessed at.
pub fn kernel_major_minor(release: &str) -> String {
    let mut parts = release.split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor))
            if !major.is_empty() && major.bytes().all(|b| b.is_ascii_digit()) =>
        {
            let minor: String = minor.chars().take_while(|c| c.is_ascii_digit()).collect();
            if minor.is_empty() {
                release.to_string()
            } else {
                format!("{major}.{minor}")
            }
        }
        _ => release.to_string(),
    }
}

/// Mounts sorted by destination, after validation: absolute, normalized
/// (no `.`, `..`, empty components or trailing slash), not `/`, unique, and
/// neither at nor under nor above one of `roots` (a pinned mount must not
/// hide observed content, nor an observed root hide a pinned mount).
fn normalized_mounts(
    mounts: &[(String, Hash)],
    roots: &[String],
) -> Result<Vec<(String, Hash)>, KeyError> {
    let mut m = mounts.to_vec();
    m.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    for w in m.windows(2) {
        if w[0].0 == w[1].0 {
            return Err(KeyError::BadMount(w[0].0.clone(), "duplicate destination"));
        }
    }
    let within = |a: &str, b: &str| a == b || a.starts_with(&format!("{b}/"));
    for (dst, _) in &m {
        let normal = dst.starts_with('/')
            && dst.len() > 1
            && dst[1..]
                .split('/')
                .all(|c| !c.is_empty() && c != "." && c != "..");
        if !normal {
            return Err(KeyError::BadMount(
                dst.clone(),
                "not an absolute, normalized path below /",
            ));
        }
        if roots.iter().any(|r| within(dst, r) || within(r, dst)) {
            return Err(KeyError::BadMount(dst.clone(), "overlaps an observed root"));
        }
    }
    Ok(m)
}

/// Schema version of the step key JSON. v2 added `warm`: a warm-state step is
/// a different key by construction, not only through the env it changes. v3
/// made `image` an object: what pinned the step's root (`kind`, `digest`),
/// the read-only trees bound into it (`mounts`), and what the host shows a
/// step that no root filesystem can pin (`platform`, with the host class
/// `platform.host`); and the step's process state became fixed (umask,
/// nice, rlimits, signals, descriptors). v4 added `toolchain`: which
/// profile's env and rules a step ran under.
pub const VERSION: u32 = 4;

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
    /// The environment pin: rootfs digest or host fingerprint, pinned
    /// mounts, and the platform.
    pub image: Image,
    pub toolchain: Toolchain,
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
    #[error("mount {0:?}: {1}")]
    BadMount(String, &'static str),
    #[error("host class names {0:?} twice")]
    DuplicateHost(String),
    #[error("warm state needs {name}={want}, but the step sets {name}={have:?}")]
    WarmConflict {
        name: String,
        have: String,
        want: &'static str,
    },
    #[error("the {0} profile has no warm state")]
    NoWarm(Toolchain),
    #[error(transparent)]
    Canon(#[from] CanonError),
}

#[derive(Serialize)]
struct KeyJson<'a> {
    v: u32,
    image: Image,
    toolchain: Toolchain,
    cmd: &'a [String],
    cwd: &'a str,
    env: Vec<[&'a str; 2]>,
    roots: &'a [String],
    warm: bool,
}

impl StepSpec {
    /// A step: `/bin/sh -e -c <script>` in `/workspace` with the toolchain
    /// profile's env plus `extra_env`.
    pub fn new(
        toolchain: Toolchain,
        image: Image,
        script: &str,
        extra_env: &[(String, String)],
    ) -> StepSpec {
        let mut env = toolchain.profile().env_vec();
        env.extend(extra_env.iter().cloned());
        StepSpec {
            image,
            toolchain,
            cmd: shell_cmd(script),
            cwd: WORKSPACE.to_string(),
            env,
            roots: vec![WORKSPACE.to_string()],
            warm: false,
        }
    }

    /// The step started warm: `new`, marked warm in the key, with the
    /// profile's warm knobs merged into its env. Refused for a toolchain
    /// whose profile defines no warm state.
    pub fn new_warm(
        toolchain: Toolchain,
        image: Image,
        script: &str,
        extra_env: &[(String, String)],
    ) -> Result<StepSpec, KeyError> {
        let warm = toolchain
            .profile()
            .warm
            .as_ref()
            .ok_or(KeyError::NoWarm(toolchain))?;
        let mut spec = StepSpec::new(toolchain, image, script, extra_env);
        spec.warm = true;
        for k in warm.knobs {
            merge_knob(&mut spec.env, k)?;
        }
        Ok(spec)
    }

    pub fn profile(&self) -> &'static Profile {
        self.toolchain.profile()
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

/// Adds the knob's `item` (a `knob=value` setting) to the `sep`-separated
/// variable `name`: appended to an existing value, set if absent, left alone
/// if already there, refused if the variable sets the same knob to something
/// else. Only the first entry named `name` is touched; a duplicate is the
/// key's error to report.
fn merge_knob(env: &mut Vec<(String, String)>, k: &Knob) -> Result<(), KeyError> {
    let (name, sep, item) = (k.name, k.sep, k.item);
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
    let image = spec.image.normalized(&spec.roots)?;
    let json = KeyJson {
        v: VERSION,
        image,
        toolchain: spec.toolchain,
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

    fn img() -> Image {
        Image::rootfs(Hash::of(b"image"))
    }

    fn spec() -> StepSpec {
        StepSpec::new(Toolchain::Go, img(), "go test ./...", &[])
    }

    #[test]
    fn json_shape() {
        let j = canonical_key_json(&spec()).unwrap();
        assert!(j.starts_with(r#"{"cmd":["/bin/sh","-e","-c","go test ./..."],"cwd":"/workspace","env":[["CGO_ENABLED","0"],"#));
        assert!(
            j.contains(&format!(
                r#"["TZ","UTC"]],"image":{{"digest":"{}","kind":"rootfs","mounts":[],"platform":{{"arch":"aarch64","cpu_features":"fp asimd "#,
                Hash::of(b"image")
            )),
            "{j}"
        );
        assert!(
            j.contains(r#"hwcap2=0x0","cpus":4,"host":[["kernel release","6.8.0-1021-azure"],["kernel version","#),
            "{j}"
        );
        assert!(
            j.contains(r#"["sysctl kernel.yama.ptrace_scope","1"]],"kernel":"6.8","page_size":4096}},"roots""#),
            "{j}"
        );
        assert!(j.ends_with(r#""roots":["/workspace"],"toolchain":"go","v":4,"warm":false}"#));
        let mut host = StepSpec::new(
            Toolchain::Go,
            Image::new(
                ImageKind::Host,
                Hash::of(b"fp"),
                Platform::new(
                    "x86_64",
                    "7.0.14-linuxkit",
                    "fpu sse2",
                    18,
                    4096,
                    vec![("z".into(), "1".into()), ("a".into(), "2".into())],
                ),
            ),
            "true",
            &[],
        );
        host.image.mounts = vec![
            ("/opt/z".into(), Hash::of(b"z")),
            ("/opt/a".into(), Hash::of(b"a")),
        ];
        let j = canonical_key_json(&host).unwrap();
        assert!(
            j.contains(&format!(
                r#""image":{{"digest":"{}","kind":"host","mounts":[["/opt/a","{}"],["/opt/z","{}"]],"platform":{{"arch":"x86_64","cpu_features":"fpu sse2","cpus":18,"host":[["a","2"],["z","1"]],"kernel":"7.0","page_size":4096}}}}"#,
                Hash::of(b"fp"),
                Hash::of(b"a"),
                Hash::of(b"z")
            )),
            "mounts and host class sorted by name: {j}"
        );
    }

    #[test]
    fn kernel_release_reduces_to_major_minor() {
        assert_eq!(kernel_major_minor("6.8.0-1021-azure"), "6.8");
        assert_eq!(kernel_major_minor("7.0.14-linuxkit"), "7.0");
        assert_eq!(kernel_major_minor("5.15.0"), "5.15");
        assert_eq!(kernel_major_minor("6.12"), "6.12");
        // Not a dotted release: kept whole, never guessed at.
        assert_eq!(kernel_major_minor("weird"), "weird");
        assert_eq!(kernel_major_minor("6.x"), "6.x");
        assert_eq!(kernel_major_minor(""), "");
    }

    #[test]
    fn the_image_pins_kind_digest_mounts_and_platform() {
        let base = step_key(&spec()).unwrap();
        let changed = |f: &dyn Fn(&mut Image)| {
            let mut s = spec();
            f(&mut s.image);
            step_key(&s).unwrap()
        };
        assert_ne!(
            changed(&|i| i.kind = ImageKind::Host),
            base,
            "same digest, other kind"
        );
        assert_ne!(changed(&|i| i.digest = Hash::of(b"other")), base);
        assert_ne!(changed(&|i| i.platform.arch = "x86_64".into()), base);
        assert_ne!(changed(&|i| i.platform.kernel = "6.11".into()), base);
        assert_ne!(changed(&|i| i.platform.cpu_features.push_str(" sve")), base);
        assert_ne!(changed(&|i| i.platform.cpus = 2), base);
        assert_ne!(changed(&|i| i.platform.page_size = 16384), base);
        // Every host-class value is pinned, and so is its presence.
        for (n, (name, _)) in img().platform.host.iter().enumerate() {
            assert_ne!(changed(&|i| i.platform.host[n].1.push('x')), base, "{name}");
            assert_ne!(
                changed(&|i| {
                    i.platform.host.remove(n);
                }),
                base,
                "{name}"
            );
        }
        assert_ne!(
            changed(&|i| i.platform.host.push(("new".into(), "1".into()))),
            base
        );
        // Order does not matter; a repeated name is an error.
        assert_eq!(changed(&|i| i.platform.host.reverse()), base);
        let mut s = spec();
        let first = s.image.platform.host[0].clone();
        s.image.platform.host.push(first);
        assert!(matches!(step_key(&s), Err(KeyError::DuplicateHost(_))));
        let m = |i: &mut Image| i.mounts = vec![("/opt/t".into(), Hash::of(b"t"))];
        let with_mount = changed(&m);
        assert_ne!(with_mount, base);
        assert_ne!(
            changed(&|i| i.mounts = vec![("/opt/t".into(), Hash::of(b"t2"))]),
            with_mount,
            "the mount's content is pinned"
        );
        assert_ne!(
            changed(&|i| i.mounts = vec![("/opt/u".into(), Hash::of(b"t"))]),
            with_mount,
            "and where it is"
        );
        // Patch level and build are not part of the pin.
        let p = |release: &str| Platform::new("aarch64", release, "fp", 4, 4096, vec![]);
        assert_eq!(p("6.8.0-1021-azure"), p("6.8.12-generic"));
        assert_ne!(p("6.8.0-1021-azure"), p("6.9.0"));
        // The image id follows the key's notion of equality.
        let mut a = img();
        a.mounts = vec![("/b".into(), Hash::of(b"b")), ("/a".into(), Hash::of(b"a"))];
        let mut b = a.clone();
        b.mounts.reverse();
        assert_eq!(a.id().unwrap(), b.id().unwrap());
        assert_ne!(a.id().unwrap(), img().id().unwrap());
    }

    #[test]
    fn bad_mounts_are_errors() {
        let bad = |dst: &str| {
            let mut s = spec();
            s.image.mounts = vec![(dst.into(), Hash::of(b"m"))];
            step_key(&s)
        };
        for dst in [
            "",
            "/",
            "opt",
            "/opt/",
            "/opt//x",
            "/opt/./x",
            "/opt/../x",
            "/workspace",
            "/workspace/sub",
        ] {
            assert!(
                matches!(bad(dst), Err(KeyError::BadMount(..))),
                "{dst:?} accepted"
            );
        }
        // A mount above the observed root would hide it.
        let mut s = spec();
        s.roots = vec!["/w/repo".into()];
        s.image.mounts = vec![("/w".into(), Hash::of(b"m"))];
        assert!(matches!(step_key(&s), Err(KeyError::BadMount(..))));
        assert!(bad("/workspacex").is_ok() && bad("/opt/x").is_ok());
        let mut s = spec();
        s.image.mounts = vec![
            ("/opt/x".into(), Hash::of(b"1")),
            ("/opt/x".into(), Hash::of(b"2")),
        ];
        assert!(matches!(step_key(&s), Err(KeyError::BadMount(..))));
        let n = StepSpec::new(Toolchain::Node, img(), "npm test", &[]);
        let j = canonical_key_json(&n).unwrap();
        assert!(
            j.contains(r#"["CI","true"]"#) && j.contains(r#""toolchain":"node""#),
            "{j}"
        );
    }

    #[test]
    fn toolchain_is_in_the_key() {
        // The same script and image under another profile is another key,
        // even with an identical env list.
        let mut a = spec();
        let mut b = spec();
        b.toolchain = Toolchain::Python;
        a.env.clear();
        b.env.clear();
        assert_ne!(step_key(&a).unwrap(), step_key(&b).unwrap());
    }

    #[test]
    fn warm_is_marked_in_the_key_and_merges_its_knobs() {
        let cold = spec();
        let warm = StepSpec::new_warm(Toolchain::Go, img(), "go test ./...", &[]).unwrap();
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
        let merged = StepSpec::new_warm(
            Toolchain::Go,
            img(),
            "true",
            &x(&[("GODEBUG", "http2debug=1")]),
        )
        .unwrap();
        assert_eq!(
            get(&merged, "GODEBUG").as_deref(),
            Some("http2debug=1,goindex=0")
        );
        let same = StepSpec::new_warm(
            Toolchain::Go,
            img(),
            "true",
            &x(&[("GODEBUG", "goindex=0")]),
        )
        .unwrap();
        assert_eq!(get(&same, "GODEBUG").as_deref(), Some("goindex=0"));
        // Asking for the opposite is refused, never silently overridden.
        let conflict = StepSpec::new_warm(
            Toolchain::Go,
            img(),
            "true",
            &x(&[("GODEBUG", "goindex=1")]),
        );
        assert!(
            matches!(conflict, Err(KeyError::WarmConflict { ref name, .. }) if name == "GODEBUG"),
            "{conflict:?}"
        );
        // Profiles without warm state refuse it.
        for t in [Toolchain::Node, Toolchain::Python] {
            assert!(matches!(
                StepSpec::new_warm(t, img(), "true", &[]),
                Err(KeyError::NoWarm(x)) if x == t
            ));
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
        let s = StepSpec::new(
            Toolchain::Go,
            img(),
            "true",
            &[("GITHUB_RUN_ID".into(), "1".into())],
        );
        assert!(matches!(step_key(&s), Err(KeyError::Denylisted(_))));
    }

    #[test]
    fn every_field_changes_the_key() {
        let base = step_key(&spec()).unwrap();
        let mut s = spec();
        s.warm = true;
        assert_ne!(step_key(&s).unwrap(), base);
        let mut s = spec();
        s.toolchain = Toolchain::Node;
        assert_ne!(step_key(&s).unwrap(), base);
        let s = StepSpec::new(Toolchain::Go, img(), "go test ./... ", &[]);
        assert_ne!(step_key(&s).unwrap(), base);
        let s = StepSpec::new(
            Toolchain::Go,
            img(),
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
