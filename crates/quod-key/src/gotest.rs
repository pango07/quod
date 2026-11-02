//! Go's test-result cache under Quod's guard.
//!
//! A warm Go step used to run with `-count=1`: go test's result cache
//! decides "inputs unchanged" from what package os logged while the test
//! ran (internal/testlog), rechecks only paths inside the package's module,
//! and knows nothing of what the test's package variables or TestMain read
//! before m.Run, what a process it started read, or scratch files written
//! before it ran. Inside Quod that is an under-record: a cached result
//! replayed into a manifest that does not hold what the result depended on.
//!
//! With the guard, go test keeps its cache and compiles every test binary
//! against files of the toolchain's own standard library changed through
//! `go -overlay`: a guard that logs from its own package initialization on
//! and classifies every access, and testing/internal/testdeps (the
//! toolchain's file, patched here) which hands go's log to it and writes
//! its verdict into the test log. A result that may depend on anything go
//! test will not recheck gets a line go test cannot parse, and go test
//! caches only results whose log it can parse (cmd/go/internal/test:
//! computeTestInputsID, errBadTestInputs). A cacheable result is one whose
//! every input go test rechecks on the next run, so the manifest of a run
//! that replays it records those inputs.
//!
//! The guard comes in two layouts, chosen per toolchain by the probe in
//! quod-cli (`Layout`):
//!
//! - [`Layout::Package`] (v3): the guard is its own package, `0quod/guard`,
//!   added to GOROOT by the overlay and imported by testdeps. Go
//!   initializes packages one at a time, the first in import-path order
//!   among those whose imports are all initialized; the package imports
//!   only what os already imports, so it is ready the moment os is, and
//!   its path sorts before every package that imports os, so it is
//!   initialized before any package-level variable can open a file through
//!   os: a dependency's as much as the test package's own. Only testdeps,
//!   the guard and the test mains compile differently from a stock build,
//!   so `go build`, `go vet` and `go test` share every other compile.
//!   cmd/go must find the package through the overlay with the module
//!   index off (Quod's profile sets `goindex=0`): go1.27's does, go1.26's
//!   reads GOROOT from disk there, and the probe sees which.
//! - [`Layout::Testing`] (v2): a file added to package testing, which
//!   initializes after its dependencies and after every package sorting
//!   before "testing" whose imports are done, so a dependency's init is
//!   not logged there; and every package importing testing compiles
//!   differently under the overlay. Kept for toolchains where the package
//!   layout does not resolve, under its own, unchanged, key.
//!
//! The guard's layout and version are in the overlay's path, which is in
//! GOFLAGS and so in the step key: a changed guard is a changed key.

use crate::Knob;

/// How the guard's code reaches the test binary. The version is part of
/// the sandbox path, so of the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// v3: the guard's own package, imported by testdeps.
    Package,
    /// v2: a file in package testing (2026-11-02: the testing file builds
    /// on linux only, the overlay names the GOROOT `go env` reports, proofs
    /// run with the cache expired).
    Testing,
}

/// The layouts in the order the probe tries them.
pub const LAYOUTS: [Layout; 2] = [Layout::Package, Layout::Testing];

impl Layout {
    /// Where the guard's files are bound in the sandbox, read-only, outside
    /// the observed tree.
    pub const fn sandbox_dir(self) -> &'static str {
        match self {
            Layout::Package => "/quod/go-test-guard/v3",
            Layout::Testing => "/quod/go-test-guard/v2",
        }
    }

    /// The GOFLAGS setting that compiles test binaries with the guard: the
    /// warm Go profile's replacement for `-count=1`.
    pub const fn knob(self) -> Knob {
        Knob {
            name: "GOFLAGS",
            sep: ' ',
            item: match self {
                Layout::Package => "-overlay=/quod/go-test-guard/v3/overlay.json",
                Layout::Testing => "-overlay=/quod/go-test-guard/v2/overlay.json",
            },
        }
    }

    /// The name the probe reports.
    pub const fn name(self) -> &'static str {
        match self {
            Layout::Package => "package",
            Layout::Testing => "testing",
        }
    }
}

