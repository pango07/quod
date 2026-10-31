//! Toolchain profiles: everything Stillpass knows about one language's
//! toolchain, in one place. Go, Node and Python are three values of the same
//! shape; generic code (the key, the observer, sealing, warm state, the
//! acceptance suite, the bench) consults the profile and knows no language.
//!
//! A profile carries: the step environment (the fixed list the key records;
//! nothing from the host is ever inherited), the default dependency-install
//! step, the sealed prefixes (immutable dependency trees served from a
//! snapshot and recorded whole), warm state and the cache-disabling knobs
//! that keep it sound, over-record rules, and what the acceptance suite
//! needs to pick files in that language. A split driver (one process per
//! test file, exp/split) is the one slot left empty here.

use serde::{Deserialize, Serialize};
use sp_cas::{Kind, Tree};

use crate::WORKSPACE;

/// The toolchains Stillpass can run a step for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Toolchain {
    Go,
    Node,
    Python,
}

impl Toolchain {
    pub const ALL: &[Toolchain] = &[Toolchain::Go, Toolchain::Node, Toolchain::Python];

    pub fn name(self) -> &'static str {
        match self {
            Toolchain::Go => "go",
            Toolchain::Node => "node",
            Toolchain::Python => "python",
        }
    }

    pub fn profile(self) -> &'static Profile {
        match self {
            Toolchain::Go => &GO,
            Toolchain::Node => &NODE,
            Toolchain::Python => &PYTHON,
        }
    }

    /// Which toolchain a repo is for, from the names at its top level. One
    /// marker file set per toolchain; a repo with markers for several needs
    /// an explicit choice, since guessing wrong would run the wrong env.
    pub fn detect<'a>(top_level: impl IntoIterator<Item = &'a str>) -> Result<Toolchain, Detect> {
        let names: Vec<&str> = top_level.into_iter().collect();
        let found: Vec<Toolchain> = Toolchain::ALL
            .iter()
            .copied()
            .filter(|t| t.profile().markers.iter().any(|m| names.contains(m)))
            .collect();
        match found.as_slice() {
            [one] => Ok(*one),
            [] => Err(Detect::None),
            many => Err(Detect::Ambiguous(many.to_vec())),
        }
    }
}

impl std::str::FromStr for Toolchain {
    type Err = String;

    fn from_str(s: &str) -> Result<Toolchain, String> {
        Toolchain::ALL
            .iter()
            .copied()
            .find(|t| t.name() == s)
            .ok_or_else(|| format!("unknown toolchain {s:?} (go, node or python)"))
    }
}

impl std::fmt::Display for Toolchain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Detect {
    #[error(
        "no toolchain marker at the top level (go.mod, package.json, pyproject.toml, ...); pass --toolchain"
    )]
    None,
    #[error("markers for several toolchains at the top level ({}); pass --toolchain", .0.iter().map(|t| t.name()).collect::<Vec<_>>().join(", "))]
    Ambiguous(Vec<Toolchain>),
}

/// A subtree the step reads from an immutable snapshot instead of the
/// observer (a dependency tree), recorded whole. `carve_outs` are
/// directories under it that stay on the observed overlay because the tool
/// writes there; absent from the tree, they are created empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seal {
    /// Workspace-relative path of the subtree.
    pub prefix: String,
    /// Workspace-relative paths under `prefix`.
    pub carve_outs: Vec<String>,
}

/// One `knob=value` setting merged into a `sep`-separated variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Knob {
    pub name: &'static str,
    pub sep: char,
    pub item: &'static str,
}

/// Warm state: a cache directory the tool keeps between runs, bound from
/// persistent host state, unobserved. Sound only when the cache is keyed
/// by content; `knobs` turn off the parts that are not, and `over_record`
/// names the files a cached verdict can stand in for (recorded in every
/// directory the step listed).
#[derive(Clone, Copy, Debug)]
pub struct Warm {
    /// The env variable naming the cache directory (inside the workspace).
    pub cache_env: &'static str,
    pub knobs: &'static [Knob],
    pub over_record: fn(&str) -> bool,
}

/// How a sealed prefix is found in a tree.
#[derive(Clone, Copy, Debug)]
enum SealRule {
    /// The directory the env variable points at (under the workspace), with
    /// these carve-outs (relative to it).
    EnvDir {
        env: &'static str,
        carve_outs: &'static [&'static str],
    },
    /// Every directory of this name not nested under another of the same
    /// name (a monorepo has one per package), with these carve-outs.
    Named {
        name: &'static str,
        carve_outs: &'static [&'static str],
    },
}

