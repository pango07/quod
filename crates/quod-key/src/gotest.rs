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
//! against two changed files of the toolchain's own standard library,
//! supplied through `go -overlay`: a file added to package testing
//! (`testing_guard.go.txt`) logs from testing's init on and classifies every
//! access, and testing/internal/testdeps (the toolchain's file, patched
//! here) hands go's log to it and writes its verdict into the test log. A
//! result that may depend on anything go test will not recheck gets a line
//! go test cannot parse, and go test caches only results whose log it can
//! parse (cmd/go/internal/test: computeTestInputsID, errBadTestInputs). A
//! cacheable result is one whose every input go test rechecks on the next
//! run, so the manifest of a run that replays it records those inputs.
//!
//! Only packages that import testing compile differently (test binaries and
//! test helpers), so go build and go vet share the rest of the build cache.
//! The guard's version is in the overlay's path, which is in GOFLAGS and so
//! in the step key: a changed guard is a changed key.

use crate::Knob;

/// Where the guard's files are bound in the sandbox, read-only, outside the
/// observed tree. The version is part of the path, so of the key: v2
/// (2026-11-02: the testing file builds on linux only, the overlay names
/// the GOROOT `go env` reports, proofs run with the cache expired) re-keys
/// every step v1 proved.
pub const SANDBOX_DIR: &str = "/quod/go-test-guard/v2";

/// The GOFLAGS setting that compiles test binaries with the guard: the
/// warm Go profile's replacement for `-count=1`.
pub const OVERLAY_KNOB: Knob = Knob {
    name: "GOFLAGS",
    sep: ' ',
    item: "-overlay=/quod/go-test-guard/v2/overlay.json",
};

/// The guard's file in package testing, with the workspace path to fill in.
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
/// overlay names the profile's GOROOT), or turns modules off.
pub fn env_allows_guard(extra_env: &[(String, String)]) -> bool {
    extra_env.iter().all(|(k, v)| match k.as_str() {
        "GOFLAGS" => !v.contains("-overlay") && !v.contains("-count") && !v.contains("-exec"),
        "GOROOT" | "GO111MODULE" | "GOTOOLCHAIN" | "GOENV" => false,
        _ => true,
    })
}

/// The Go releases whose cmd/go and package testing this guard was
/// reviewed against (the test cache's rules, testdeps, the overlay):
/// `go1.26`, `go1.27` and their patch releases. Any other version (a
/// release candidate, an experiment build, the next minor) keeps
/// `-count=1` until the guard is reviewed against it.
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

/// The name the guard's testing file has in GOROOT's src/testing; a `go
/// list` of package testing under the overlay lists it exactly when the
/// overlay applies.
pub const TESTING_FILE: &str = "quod_test_cache_guard.go";

/// The overlay (`go -overlay` JSON) that adds `testing_go` to package
/// testing under `goroot` and replaces testdeps with `testdeps_go`.
pub fn overlay_json(goroot: &str, testdeps_go: &str, testing_go: &str) -> String {
    let root = goroot.trim_end_matches('/');
    let mut replace = serde_json::Map::new();
    replace.insert(format!("{root}/{TESTDEPS}"), testdeps_go.into());
    replace.insert(
        format!("{root}/src/testing/{TESTING_FILE}"),
        testing_go.into(),
    );
    serde_json::json!({ "Replace": replace }).to_string()
}

/// The guard's files, as (name in [`SANDBOX_DIR`], contents): the overlay
/// JSON naming both replaced files under `goroot`, spelled exactly as the
/// step's go command spells GOROOT (`go env GOROOT`: cmd/go matches overlay
/// paths by spelling, not by file), the patched testdeps (from `deps_go`,
/// the toolchain's own), and the testing file. `namespace` (the step key)
/// goes into the testdeps file, so test binaries, and with them cached
/// results, of different keys never coincide. `Err` when testdeps is not
/// the file this patch knows; the step then keeps `-count=1`.
pub fn guard_files(
    goroot: &str,
    deps_go: &str,
    namespace: &str,
    workspace: &str,
) -> Result<Vec<(&'static str, String)>, String> {
    let testdeps = patch_testdeps(deps_go, namespace)?;
    let testing = TESTING_GUARD.replace("QUOD_WORKSPACE", workspace);
    let overlay = overlay_json(
        goroot,
        &format!("{SANDBOX_DIR}/testdeps.go"),
        &format!("{SANDBOX_DIR}/testing.go"),
    );
    Ok(vec![
        ("overlay.json", overlay),
        ("testdeps.go", testdeps),
        ("testing.go", testing),
    ])
}