/// The guard's own package (`Layout::Package`): its import path. It must
/// sort before every package that imports os, in the linker's order
/// (`objabi.PathToPrefix(path) + "..inittask"`, bytewise): a leading
/// digit sorts before every letter, so before every standard package and
/// every module path that starts with a letter. The residual: a package
/// whose path starts with `+`, `-` or `0` followed by a byte below `q`
/// and that reads a file through os at init.
pub const GUARD_PKG: &str = "0quod/guard";

/// The package's files in GOROOT's src/0quod/guard: the guard (linux) and
/// the stub for every other GOOS.
pub const GUARD_FILE: &str = "guard.go";
pub const GUARD_OTHER_FILE: &str = "unsupported.go";

const GUARD_PACKAGE: &str = include_str!("gotest/guard_package.go.txt");
const GUARD_PACKAGE_OTHER: &str = include_str!("gotest/guard_package_other.go.txt");

/// What the guard package may import: os, and what os imports itself
/// (transitively, as `go list -deps os` lists it in go1.26 and go1.27).
/// Anything else could be initialized after a package that sorts after
/// the guard, and that package before the guard. The probe checks the
/// same against the step's toolchain.
pub const GUARD_IMPORTS: &[&str] = &[
    "internal/stringslite",
    "internal/testlog",
    "os",
    "path",
    "sync",
    "syscall",
    "unsafe",
];

/// The imports of a Go file's single import block, in order.
pub fn go_imports(src: &str) -> Vec<&str> {
    let Some(start) = src.find("\nimport (\n") else {
        return src
            .lines()
            .filter_map(|l| l.strip_prefix("import \""))
            .filter_map(|l| l.strip_suffix('"'))
            .collect();
    };
    let block = &src[start + "\nimport (\n".len()..];
    let block = &block[..block.find("\n)\n").unwrap_or(block.len())];
    block
        .lines()
        .filter_map(|l| l.trim().rsplit(' ').next())
        .filter_map(|l| l.strip_prefix('"').and_then(|l| l.strip_suffix('"')))
        .collect()
}

/// The guard's file in package testing (`Layout::Testing`), with the
/// workspace path to fill in.
const TESTING_GUARD: &str = include_str!("gotest/testing_guard.go.txt");

/// testdeps' own file, relative to GOROOT.
pub const TESTDEPS: &str = "src/testing/internal/testdeps/deps.go";

/// The script's words, as the guard reads them: split at whitespace and
/// the shell's operators, with leading quotes dropped (`"GOFLAGS=x"`).
fn words(script: &str) -> Vec<&str> {
    script
        .split(|c: char| c.is_whitespace() || ";|&()`".contains(c))
        .map(|w| w.trim_start_matches(['"', '\'']))
        .filter(|w| !w.is_empty())
        .collect()
}

/// A word that names the go command: `go`, or a path ending in `/go`.
fn is_go(w: &str) -> bool {
    w == "go" || w.ends_with("/go")
}

/// Whether `script` runs `go test` (a `go` word followed by `test`), and no
/// go invocation in it makes something from packages compiled with the
/// guard's files: a test binary or a build output (`-c`, `-o`), an
/// installed binary (`go install`), or a program it runs (`go run`), each
/// of which would then carry Quod's code into what the step leaves or runs.
/// Anything else keeps `-count=1`: Go's test cache stays off where Quod
/// cannot tell.
pub fn runs_go_test(script: &str) -> bool {
    let words = words(script);
    let after_go = |sub: &str| words.windows(2).any(|w| is_go(w[0]) && w[1] == sub);
    let artifacts = words
        .iter()
        .any(|w| *w == "-c" || *w == "-o" || w.starts_with("-o=") || w.starts_with("-exec"));
    after_go("test") && !after_go("install") && !after_go("run") && !artifacts
}

/// Whether `script` runs `go <sub>` (a `go` word, or a path ending in
/// `/go`, followed by `sub`).
pub fn runs_go(script: &str, sub: &str) -> bool {
    words(script).windows(2).any(|w| is_go(w[0]) && w[1] == sub)
}