pub struct Profile {
    pub toolchain: Toolchain,
    /// Top-level files that mark a repo as this toolchain's.
    pub markers: &'static [&'static str],
    /// The step environment, complete. Sorted into the key; nothing from
    /// the host is ever inherited.
    pub env: &'static [(&'static str, &'static str)],
    /// Default step A: install dependencies. Network, never memoized.
    pub setup: &'static str,
    /// Default step B: the test suite. Observed, memoized.
    pub test: &'static str,
    /// Env overrides for a step with network (step A): what the tool needs
    /// to fetch.
    pub net_env: &'static [(&'static str, &'static str)],
    /// Workspace-relative directories bound from persistent host state
    /// during step A only (download caches), so they never enter a tree.
    pub setup_cache: &'static [&'static str],
    /// Workspace-relative directories step A leaves behind that are not
    /// step B's inputs (a build cache, per-day files under HOME); removed
    /// before step B so step B starts from the dependency tree alone.
    pub ephemeral: &'static [&'static str],
    /// Dependency-tree directories worth seeding a fresh clone with, so
    /// step A only fetches what a commit adds.
    pub seed_dirs: &'static [&'static str],
    seals: &'static [SealRule],
    pub warm: Option<Warm>,
    /// Directory names a correctly configured step never writes under: a
    /// cache the env is meant to keep off (Python's `__pycache__`). The
    /// acceptance suite checks a recorded result's outputs against this.
    pub no_outputs_under: &'static [&'static str],
    /// Source file extensions of the language (acceptance picks one to edit).
    pub source_exts: &'static [&'static str],
    /// A line comment, appended to a source file to change it harmlessly.
    pub comment: &'static str,
}

impl Profile {
    pub fn env_vec(&self) -> Vec<(String, String)> {
        self.env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The step env's value of `name`.
    pub fn env_value(&self, name: &str) -> Option<&'static str> {
        self.env.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }

    /// Workspace-relative path of the env variable's directory, if it
    /// points under the workspace.
    pub fn env_dir(&self, name: &str) -> Option<String> {
        workspace_relative(self.env_value(name)?)
    }

    /// The sealed subtrees this tree has. Only prefixes present as
    /// directories are sealed; a carve-out need not exist.
    pub fn seals(&self, tree: &Tree) -> Vec<Seal> {
        let has_dir = |p: &str| tree.get(p).is_some_and(|e| e.k == Kind::Dir);
        let mut out = Vec::new();
        for rule in self.seals {
            match *rule {
                SealRule::EnvDir { env, carve_outs } => {
                    if let Some(prefix) = self.env_dir(env)
                        && has_dir(&prefix)
                    {
                        out.push(Seal {
                            carve_outs: carve_outs
                                .iter()
                                .map(|c| format!("{prefix}/{c}"))
                                .collect(),
                            prefix,
                        });
                    }
                }
                SealRule::Named { name, carve_outs } => {
                    let suffix = format!("/{name}");
                    let nested = format!("/{name}/");
                    for e in tree.iter().filter(|e| e.k == Kind::Dir) {
                        let named = e.p == name || e.p.ends_with(&suffix);
                        let under_another =
                            e.p.starts_with(&format!("{name}/")) || e.p.contains(&nested);
                        if named && !under_another {
                            out.push(Seal {
                                carve_outs: carve_outs
                                    .iter()
                                    .map(|c| format!("{}/{c}", e.p))
                                    .collect(),
                                prefix: e.p.clone(),
                            });
                        }
                    }
                }
            }
        }
        out
    }

    /// Is `path` (workspace-relative) a source file of this language that
    /// the acceptance suite may edit: right extension, not a test file, not
    /// under a dependency tree or Stillpass's own directory.
    pub fn is_editable_source(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        if !self.source_exts.iter().any(|e| name.ends_with(e)) || path.starts_with(".stillpass/") {
            return false;
        }
        let parts: Vec<&str> = path.split('/').collect();
        if parts.iter().any(|p| {
            matches!(
                *p,
                "node_modules" | "__pycache__" | "__tests__" | "testdata" | "vendor"
            )
        }) {
            return false;
        }
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        let is_test = match self.toolchain {
            Toolchain::Go => stem.ends_with("_test"),
            Toolchain::Node => {
                stem.ends_with(".test")
                    || stem.ends_with(".spec")
                    || stem.ends_with("-test")
                    || parts
                        .iter()
                        .any(|p| matches!(*p, "test" | "tests" | "spec"))
            }
            Toolchain::Python => {
                stem.starts_with("test_")
                    || stem.ends_with("_test")
                    || name == "conftest.py"
                    || parts.iter().any(|p| matches!(*p, "test" | "tests"))
            }
        };
        !is_test
    }
}