/// The two edits to testdeps, each found exactly once.
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
		quodAttach()
	}
}";

const STOP_OLD: &str = "func (TestDeps) StopTestLog() error {
	log.mu.Lock()
	defer log.mu.Unlock()
	err := log.w.Flush()";

const STOP_NEW: &str = "func (TestDeps) StopTestLog() error {
	why := quodVerdict()
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

const TESTDEPS_TAIL: &str = "
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

fn patch_testdeps(deps_go: &str, namespace: &str) -> Result<String, String> {
    if !namespace
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b':')
    {
        return Err(format!("namespace {namespace:?} is not a step key"));
    }
    let mut s = deps_go.to_string();
    for (old, new) in [(START_OLD, START_NEW), (STOP_OLD, STOP_NEW)] {
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
    s.push_str(&TESTDEPS_TAIL.replace("QUOD_NAMESPACE", namespace));
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// testdeps as Go 1.27 ships it, the parts the patch reads.
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
    fn a_script_that_sets_the_toolchain_or_names_a_go_is_seen() {
        for s in [
            "GOFLAGS=-count=1 go test ./...",
            "export PATH=/opt/go/bin:$PATH && go test ./...",
            "PATH+=:/x go test ./...",
            "env GOROOT=/opt/go go test ./...",
            "GOTOOLCHAIN=go1.28.0 go test ./...",
            "go env -w GOFLAGS=-count=1; go test ./...",
            "\"GOFLAGS=-x\" go test ./...",
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
    fn the_env_may_not_bring_its_own_overlay_count_or_goroot() {
        let e = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        assert!(env_allows_guard(&[]));
        assert!(env_allows_guard(&e("GOFLAGS", "-mod=mod -race")));
        assert!(!env_allows_guard(&e("GOFLAGS", "-count=1")));
        assert!(!env_allows_guard(&e("GOFLAGS", "-overlay=/x.json")));
        assert!(!env_allows_guard(&e("GOROOT", "/opt/go")));
        assert!(!env_allows_guard(&e("GO111MODULE", "off")));
    }

    /// The testing file builds for linux only: it calls getrusage and
    /// waitid, which other GOOSes' syscall lacks, and the overlay applies
    /// to every build in the step (`GOOS=windows go build`, MAJOR 3).
    #[test]
    fn the_testing_file_is_linux_only() {
        let first = TESTING_GUARD.lines().next().unwrap();
        assert_eq!(first, "//go:build linux");
        // A constraint must be followed by a blank line, before the package
        // clause.
        assert_eq!(TESTING_GUARD.lines().nth(1), Some(""));
        assert!(TESTING_GUARD.contains("\npackage testing\n"));
    }

    #[test]
    fn testdeps_is_patched_or_refused() {
        let files = guard_files("/usr/local/go/", DEPS, "b3:abc123", "/workspace").unwrap();
        let get = |n: &str| &files.iter().find(|(f, _)| *f == n).unwrap().1;
        let deps = get("testdeps.go");
        assert!(deps.contains("quodAttach()"));
        assert!(!deps.contains("testlog.SetLogger(&log)\n\t\tlog.w.WriteString"));
        assert!(deps.contains("why := quodVerdict()"));
        assert!(deps.contains("var quodNamespace = \"b3:abc123\""));
        assert!(get("testing.go").contains("quodWorkspace = \"/workspace\""));
        let overlay: serde_json::Value = serde_json::from_str(get("overlay.json")).unwrap();
        let r = overlay["Replace"].as_object().unwrap();
        // One spelling of GOROOT: the one the go command reports.
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!(
            r["/usr/local/go/src/testing/internal/testdeps/deps.go"],
            "/quod/go-test-guard/v2/testdeps.go"
        );
        assert_eq!(
            r["/usr/local/go/src/testing/quod_test_cache_guard.go"],
            "/quod/go-test-guard/v2/testing.go"
        );
        assert!(
            OVERLAY_KNOB
                .item
                .ends_with(&format!("{SANDBOX_DIR}/overlay.json"))
        );
        // Another testdeps: refused, never half-patched.
        let changed = DEPS.replace(
            "defer log.mu.Unlock()\n\terr",
            "defer log.mu.Unlock()\n\n\terr",
        );
        assert!(guard_files("/g", &changed, "b3:abc", "/workspace").is_err());
        assert!(guard_files("/g", DEPS, "b3:a\"b", "/workspace").is_err());
    }
}