/// Variables the guard's decision depends on: the step's env may not set
/// them (`env_allows_guard`), and neither may its script
/// (`script_sets_toolchain_env`).
const TOOLCHAIN_VARS: &[&str] = &[
    "GOFLAGS",
    "GOROOT",
    "PATH",
    "GOTOOLCHAIN",
    "GOENV",
    "GO111MODULE",
    "GODEBUG",
];

/// The first assignment word in `script` to a variable the guard depends
/// on (`GOFLAGS=-count=1 go test`, `export PATH=...`, `PATH+=...`, `go env
/// -w GOFLAGS=...`): the go command the script runs may then not be the
/// one, or have the flags, the guard was made for. None if there is none.
pub fn script_sets_toolchain_env(script: &str) -> Option<&str> {
    words(script).into_iter().find(|w| {
        TOOLCHAIN_VARS.iter().any(|v| {
            w.strip_prefix(v)
                .is_some_and(|r| r.starts_with('=') || r.starts_with("+="))
        })
    })
}

/// The go commands `script` names by path (`/usr/local/go/bin/go test`):
/// each must be the toolchain the guard was made for.
pub fn script_go_paths(script: &str) -> Vec<&str> {
    words(script)
        .into_iter()
        .filter(|w| *w != "go" && w.ends_with("/go"))
        .collect()
}

/// Whether the guard can stand in for `-count=1` given the step's extra
/// env: not when it brings its own overlay, test count or GOROOT (the
/// overlay names the profile's GOROOT), turns modules off, or speaks of
/// the module index (the probe runs with the profile's `goindex=0`, which
/// decides whether cmd/go finds the guard's package through the overlay;
/// the profile's knob adds it to any other GODEBUG).
pub fn env_allows_guard(extra_env: &[(String, String)]) -> bool {
    extra_env.iter().all(|(k, v)| match k.as_str() {
        "GOFLAGS" => !v.contains("-overlay") && !v.contains("-count") && !v.contains("-exec"),
        "GOROOT" | "GO111MODULE" | "GOTOOLCHAIN" | "GOENV" => false,
        "GODEBUG" => !v.contains("goindex"),
        _ => true,
    })
}

/// The Go releases whose cmd/go and package testing this guard was
/// reviewed against (the test cache's rules, testdeps, the overlay, the
/// package initialization order): `go1.26`, `go1.27` and their patch
/// releases. Any other version (a release candidate, an experiment build,
/// the next minor) keeps `-count=1` until the guard is reviewed against it.
pub const REVIEWED_GO: &[&str] = &["1.26", "1.27"];