/// `/workspace/a/b` -> `a/b`; anything outside the workspace is `None`.
pub fn workspace_relative(abs: &str) -> Option<String> {
    let rel = abs
        .strip_prefix(WORKSPACE)?
        .strip_prefix('/')?
        .trim_end_matches('/');
    (!rel.is_empty()).then(|| rel.to_string())
}

/// Where the toolchains live on the host, for the fingerprint and the step
/// PATHs below. The sandbox binds /usr read-only; the fingerprint hashes
/// the binaries.
pub const GO_BIN: &str = "/usr/local/go/bin";
pub const NODE_BIN: &str = "/usr/local/node/bin";
pub const PYTHON_BIN: &str = "/usr/local/python/bin";

/// PATH the host fingerprint runs with: every toolchain dir, so it resolves
/// the same binaries the steps do.
pub const HOST_PATH: &str =
    "/usr/local/go/bin:/usr/local/node/bin:/usr/local/python/bin:/usr/bin:/bin";

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

/// Go, exactly as the handoff's Schemas section lists the spike env.
///
/// Warm state is GOCACHE (content-addressed: action id = hash of source
/// contents, tool ids, flags, dependency outputs). `-count=1` turns off the
/// test-result cache, which decides "inputs unchanged" from size and mtime
/// while the observer serves one constant mtime. `goindex=0` turns off the
/// module index, which caches file lists of every module keyed by path on
/// the assumption that the module cache never changes, so a warm run using
/// it never reads those directories (7k inputs against 21k on cli/cli).
pub static GO: Profile = Profile {
    toolchain: Toolchain::Go,
    markers: &["go.mod"],
    env: &[
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
    ],
    setup: "go mod download",
    test: "go test ./...",
    net_env: &[("GOPROXY", "https://proxy.golang.org,direct")],
    setup_cache: &[],
    ephemeral: &[".stillpass/gocache", ".stillpass/home"],
    seed_dirs: &[".stillpass/gomod"],
    // GOMODCACHE's module@version directories never change once extracted;
    // go writes under cache/ (downloads, locks), which stays observed.
    seals: &[SealRule::EnvDir {
        env: "GOMODCACHE",
        carve_outs: &["cache"],
    }],
    warm: Some(Warm {
        cache_env: "GOCACHE",
        knobs: &[
            Knob {
                name: "GOFLAGS",
                sep: ' ',
                item: "-count=1",
            },
            Knob {
                name: "GODEBUG",
                sep: ',',
                item: "goindex=0",
            },
        ],
        over_record: go_source_file,
    }),
    no_outputs_under: &[],
    source_exts: &[".go"],
    comment: "//",
};

/// Node. `CI=true` is what every Actions step sees and what Jest reads as
/// `--ci` (no snapshot writes, so a missing snapshot fails as it does in
/// CI). npm's download cache is step A's, bound from host state and never
/// in the tree; its update notifier and debug logs would write
/// timestamped files. Jest's transform and haste-map caches live under
/// os.tmpdir(), the sandbox's fresh tmpfs, so every run is cold and no
/// mtime-keyed haste map survives between runs (see DECISIONS.md).
/// Vitest writes its results cache under node_modules/.vite, a carve-out of
/// the sealed node_modules; babel and others under node_modules/.cache.
pub static NODE: Profile = Profile {
    toolchain: Toolchain::Node,
    markers: &["package.json"],
    env: &[
        ("CI", "true"),
        ("HOME", "/workspace/.stillpass/home"),
        ("LANG", "C.UTF-8"),
        ("PATH", "/usr/local/node/bin:/usr/bin:/bin"),
        ("TZ", "UTC"),
        ("npm_config_audit", "false"),
        ("npm_config_cache", "/workspace/.stillpass/npm-cache"),
        ("npm_config_fund", "false"),
        ("npm_config_logs_max", "0"),
        ("npm_config_update_notifier", "false"),
    ],
    setup: "npm ci",
    test: "npm test",
    net_env: &[],
    setup_cache: &[".stillpass/npm-cache"],
    ephemeral: &[".stillpass/home"],
    // npm ci removes node_modules before installing; nothing to seed.
    seed_dirs: &[],
    seals: &[SealRule::Named {
        name: "node_modules",
        carve_outs: &[".cache", ".vite", ".vitest"],
    }],
    warm: None,
    no_outputs_under: &[],
    source_exts: &[".js", ".mjs", ".cjs", ".ts", ".mts", ".cts", ".jsx", ".tsx"],
    comment: "//",
};

/// Python. The dependency tree is a venv under the workspace, sealed.
/// `PYTHONDONTWRITEBYTECODE=1`: a timestamp pyc written under observation
/// would carry the observer's constant mtime, and if it ever came back into
/// a tree Python would trust it over a same-size source edit; so no step
/// writes one. `SOURCE_DATE_EPOCH` makes install-time compilation (step A:
/// pip's, or uv's with `UV_COMPILE_BYTECODE`) produce checked-hash pycs
/// instead of timestamp ones (PEP 552): Python then validates a
/// site-packages pyc by hashing its source, which does not depend on
/// mtime, so a sealed venv is neither recompiled on every run nor trusted
/// stale. `PYTHONHASHSEED=0` keeps set and dict ordering, and so test
/// output, the same from run to run. uv uses the host's Python (never a
/// downloaded one) and puts the project environment in the venv;
/// `UV_LINK_MODE=copy` keeps the venv's files their own rather than hard
/// links into uv's cache.
pub static PYTHON: Profile = Profile {
    toolchain: Toolchain::Python,
    markers: &[
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
        "requirements.txt",
        "tox.ini",
    ],
    env: &[
        ("CI", "true"),
        ("HOME", "/workspace/.stillpass/home"),
        ("LANG", "C.UTF-8"),
        (
            "PATH",
            "/workspace/.stillpass/venv/bin:/usr/local/python/bin:/usr/local/uv/bin:/usr/bin:/bin",
        ),
        ("PIP_CACHE_DIR", "/workspace/.stillpass/pip-cache"),
        ("PIP_DISABLE_PIP_VERSION_CHECK", "1"),
        ("PIP_NO_INPUT", "1"),
        ("PYTHONDONTWRITEBYTECODE", "1"),
        ("PYTHONHASHSEED", "0"),
        ("PYTHONNOUSERSITE", "1"),
        ("SOURCE_DATE_EPOCH", "315532800"),
        ("TZ", "UTC"),
        ("UV_CACHE_DIR", "/workspace/.stillpass/uv-cache"),
        ("UV_COMPILE_BYTECODE", "1"),
        ("UV_FROZEN", "1"),
        ("UV_LINK_MODE", "copy"),
        ("UV_NO_PROGRESS", "1"),
        ("UV_PROJECT_ENVIRONMENT", "/workspace/.stillpass/venv"),
        ("UV_PYTHON", "/usr/local/python/bin/python3"),
        ("UV_PYTHON_DOWNLOADS", "never"),
        ("UV_PYTHON_PREFERENCE", "only-system"),
        ("VIRTUAL_ENV", "/workspace/.stillpass/venv"),
    ],
    setup: "uv sync --frozen",
    test: "python -m pytest",
    net_env: &[],
    setup_cache: &[".stillpass/pip-cache", ".stillpass/uv-cache"],
    ephemeral: &[".stillpass/home"],
    seed_dirs: &[".stillpass/venv"],
    seals: &[SealRule::EnvDir {
        env: "VIRTUAL_ENV",
        carve_outs: &[],
    }],
    warm: None,
    no_outputs_under: &["__pycache__"],
    source_exts: &[".py"],
    comment: "#",
};

#[cfg(test)]
mod tests {
    use sp_cas::{Entry, Hash};

    use super::*;

    #[test]
    fn detection() {
        assert_eq!(
            Toolchain::detect(["go.mod", "README.md"]),
            Ok(Toolchain::Go)
        );
        assert_eq!(
            Toolchain::detect(["package.json", "package-lock.json"]),
            Ok(Toolchain::Node)
        );
        assert_eq!(Toolchain::detect(["pyproject.toml"]), Ok(Toolchain::Python));
        assert_eq!(
            Toolchain::detect(["setup.py", "tox.ini"]),
            Ok(Toolchain::Python)
        );
        assert_eq!(Toolchain::detect(["README.md"]), Err(Detect::None));
        assert_eq!(
            Toolchain::detect(["go.mod", "package.json"]),
            Err(Detect::Ambiguous(vec![Toolchain::Go, Toolchain::Node]))
        );
        assert_eq!("python".parse(), Ok(Toolchain::Python));
        assert!("ruby".parse::<Toolchain>().is_err());
        for t in Toolchain::ALL {
            assert_eq!(t.profile().toolchain, *t);
            assert_eq!(t.name().parse::<Toolchain>(), Ok(*t));
        }
    }