/// Whether `goversion` (as `go env GOVERSION` prints it) is a release in
/// `REVIEWED_GO`.
pub fn reviewed_go_version(goversion: &str) -> bool {
    let Some(v) = goversion.strip_prefix("go") else {
        return false;
    };
    REVIEWED_GO.iter().any(|minor| {
        v.strip_prefix(minor).is_some_and(|rest| {
            rest.is_empty()
                || rest
                    .strip_prefix('.')
                    .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
    })
}

/// The name the guard's testing file has in GOROOT's src/testing
/// (`Layout::Testing`); a `go list` of package testing under the overlay
/// lists it exactly when the overlay applies.
pub const TESTING_FILE: &str = "quod_test_cache_guard.go";

/// The overlay (`go -overlay` JSON) for `layout` under `goroot`: testdeps
/// replaced by `testdeps_go`, and the guard's files (`guard`: for the
/// package layout the linux file and the stub, for the testing layout the
/// testing file) added where the layout puts them.
pub fn overlay_json(layout: Layout, goroot: &str, testdeps_go: &str, guard: &[&str]) -> String {
    let root = goroot.trim_end_matches('/');
    let mut replace = serde_json::Map::new();
    replace.insert(format!("{root}/{TESTDEPS}"), testdeps_go.into());
    match layout {
        Layout::Package => {
            for (name, src) in [GUARD_FILE, GUARD_OTHER_FILE].iter().zip(guard) {
                replace.insert(format!("{root}/src/{GUARD_PKG}/{name}"), (*src).into());
            }
        }
        Layout::Testing => {
            replace.insert(
                format!("{root}/src/testing/{TESTING_FILE}"),
                guard[0].into(),
            );
        }
    }
    serde_json::json!({ "Replace": replace }).to_string()
}

/// The guard's files for `layout`, as (name in its sandbox dir, contents):
/// the overlay JSON naming every replaced file under `goroot`, spelled
/// exactly as the step's go command spells GOROOT (`go env GOROOT`: cmd/go
/// matches overlay paths by spelling, not by file), the patched testdeps
/// (from `deps_go`, the toolchain's own), and the guard's own files.
/// `namespace` (the step key) goes into the testdeps file, so test
/// binaries, and with them cached results, of different keys never
/// coincide. `Err` when testdeps is not the file this patch knows; the
/// step then keeps `-count=1`.
pub fn guard_files(
    layout: Layout,
    goroot: &str,
    deps_go: &str,
    namespace: &str,
    workspace: &str,
) -> Result<Vec<(&'static str, String)>, String> {
    let dir = layout.sandbox_dir();
    let testdeps = patch_testdeps(layout, deps_go, namespace)?;
    match layout {
        Layout::Package => {
            let overlay = overlay_json(
                layout,
                goroot,
                &format!("{dir}/testdeps.go"),
                &[&format!("{dir}/guard.go"), &format!("{dir}/unsupported.go")],
            );
            Ok(vec![
                ("overlay.json", overlay),
                ("testdeps.go", testdeps),
                (
                    "guard.go",
                    GUARD_PACKAGE.replace("QUOD_WORKSPACE", workspace),
                ),
                ("unsupported.go", GUARD_PACKAGE_OTHER.to_string()),
            ])
        }
        Layout::Testing => {
            let overlay = overlay_json(
                layout,
                goroot,
                &format!("{dir}/testdeps.go"),
                &[&format!("{dir}/testing.go")],
            );
            Ok(vec![
                ("overlay.json", overlay),
                ("testdeps.go", testdeps),
                (
                    "testing.go",
                    TESTING_GUARD.replace("QUOD_WORKSPACE", workspace),
                ),
            ])
        }
    }
}

/// The edits to testdeps, each found exactly once. The first is the
/// testlog import, after which the package layout imports the guard.
const IMPORT_OLD: &str = "\t\"internal/testlog\"\n";

const IMPORT_NEW: &str = "\t\"internal/testlog\"\n\tquodguard \"0quod/guard\"\n";

const START_OLD: &str = "	if !log.set {
		// Tests that define TestMain and then run m.Run multiple times
		// will call StartTestLog/StopTestLog multiple times.
		// Checking log.set avoids calling testlog.SetLogger multiple times
		// (which will panic) and also avoids writing the header multiple times.
		log.set = true
		testlog.SetLogger(&log)
		log.w.WriteString(\"# test log\\n\") // known to cmd/go/internal/test/test.go
	}
	log.mu.Unlock()
}";

/// `ATTACH` is the layout's attach call.
const START_NEW: &str = "	first := !log.set
	if first {
		// Tests that define TestMain and then run m.Run multiple times
		// will call StartTestLog/StopTestLog multiple times.
		// Checking log.set avoids calling testlog.SetLogger multiple times
		// (which will panic) and also avoids writing the header multiple times.
		log.set = true
		log.w.WriteString(\"# test log\\n\") // known to cmd/go/internal/test/test.go
	}
	log.mu.Unlock()
	if first {
		ATTACH
	}
}";

const STOP_OLD: &str = "func (TestDeps) StopTestLog() error {
	log.mu.Lock()
	defer log.mu.Unlock()
	err := log.w.Flush()";

/// `VERDICT` is the layout's verdict call.
const STOP_NEW: &str = "func (TestDeps) StopTestLog() error {
	why := VERDICT
	log.mu.Lock()
	defer log.mu.Unlock()
	// A test-log entry, not a variable anything sets or reads: go test
	// hashes each logged getenv's name and value (here: unset) into the
	// result's inputs, so they name the step key too. What separates keys
	// is quodNamespace, compiled into the binary; this is a second
	// separation, not one the guard relies on.
	log.w.WriteString(\"getenv QUOD_TESTCACHE_\" + quodNamespace + \"\\n\")
	if why != \"\" {
		log.w.WriteString(\"quod-uncacheable \" + why + \"\\n\")
	}
	err := log.w.Flush()";

const PACKAGE_TAIL: &str = "
// ---- Quod's test-cache guard, version 3: testdeps' half (the guard is
// package 0quod/guard, imported above). Changed by Quod, through go's
// -overlay; not Go's. ----

// The step key: test binaries of different keys differ, so go test never
// serves one key's cached result to another.
var quodNamespace = \"QUOD_NAMESPACE\"
";

const TESTING_TAIL: &str = "
// ---- Quod's test-cache guard, version 2: testdeps' half (the other is
// the guard's file in package testing). Changed by Quod, through go's
// -overlay; not Go's. ----

// The step key: test binaries of different keys differ, so go test never
// serves one key's cached result to another.
var quodNamespace = \"QUOD_NAMESPACE\"

type quodGuardLog interface {
	QuodAttach(testlog.Interface)
	QuodVerdict() string
}

// quodUnguarded: go's log started without the guard's, so nothing was
// logged before m.Run.
var quodUnguarded bool

func quodAttach() {
	if g, ok := testlog.Logger().(quodGuardLog); ok {
		g.QuodAttach(&log)
		return
	}
	quodUnguarded = true
	if testlog.Logger() == nil {
		testlog.SetLogger(&log)
	}
}

func quodVerdict() string {
	if g, ok := testlog.Logger().(quodGuardLog); ok && !quodUnguarded {
		return g.QuodVerdict()
	}
	return \"nothing was logged before the tests started\"
}
";

fn patch_testdeps(layout: Layout, deps_go: &str, namespace: &str) -> Result<String, String> {
    if !namespace
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b':')
    {
        return Err(format!("namespace {namespace:?} is not a step key"));
    }
    let (attach, verdict, tail, import) = match layout {
        Layout::Package => (
            "quodguard.Attach(&log)",
            "quodguard.Verdict()",
            PACKAGE_TAIL,
            Some((IMPORT_OLD, IMPORT_NEW)),
        ),
        Layout::Testing => ("quodAttach()", "quodVerdict()", TESTING_TAIL, None),
    };
    let start_new = START_NEW.replace("ATTACH", attach);
    let stop_new = STOP_NEW.replace("VERDICT", verdict);
    let mut edits = vec![
        (START_OLD, start_new.as_str()),
        (STOP_OLD, stop_new.as_str()),
    ];
    edits.extend(import);
    let mut s = deps_go.to_string();
    for (old, new) in edits {
        if s.matches(old).count() != 1 {
            return Err(format!(
                "testdeps has {} copies of the code the guard changes (`{}`)",
                s.matches(old).count(),
                old.lines().next().unwrap_or_default().trim()
            ));
        }
        s = s.replacen(old, new, 1);
    }
    for needed in ["\"bufio\"", "\"internal/testlog\"", "var log testLog"] {
        if !s.contains(needed) {
            return Err(format!("testdeps lacks {needed}"));
        }
    }
    s.push_str(&tail.replace("QUOD_NAMESPACE", namespace));
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// testdeps as Go 1.26 and 1.27 ship it, the parts the patch reads.
    const DEPS: &str = "package testdeps

import (
	\"bufio\"
	\"internal/testlog\"
	\"io\"
	\"strings\"
	\"sync\"
)

type testLog struct {
	mu  sync.Mutex
	w   *bufio.Writer
	set bool
}

var log testLog

func (TestDeps) StartTestLog(w io.Writer) {
	log.mu.Lock()
	log.w = bufio.NewWriter(w)
	if !log.set {
		// Tests that define TestMain and then run m.Run multiple times
		// will call StartTestLog/StopTestLog multiple times.
		// Checking log.set avoids calling testlog.SetLogger multiple times
		// (which will panic) and also avoids writing the header multiple times.
		log.set = true
		testlog.SetLogger(&log)
		log.w.WriteString(\"# test log\\n\") // known to cmd/go/internal/test/test.go
	}
	log.mu.Unlock()
}

func (TestDeps) StopTestLog() error {
	log.mu.Lock()
	defer log.mu.Unlock()
	err := log.w.Flush()
	log.w = nil
	return err
}
";

    #[test]
    fn go_test_steps_are_found_and_artifact_builds_are_not() {
        assert!(runs_go_test("go test ./..."));
        assert!(runs_go_test("go build ./... && go test -race ./pkg/..."));
        assert!(runs_go_test("cd sub; /usr/local/go/bin/go test ./..."));
        assert!(!runs_go_test("go vet ./..."));
        assert!(!runs_go_test("make test"));
        assert!(!runs_go_test("go test -c -o x.test ./pkg"));
        assert!(!runs_go_test(
            "go test ./... && go build -o bin/gh ./cmd/gh"
        ));
        assert!(!runs_go_test("go test -exec=sudo ./..."));
        // An installed binary or a program run would carry the guard's
        // files (MINOR 5).
        assert!(!runs_go_test("go install ./cmd/gh && go test ./..."));
        assert!(!runs_go_test("go test ./... && go run ./script/gen.go"));
        assert!(!runs_go_test("/usr/local/go/bin/go run . ; go test ./..."));
    }

    #[test]
    fn go_subcommands_are_found() {
        assert!(runs_go("go vet ./...", "vet"));
        assert!(runs_go("cd x && /usr/local/go/bin/go test ./...", "test"));
        assert!(runs_go("go build ./... && go vet ./...", "vet"));
        assert!(!runs_go("go build ./...", "vet"));
        assert!(!runs_go("make vet", "vet"));
        assert!(!runs_go("golangci-lint run ./...", "run"));
    }

    #[test]
    fn a_script_that_sets_the_toolchain_or_names_a_go_is_seen() {
        for s in [
            "GOFLAGS=-count=1 go test ./...",
            "export PATH=/opt/go/bin:$PATH && go test ./...",
            "PATH+=:/x go test ./...",
            "env GOROOT=/opt/go go test ./...",
            "GOTOOLCHAIN=go1.28.0 go test ./...",
            "go env -w GOFLAGS=-count=1; go test ./...",
            "\"GOFLAGS=-x\" go test ./...",
            "GODEBUG=goindex=1 go test ./...",
        ] {
            assert!(script_sets_toolchain_env(s).is_some(), "{s}");
        }
        for s in [
            "go test ./...",
            "GOOS=windows go build ./... && go test ./...",
            "MYPATH=/x go test ./...",
            "echo $PATH; go test ./...",
        ] {
            assert_eq!(script_sets_toolchain_env(s), None, "{s}");
        }
        assert_eq!(
            script_go_paths("cd sub; /usr/local/go/bin/go test ./... && ./bin/go vet"),
            vec!["/usr/local/go/bin/go", "./bin/go"]
        );
        assert!(script_go_paths("go test ./...").is_empty());
    }

    #[test]
    fn only_reviewed_go_releases_get_the_guard() {
        for v in [
            "go1.26",
            "go1.26.0",
            "go1.26.4",
            "go1.27",
            "go1.27.3",
            "go1.27.12",
        ] {
            assert!(reviewed_go_version(v), "{v}");
        }
        for v in [
            "go1.25.7",
            "go1.28.0",
            "go1.270",
            "go1.27rc1",
            "go1.27.3 X:boringcrypto",
            "go1.27.",
            "devel go1.28-abc",
            "1.27.3",
            "",
        ] {
            assert!(!reviewed_go_version(v), "{v}");
        }
    }

    #[test]
    fn the_env_may_not_bring_its_own_overlay_count_goroot_or_godebug() {
        let e = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        assert!(env_allows_guard(&[]));
        assert!(env_allows_guard(&e("GOFLAGS", "-mod=mod -race")));
        assert!(!env_allows_guard(&e("GOFLAGS", "-count=1")));
        assert!(!env_allows_guard(&e("GOFLAGS", "-overlay=/x.json")));
        assert!(!env_allows_guard(&e("GOROOT", "/opt/go")));
        assert!(!env_allows_guard(&e("GO111MODULE", "off")));
        assert!(!env_allows_guard(&e("GODEBUG", "goindex=1")));
        assert!(env_allows_guard(&e("GODEBUG", "http2debug=1")));
    }

    /// The guard's linux files build for linux only: they call getrusage
    /// and waitid, which other GOOSes' syscall lacks, and the overlay
    /// applies to every build in the step (`GOOS=windows go build`, MAJOR
    /// 3). The package layout's stub covers the rest.
    #[test]
    fn the_linux_files_are_linux_only_and_the_stub_is_the_rest() {
        for (src, pkg) in [(TESTING_GUARD, "testing"), (GUARD_PACKAGE, "quodguard")] {
            assert_eq!(src.lines().next(), Some("//go:build linux"), "{pkg}");
            // A constraint must be followed by a blank line, before the
            // package clause.
            assert_eq!(src.lines().nth(1), Some(""), "{pkg}");
            assert!(src.contains(&format!("\npackage {pkg}\n")), "{pkg}");
        }
        assert_eq!(
            GUARD_PACKAGE_OTHER.lines().next(),
            Some("//go:build !linux")
        );
        assert!(GUARD_PACKAGE_OTHER.contains("\npackage quodguard\n"));
        assert!(GUARD_PACKAGE_OTHER.contains("func Attach(to testlog.Interface)"));
        assert!(GUARD_PACKAGE_OTHER.contains("func Verdict() string"));
    }

    /// The package layout's ordering argument needs the guard to be ready
    /// the moment os is initialized: it may import os and what os imports,
    /// nothing else (not path/filepath, which imports os; not strings,
    /// which os does not import).
    #[test]
    fn the_guard_package_imports_only_what_os_imports() {
        let imports = go_imports(GUARD_PACKAGE);
        assert!(!imports.is_empty());
        for i in &imports {
            assert!(GUARD_IMPORTS.contains(i), "{i} is not an import of os");
        }
        assert!(!imports.contains(&"path/filepath"));
        assert!(!imports.contains(&"strings"));
        assert_eq!(go_imports(GUARD_PACKAGE_OTHER), vec!["internal/testlog"]);
        assert_eq!(
            go_imports("package x\n\nimport (\n\tq \"a/b\"\n\t\"c\"\n)\n\nvar v = 1\n"),
            vec!["a/b", "c"]
        );
        assert_eq!(go_imports("package x\n\nimport \"os\"\n"), vec!["os"]);
    }

    /// The guard's path sorts first: in the linker's order (bytewise on
    /// the escaped path) before every standard package, every module path
    /// starting with a letter, and the test main.
    #[test]
    fn the_guard_package_sorts_before_every_package_importing_os() {
        let key = |p: &str| format!("{p}..inittask");
        for p in [
            "archive/zip",
            "crypto/internal/cryptotest",
            "flag",
            "fmt",
            "os",
            "testing",
            "testing/internal/testdeps",
            "main",
            "Example/x",
            "example.com/g/h",
            "github.com/cli/cli/v2/internal/config",
            "cloud.google.com/go",
            "1x/y",
            "0r",
        ] {
            assert!(key(GUARD_PKG) < key(p), "{p}");
        }
        assert!(GUARD_PKG.starts_with('0'));
        assert!(!GUARD_PKG.contains('.'), "a dot would be escaped as %2e");
        assert!(GUARD_PACKAGE.contains("\npackage quodguard\n"));
    }

    #[test]
    fn testdeps_is_patched_for_the_package_layout_or_refused() {
        let files = guard_files(
            Layout::Package,
            "/usr/local/go/",
            DEPS,
            "b3:abc123",
            "/workspace",
        )
        .unwrap();
        let get = |n: &str| &files.iter().find(|(f, _)| *f == n).unwrap().1;
        let deps = get("testdeps.go");
        assert!(deps.contains("\tquodguard \"0quod/guard\"\n"));
        assert!(deps.contains("quodguard.Attach(&log)"));
        assert!(!deps.contains("testlog.SetLogger(&log)\n\t\tlog.w.WriteString"));
        assert!(deps.contains("why := quodguard.Verdict()"));
        assert!(deps.contains("var quodNamespace = \"b3:abc123\""));
        assert!(!deps.contains("quodUnguarded"));
        assert!(get("guard.go").contains("quodWorkspace = \"/workspace\""));
        assert!(get("unsupported.go").contains("//go:build !linux"));
        let overlay: serde_json::Value = serde_json::from_str(get("overlay.json")).unwrap();
        let r = overlay["Replace"].as_object().unwrap();
        // One spelling of GOROOT: the one the go command reports.
        assert_eq!(r.len(), 3, "{r:?}");
        assert_eq!(
            r["/usr/local/go/src/testing/internal/testdeps/deps.go"],
            "/quod/go-test-guard/v3/testdeps.go"
        );
        assert_eq!(
            r["/usr/local/go/src/0quod/guard/guard.go"],
            "/quod/go-test-guard/v3/guard.go"
        );
        assert_eq!(
            r["/usr/local/go/src/0quod/guard/unsupported.go"],
            "/quod/go-test-guard/v3/unsupported.go"
        );
        assert_eq!(
            Layout::Package.knob().item,
            "-overlay=/quod/go-test-guard/v3/overlay.json"
        );
        // Another testdeps: refused, never half-patched.
        let changed = DEPS.replace("\t\"internal/testlog\"\n", "\t\"internal/testlog\" // x\n");
        assert!(guard_files(Layout::Package, "/g", &changed, "b3:abc", "/workspace").is_err());
        assert!(guard_files(Layout::Package, "/g", DEPS, "b3:a\"b", "/workspace").is_err());
    }

    #[test]
    fn testdeps_is_patched_for_the_testing_layout_or_refused() {
        let files = guard_files(
            Layout::Testing,
            "/usr/local/go/",
            DEPS,
            "b3:abc123",
            "/workspace",
        )
        .unwrap();
        let get = |n: &str| &files.iter().find(|(f, _)| *f == n).unwrap().1;
        let deps = get("testdeps.go");
        assert!(deps.contains("quodAttach()"));
        assert!(!deps.contains("0quod/guard"));
        assert!(!deps.contains("testlog.SetLogger(&log)\n\t\tlog.w.WriteString"));
        assert!(deps.contains("why := quodVerdict()"));
        assert!(deps.contains("var quodNamespace = \"b3:abc123\""));
        assert!(get("testing.go").contains("quodWorkspace = \"/workspace\""));
        let overlay: serde_json::Value = serde_json::from_str(get("overlay.json")).unwrap();
        let r = overlay["Replace"].as_object().unwrap();
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!(
            r["/usr/local/go/src/testing/internal/testdeps/deps.go"],
            "/quod/go-test-guard/v2/testdeps.go"
        );
        assert_eq!(
            r["/usr/local/go/src/testing/quod_test_cache_guard.go"],
            "/quod/go-test-guard/v2/testing.go"
        );
        assert_eq!(
            Layout::Testing.knob().item,
            "-overlay=/quod/go-test-guard/v2/overlay.json"
        );
        let changed = DEPS.replace(
            "defer log.mu.Unlock()\n\terr",
            "defer log.mu.Unlock()\n\n\terr",
        );
        assert!(guard_files(Layout::Testing, "/g", &changed, "b3:abc", "/workspace").is_err());
        assert!(guard_files(Layout::Testing, "/g", DEPS, "b3:a\"b", "/workspace").is_err());
    }

    /// Each layout's key path and knob name its version; the two never
    /// coincide, so a step proved under one is not a hit under the other.
    #[test]
    fn the_layouts_have_distinct_keys() {
        assert_ne!(Layout::Package.sandbox_dir(), Layout::Testing.sandbox_dir());
        for l in LAYOUTS {
            assert!(
                l.knob()
                    .item
                    .ends_with(&format!("{}/overlay.json", l.sandbox_dir()))
            );
        }
        assert_eq!(LAYOUTS[0], Layout::Package);
    }
}