    #[test]
    fn every_profile_env_is_complete_and_sorted_by_name() {
        for t in Toolchain::ALL {
            let p = t.profile();
            let names: Vec<&str> = p.env.iter().map(|(k, _)| *k).collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(names, sorted, "{t}: env listed sorted and unique");
            for required in ["HOME", "PATH", "LANG", "TZ"] {
                assert!(p.env_value(required).is_some(), "{t}: {required}");
            }
            assert!(p.env_dir("HOME").is_some(), "{t}: HOME under the workspace");
            for c in p.setup_cache {
                assert!(!c.starts_with('/') && !c.is_empty());
            }
            if let Some(w) = &p.warm {
                assert!(
                    p.env_dir(w.cache_env).is_some(),
                    "{t}: warm cache in the workspace"
                );
            }
        }
    }

    #[test]
    fn seals_follow_the_tree() {
        let go = Tree::from_leaves([
            Entry::file("go.mod", Hash::of(b"m"), 0o644),
            Entry::file(".stillpass/gomod/a.b/c@v1/x.go", Hash::of(b"x"), 0o444),
            Entry::file(".stillpass/gomod/cache/download/x", Hash::of(b"d"), 0o444),
        ]);
        assert_eq!(
            GO.seals(&go),
            [Seal {
                prefix: ".stillpass/gomod".into(),
                carve_outs: vec![".stillpass/gomod/cache".into()],
            }]
        );
        assert!(
            GO.seals(&Tree::from_leaves([Entry::file(
                "go.mod",
                Hash::of(b"m"),
                0o644
            )]))
            .is_empty()
        );

        let node = Tree::from_leaves([
            Entry::file("package.json", Hash::of(b"p"), 0o644),
            Entry::file("node_modules/a/index.js", Hash::of(b"a"), 0o644),
            Entry::file(
                "node_modules/a/node_modules/b/index.js",
                Hash::of(b"b"),
                0o644,
            ),
            Entry::file("packages/x/node_modules/c/index.js", Hash::of(b"c"), 0o644),
            Entry::file("src/node_modules.js", Hash::of(b"n"), 0o644),
        ]);
        let seals = NODE.seals(&node);
        let prefixes: Vec<&str> = seals.iter().map(|s| s.prefix.as_str()).collect();
        assert_eq!(prefixes, ["node_modules", "packages/x/node_modules"]);
        assert_eq!(
            seals[0].carve_outs,
            [
                "node_modules/.cache",
                "node_modules/.vite",
                "node_modules/.vitest"
            ]
        );

        let py = Tree::from_leaves([
            Entry::file("pyproject.toml", Hash::of(b"p"), 0o644),
            Entry::file(".stillpass/venv/bin/python", Hash::of(b"p"), 0o755),
        ]);
        assert_eq!(PYTHON.seals(&py)[0].prefix, ".stillpass/venv");
        assert!(PYTHON.seals(&py)[0].carve_outs.is_empty());
    }

    #[test]
    fn editable_sources() {
        assert!(GO.is_editable_source("pkg/cmd/root.go"));
        assert!(!GO.is_editable_source("pkg/cmd/root_test.go"));
        assert!(!GO.is_editable_source("pkg/testdata/x.go"));
        assert!(!GO.is_editable_source(".stillpass/gomod/a/b.go"));
        assert!(NODE.is_editable_source("src/index.js"));
        assert!(NODE.is_editable_source("src/impl/zone.ts"));
        assert!(!NODE.is_editable_source("src/index.test.js"));
        assert!(!NODE.is_editable_source("test/helpers.js"));
        assert!(!NODE.is_editable_source("node_modules/x/index.js"));
        assert!(!NODE.is_editable_source("package.json"));
        assert!(PYTHON.is_editable_source("starlette/routing.py"));
        assert!(!PYTHON.is_editable_source("tests/test_routing.py"));
        assert!(!PYTHON.is_editable_source("tests/conftest.py"));
        assert!(!PYTHON.is_editable_source(".stillpass/venv/lib/python3.12/site-packages/x.py"));
        assert!(!PYTHON.is_editable_source("pkg/__pycache__/x.py"));
    }

    #[test]
    fn workspace_paths() {
        assert_eq!(workspace_relative("/workspace/a/b/"), Some("a/b".into()));
        assert_eq!(workspace_relative("/workspace"), None);
        assert_eq!(workspace_relative("/tmp/x"), None);
        assert_eq!(
            GO.env_dir("GOMODCACHE").as_deref(),
            Some(".stillpass/gomod")
        );
        assert_eq!(GO.env_dir("GOPROXY"), None);
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
}
