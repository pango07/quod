//! Go's test-result cache in a warm step, under Quod's guard
//! (`quod_key::gotest` has the why). This half asks the toolchain the step
//! will run where its GOROOT is, checks which of the guard's layouts its
//! overlay applies in (the guard's own package where cmd/go finds it
//! through the overlay, else the file in package testing), makes the
//! guard's files from that toolchain's own testdeps, and gives an
//! execution that must not trust go test's cache (an audit, a proof, a
//! re-proof after a quarantine) an expired one.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use anyhow::Context;
use quod_cas::Hash;
use quod_key::gotest::{self, Layout};
use quod_key::{Knob, Profile};

/// The toolchain's testdeps, as the step's go command will find it, and
/// the layout its overlay was seen to apply in.
#[derive(Clone, Debug)]
pub(crate) struct GoTestGuard {
    /// GOROOT exactly as the step's go command spells it (`go env GOROOT`):
    /// cmd/go matches overlay paths by spelling, and spells GOROOT as its
    /// own default when that names the same directory, which no path
    /// arithmetic on PATH can tell.
    goroot: String,
    deps_go: String,
    layout: Layout,
}

/// Why the guard does not apply to a step (it keeps `-count=1`).
type Refusal = String;

impl GoTestGuard {
    /// The guard for a warm step on the host root, when it applies: the
    /// script runs `go test` and sets none of the toolchain's variables,
    /// the env leaves the toolchain alone, the go first on the step's PATH
    /// is a reviewed release (`gotest::REVIEWED_GO`) whose testdeps the
    /// guard knows how to change, and its go command was seen to apply the
    /// guard's overlay, in one of its layouts, at the GOROOT it reports.
    /// Otherwise None, and the step keeps `-count=1`. `cache` is where
    /// verified toolchains are remembered, by the go binary's identity.
    pub(crate) fn for_step(
        script: &str,
        extra_env: &[(String, String)],
        profile: &Profile,
        cache: &Path,
    ) -> Option<GoTestGuard> {
        let knobs = profile.warm.as_ref().map_or(&[][..], |w| w.guarded_knobs);
        Self::for_step_in(script, extra_env, profile.env, knobs, cache)
    }

    /// `for_step` with the profile's env and guarded knobs given apart.
    fn for_step_in(
        script: &str,
        extra_env: &[(String, String)],
        profile_env: &[(&str, &str)],
        guarded_knobs: &[Knob],
        cache: &Path,
    ) -> Option<GoTestGuard> {
        if !gotest::runs_go_test(script) || !gotest::env_allows_guard(extra_env) {
            return None;
        }
        match Self::probe(script, extra_env, profile_env, guarded_knobs, cache) {
            Ok(g) => {
                tracing::info!(target: "quod", layout = g.layout.name(), "go test cache guard");
                Some(g)
            }
            Err(why) => {
                tracing::info!(target: "quod", %why, "go test cache guard unavailable; -count=1");
                None
            }
        }
    }

    fn probe(
        script: &str,
        extra_env: &[(String, String)],
        profile_env: &[(&str, &str)],
        guarded_knobs: &[Knob],
        cache: &Path,
    ) -> Result<GoTestGuard, Refusal> {
        if let Some(w) = gotest::script_sets_toolchain_env(script) {
            return Err(format!("the script sets {w}"));
        }
        // The step's env, as StepSpec::new_warm_guarded merges it: the
        // profile's, the step's own on top, then the guarded knobs (GODEBUG
        // goindex=0: whether cmd/go finds a package through the overlay
        // depends on it).
        let mut env: Vec<(String, String)> = profile_env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        for (k, v) in extra_env {
            env.retain(|(n, _)| n != k);
            env.push((k.clone(), v.clone()));
        }
        for k in guarded_knobs {
            quod_key::merge_knob(&mut env, k).map_err(|e| e.to_string())?;
        }
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .ok_or("the step has no PATH")?;
        // The go the step's shell finds (run by that name: cmd/go finds
        // GOROOT from its own executable, so a symlink on PATH is followed).
        let go = path
            .split(':')
            .map(|d| Path::new(d).join("go"))
            .find(|p| p.is_file())
            .ok_or_else(|| format!("no go on PATH {path}"))?;
        let real = go
            .canonicalize()
            .map_err(|e| format!("{}: {e}", go.display()))?;
        let meta = std::fs::metadata(&real).map_err(|e| format!("{}: {e}", real.display()))?;
        let scratch = TempDir::new(cache)?;
        // Where the go command puts its caches, for these runs only: the
        // step's own (under /workspace) are not paths on this host.
        let mut probe_env = env.clone();
        for (k, v) in [
            ("GOCACHE", scratch.0.join("gocache")),
            ("GOMODCACHE", scratch.0.join("gomod")),
            ("GOPATH", scratch.0.join("gopath")),
            ("HOME", scratch.0.join("home")),
        ] {
            probe_env.retain(|(n, _)| n != k);
            probe_env.push((k.into(), v.to_string_lossy().into_owned()));
        }
        let out = go_cmd(&go, &probe_env, &scratch.0)
            .args(["env", "GOROOT", "GOVERSION"])
            .output()
            .map_err(|e| format!("go env: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "go env failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut lines = text.lines();
        let (Some(goroot), Some(version), None) = (lines.next(), lines.next(), lines.next()) else {
            return Err(format!("go env printed {text:?}"));
        };
        if !Path::new(goroot).is_absolute() {
            return Err(format!("go env GOROOT is {goroot:?}"));
        }
        if !gotest::reviewed_go_version(version) {
            return Err(format!(
                "{version} is not a release the guard was reviewed against ({})",
                gotest::REVIEWED_GO.join(", ")
            ));
        }
        // A go the script names by path must be this toolchain.
        let root_real = Path::new(goroot)
            .canonicalize()
            .map_err(|e| format!("GOROOT {goroot}: {e}"))?;
        for p in gotest::script_go_paths(script) {
            let theirs = if Path::new(p).is_absolute() {
                Path::new(p)
                    .canonicalize()
                    .ok()
                    .and_then(|r| r.parent()?.parent()?.canonicalize().ok())
            } else {
                None
            };
            if theirs.as_deref() != Some(root_real.as_path()) {
                return Err(format!("the script runs {p}, not {goroot}/bin/go"));
            }
        }
        let deps_go = std::fs::read_to_string(Path::new(goroot).join(gotest::TESTDEPS))
            .map_err(|e| format!("{goroot}/{}: {e}", gotest::TESTDEPS))?;
        // Seen to apply: remembered by the binary's identity, its GOROOT
        // spelling, the version, the guard's layouts and the knobs the
        // probe ran with (goindex decides whether the package resolves).
        let id = Hash::of(
            format!(
                "{}\0{}\0{}\0{}\0{}\0{:?}\0{:?}\0{goroot}\0{version}\0{}\0{}",
                real.display(),
                meta.dev(),
                meta.ino(),
                meta.size(),
                meta.mode(),
                (meta.mtime(), meta.mtime_nsec()),
                (meta.ctime(), meta.ctime_nsec()),
                gotest::LAYOUTS
                    .iter()
                    .map(|l| l.sandbox_dir())
                    .collect::<Vec<_>>()
                    .join(","),
                guarded_knobs
                    .iter()
                    .map(|k| format!("{}{}{}", k.name, k.sep, k.item))
                    .collect::<Vec<_>>()
                    .join(","),
            )
            .as_bytes(),
        );
        let seen = cache.join(id.hex());
        let guard = |layout| GoTestGuard {
            goroot: goroot.to_string(),
            deps_go: deps_go.clone(),
            layout,
        };
        if let Ok(b) = std::fs::read(&seen)
            && let Some(l) = gotest::LAYOUTS.into_iter().find(|l| b == verified(*l))
        {
            // A testdeps the patch does not know keeps the step on
            // -count=1 (checked here too: the file is read afresh).
            gotest::guard_files(l, goroot, &deps_go, "b3:0", quod_key::WORKSPACE)?;
            return Ok(guard(l));
        }
        let started = SystemTime::now();
        let mut refusals = Vec::new();
        for layout in gotest::LAYOUTS {
            match check_layout(layout, &go, &probe_env, &scratch.0, goroot, &deps_go) {
                Ok(()) => {
                    // Kept only for a binary that had settled before the
                    // check (a binary rewritten within the clock's tick
                    // could keep its identity).
                    let settled = |s: i64, ns: i64| {
                        u64::try_from(s).is_ok_and(|s| {
                            SystemTime::UNIX_EPOCH
                                + Duration::new(s, ns.clamp(0, 999_999_999) as u32)
                                + Duration::from_secs(2)
                                < started
                        })
                    };
                    if settled(meta.mtime(), meta.mtime_nsec())
                        && settled(meta.ctime(), meta.ctime_nsec())
                    {
                        let tmp = scratch.0.join("verified");
                        if std::fs::write(&tmp, verified(layout)).is_ok() {
                            let _ = std::fs::rename(&tmp, &seen);
                        }
                    }
                    return Ok(guard(layout));
                }
                Err(why) => refusals.push(format!("{} layout: {why}", layout.name())),
            }
        }
        Err(refusals.join("; "))
    }

    /// The layout the guard's files take.
    pub(crate) fn layout(&self) -> Layout {
        self.layout
    }

    /// The GOROOT spelling the overlay names.
    #[cfg(test)]
    pub(crate) fn goroot(&self) -> &str {
        &self.goroot
    }

    /// Writes the guard's files for the step `key` into `dir` (bound at
    /// the layout's sandbox dir).
    pub(crate) fn write(&self, dir: &Path, key: &Hash) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let files = gotest::guard_files(
            self.layout,
            &self.goroot,
            &self.deps_go,
            &key.to_string(),
            quod_key::WORKSPACE,
        )
        .map_err(|e| anyhow::anyhow!("go test cache guard: {e}"))?;
        for (name, body) in files {
            let p = dir.join(name);
            std::fs::write(&p, body).with_context(|| format!("write {}", p.display()))?;
        }
        Ok(())
    }
}

/// Whether `layout`'s overlay, keyed by `goroot`, applies under this go
/// command: the guard's files are written to `scratch`, the overlay names
/// them under `goroot`, and a `go list` under it must show the guard where
/// the layout puts it. For the package layout, cmd/go must find the
/// package through the overlay (`go list 0quod/guard` lists the guard's
/// file; a cmd/go that reads GOROOT from disk with the module index off
/// says it is not in std), and the package's ordering argument must hold
/// against this toolchain's package os: every import of the guard is os
/// or a dependency of os (`go list -deps os`). For the testing layout, a
/// `go list` of package testing lists the added file.
fn check_layout(
    layout: Layout,
    go: &Path,
    env: &[(String, String)],
    scratch: &Path,
    goroot: &str,
    deps_go: &str,
) -> Result<(), Refusal> {
    let dir = scratch.join(layout.name());
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let files = gotest::guard_files(layout, goroot, deps_go, "b3:0", quod_key::WORKSPACE)?;
    let mut written = std::collections::HashMap::new();
    for (name, body) in &files {
        let p = dir.join(name);
        std::fs::write(&p, body).map_err(|e| format!("write {}: {e}", p.display()))?;
        written.insert(*name, p.to_string_lossy().into_owned());
    }
    let (pkg, wanted, guard_files): (&str, &str, Vec<&str>) = match layout {
        Layout::Package => (
            gotest::GUARD_PKG,
            gotest::GUARD_FILE,
            vec![&written["guard.go"], &written["unsupported.go"]],
        ),
        Layout::Testing => (
            "testing",
            gotest::TESTING_FILE,
            vec![&written["testing.go"]],
        ),
    };
    let overlay = dir.join("overlay.json");
    std::fs::write(
        &overlay,
        gotest::overlay_json(layout, goroot, &written["testdeps.go"], &guard_files),
    )
    .map_err(|e| format!("write {}: {e}", overlay.display()))?;
    let list = |args: &[&str]| -> Result<String, Refusal> {
        let out = go_cmd(go, env, scratch)
            .arg("list")
            .arg(format!("-overlay={}", overlay.display()))
            .args(args)
            .output()
            .map_err(|e| format!("go list: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() {
            return Err(format!(
                "go list {} failed: {stdout} {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(stdout)
    };
    let listed = list(&["-f", "{{.GoFiles}}", pkg])?;
    if !listed
        .trim_matches(['[', ']'])
        .split(' ')
        .any(|f| f == wanted)
    {
        return Err(format!(
            "the overlay at GOROOT {goroot} does not apply: go list {pkg} gave {listed:?}"
        ));
    }
    if layout == Layout::Package {
        let deps = list(&["-deps", "-f", "{{.ImportPath}}", "os"])?;
        let deps: Vec<&str> = deps.lines().collect();
        let guard_src = &files.iter().find(|(n, _)| *n == "guard.go").unwrap().1;
        for i in gotest::go_imports(guard_src) {
            if !deps.contains(&i) {
                return Err(format!(
                    "the guard imports {i}, which is not a dependency of this toolchain's os"
                ));
            }
        }
    }
    Ok(())
}

/// What a verified toolchain's entry in the cache holds for `layout`;
/// anything else is no entry.
fn verified(layout: Layout) -> Vec<u8> {
    format!("quod go test guard: overlay applies ({})\n", layout.name()).into_bytes()
}

/// `go` with exactly `env`, in `dir`.
fn go_cmd(go: &Path, env: &[(String, String)], dir: &Path) -> Command {
    let mut c = Command::new(go);
    c.env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(dir)
        .stdin(std::process::Stdio::null());
    c
}

/// A scratch directory under the guard's cache, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(cache: &Path) -> Result<TempDir, Refusal> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = cache.join(format!(".probe.{}.{n}", std::process::id()));
        let _ = quod_cas::remove_any(&d);
        std::fs::create_dir_all(&d).map_err(|e| format!("create {}: {e}", d.display()))?;
        Ok(TempDir(d))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = quod_cas::remove_any(&self.0);
    }
}

/// What a warm Go step's execution does about go test's result cache.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GoTests<'a> {
    /// The guard's files go in the sandbox (the key says so: GOFLAGS).
    pub(crate) guard: Option<&'a GoTestGuard>,
    /// go test may not serve a result from its cache in this execution: a
    /// proof, an audit or a re-proof checks what the tests do now (a result
    /// go test serves was computed by an earlier execution, under inputs it
    /// may not recheck). Results it stores stay usable (they are fresh).
    pub(crate) expire: bool,
}

/// A `testexpire.txt` (as `go clean -testcache` writes it) that expires
/// every cached test result older than now, in `dir`; bound over the warm
/// cache's own for one execution, so the shared cache is untouched. The
/// bind is writable: the file is this run's own copy, and the step's own
/// `go clean -testcache` edits it in place (lockedfile.Edit), which a
/// read-only bind would fail with EROFS, in audits and proofs only. The
/// warm cache gets an empty one as the mount point if it has none, which
/// go test ignores.
pub(crate) fn expired_tests(dir: &Path, warm: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("clock before 1970")?
        .as_nanos();
    let src = dir.join("testexpire.txt");
    std::fs::write(&src, format!("{now}\n")).with_context(|| format!("write {}", src.display()))?;
    let point = warm.join("testexpire.txt");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&point)
        .with_context(|| format!("create {}", point.display()))?;
    Ok((src, point))
}

/// A path in a manifest that is another Go toolchain's testdeps: a GOROOT
/// inside the workspace (an SDK under HOME), whose go test the guard does
/// not reach, so its result cache was unguarded.
pub(crate) fn unguarded_toolchain(
    paths: impl IntoIterator<Item = impl AsRef<str>>,
) -> Option<String> {
    paths
        .into_iter()
        .find(|p| {
            let p = p.as_ref();
            p == gotest::TESTDEPS || p.ends_with(&format!("/{}", gotest::TESTDEPS))
        })
        .map(|p| p.as_ref().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_toolchain_in_the_workspace_is_found() {
        assert_eq!(unguarded_toolchain(["go.mod", "pkg/a.go"]), None);
        assert_eq!(
            unguarded_toolchain([".quod/home/sdk/go1.22/src/testing/internal/testdeps/deps.go"]),
            Some(".quod/home/sdk/go1.22/src/testing/internal/testdeps/deps.go".into())
        );
    }

    #[test]
    fn an_expired_test_cache_is_now_and_leaves_the_cache_alone() {
        let d = tempfile::tempdir().unwrap();
        let warm = d.path().join("warm");
        std::fs::create_dir_all(&warm).unwrap();
        std::fs::write(warm.join("testexpire.txt"), "5\n").unwrap();
        let (src, point) = expired_tests(d.path(), &warm).unwrap();
        let t: u128 = std::fs::read_to_string(&src)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(t > 1_700_000_000_000_000_000);
        assert_eq!(std::fs::read_to_string(point).unwrap(), "5\n");
    }

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

    /// What the fake toolchain's cmd/go does with the overlay.
    #[derive(Clone, Copy)]
    enum Fake {
        /// go1.27: the guard's package is found through the overlay with
        /// the module index off.
        Package,
        /// go1.26: with the module index off (`goindex=0`, Quod's profile)
        /// GOROOT is read from disk and the package is not in std; with it
        /// on, the package is found. The file in package testing applies.
        Testing,
        /// A package os that lacks one of the guard's imports: the
        /// ordering argument fails, the testing layout applies.
        OsWithoutPath,
        /// Neither overlay applies.
        None,
    }

    /// A toolchain whose `go` is a script: its GOROOT is `<tmp>/real`, and
    /// `go env GOROOT` spells it `<tmp>/alias` (a symlink to it), as cmd/go
    /// does when its default GOROOT names the same directory. Its `go
    /// list` finds the guard only for an overlay keyed by that spelling,
    /// as cmd/go's does, and behaves as `fake` says.
    fn toolchain(version: &str, fake: Fake) -> (tempfile::TempDir, String) {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real");
        let alias = d.path().join("alias");
        std::fs::create_dir_all(real.join("bin")).unwrap();
        let deps = real.join(gotest::TESTDEPS);
        std::fs::create_dir_all(deps.parent().unwrap()).unwrap();
        std::fs::write(&deps, DEPS).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let testing_key = format!("{}/src/testing/{}", alias.display(), gotest::TESTING_FILE);
        let package_key = format!(
            "{}/src/{}/{}",
            alias.display(),
            gotest::GUARD_PKG,
            gotest::GUARD_FILE
        );
        let os_deps = match fake {
            Fake::OsWithoutPath => gotest::GUARD_IMPORTS
                .iter()
                .filter(|i| **i != "path")
                .copied()
                .collect::<Vec<_>>()
                .join("\\n"),
            _ => gotest::GUARD_IMPORTS.join("\\n"),
        };
        // `$ov` is the overlay file; `$last` the package named.
        let package_case = match fake {
            Fake::Package | Fake::OsWithoutPath => format!(
                "grep -qF '\"{package_key}\"' \"$ov\" && echo '[{}]' || {{ echo \"package {} is not in std\" >&2; exit 1; }}",
                gotest::GUARD_FILE,
                gotest::GUARD_PKG
            ),
            Fake::Testing => format!(
                "case \"$GODEBUG\" in *goindex=0*) echo \"package {} is not in std\" >&2; exit 1;; esac; grep -qF '\"{package_key}\"' \"$ov\" && echo '[{}]' || {{ echo \"package {} is not in std\" >&2; exit 1; }}",
                gotest::GUARD_PKG,
                gotest::GUARD_FILE,
                gotest::GUARD_PKG
            ),
            Fake::None => format!(
                "echo \"package {} is not in std\" >&2; exit 1",
                gotest::GUARD_PKG
            ),
        };
        let testing_case = match fake {
            Fake::None => "echo '[testing.go]'".to_string(),
            _ => format!(
                "grep -qF '\"{testing_key}\"' \"$ov\" && echo '[testing.go {}]' || echo '[testing.go]'",
                gotest::TESTING_FILE
            ),
        };
        let list = format!(
            "ov=; for a; do case \"$a\" in -overlay=*) ov=${{a#-overlay=}};; esac; last=$a; done; case \"$last\" in {}) {package_case};; os) printf '{os_deps}\\n';; testing) {testing_case};; *) exit 2;; esac",
            gotest::GUARD_PKG
        );
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\nenv) printf '%s\\n%s\\n' '{}' '{version}' ;;\nlist) {list} ;;\n*) exit 2 ;;\nesac\n",
            alias.display()
        );
        let go = real.join("bin/go");
        std::fs::write(&go, script).unwrap();
        std::fs::set_permissions(&go, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", real.join("bin").display());
        (d, path)
    }

    /// The guard for `script` under the Go profile's guarded knobs
    /// (GODEBUG goindex=0, which the fake toolchain's `go list` reads), with
    /// the fake toolchain's bin first on PATH.
    fn guard(script: &str, path: &str, cache: &Path) -> Option<GoTestGuard> {
        let go = &quod_key::profile::GO;
        let knobs = go.warm.as_ref().map_or(&[][..], |w| w.guarded_knobs);
        assert!(knobs.iter().any(|k| k.item == "goindex=0"));
        GoTestGuard::for_step_in(
            script,
            &[],
            &[("PATH", path), ("GOTOOLCHAIN", "local")],
            knobs,
            cache,
        )
    }

    fn overlay_keys(g: &GoTestGuard, dir: &Path) -> Vec<String> {
        g.write(dir, &Hash::of(b"key")).unwrap();
        let overlay: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("overlay.json")).unwrap())
                .unwrap();
        overlay["Replace"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    /// MAJOR 1: the overlay names GOROOT as `go env` spells it, the one
    /// spelling cmd/go matches, which is neither the PATH entry's parent nor
    /// the binary's real path here; and the guard is used only once a `go
    /// list` under that overlay finds it. On a toolchain that finds the
    /// guard's package through the overlay, the package layout is used.
    #[test]
    fn the_overlay_names_the_goroot_go_reports_and_is_seen_to_apply() {
        let (d, path) = toolchain("go1.27.3", Fake::Package);
        let cache = d.path().join("cache");
        let g = guard("go test ./...", &path, &cache).expect("the guard applies");
        let alias = d.path().join("alias");
        assert_eq!(g.goroot(), alias.to_str().unwrap());
        assert_eq!(g.layout(), Layout::Package);
        let keys = overlay_keys(&g, &d.path().join("files"));
        assert_eq!(keys.len(), 3, "{keys:?}");
        assert!(
            keys.iter()
                .all(|k| k.starts_with(&format!("{}/", alias.display()))),
            "{keys:?}"
        );
        // Remembered: the second probe reads the cache, same layout.
        let g2 = guard("go test ./...", &path, &cache).unwrap();
        assert_eq!(g2.layout(), Layout::Package);
        // A go the script names by path is the same toolchain, or no guard.
        let named = d.path().join("real/bin/go");
        assert!(guard(&format!("{} test ./...", named.display()), &path, &cache).is_some());
        assert!(guard("/opt/other/bin/go test ./...", &path, &cache).is_none());
        assert!(guard("./bin/go test ./...", &path, &cache).is_none());
        // The script may not set the toolchain's variables (MINOR 4).
        assert!(guard("GOFLAGS=-count=1 go test ./...", &path, &cache).is_none());
        assert!(guard("export PATH=/x:$PATH; go test ./...", &path, &cache).is_none());
        assert!(guard("GODEBUG=goindex=1 go test ./...", &path, &cache).is_none());
        // Nothing left of the probes.
        let left: Vec<_> = std::fs::read_dir(&cache)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".probe"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    /// A cmd/go that reads GOROOT from disk with the module index off
    /// (go1.26) cannot find the guard's package: the probe, which runs
    /// with the profile's `goindex=0` as the step will, falls back to the
    /// file in package testing, under that layout's own (v2) key.
    #[test]
    fn a_toolchain_that_reads_goroot_from_disk_keeps_the_testing_layout() {
        let (d, path) = toolchain("go1.26.3", Fake::Testing);
        let cache = d.path().join("cache");
        let g = guard("go test ./...", &path, &cache).expect("the testing layout applies");
        assert_eq!(g.layout(), Layout::Testing);
        let keys = overlay_keys(&g, &d.path().join("files"));
        assert_eq!(keys.len(), 2, "{keys:?}");
        assert!(keys.iter().any(|k| k.ends_with(gotest::TESTING_FILE)));
        assert_eq!(
            guard("go test ./...", &path, &cache).unwrap().layout(),
            Layout::Testing,
            "remembered"
        );
    }

    /// The package layout's ordering argument is checked against the
    /// toolchain: a guard import that is not a dependency of its os
    /// refuses the layout.
    #[test]
    fn the_package_layout_needs_every_guard_import_to_be_a_dependency_of_os() {
        let (d, path) = toolchain("go1.27.3", Fake::OsWithoutPath);
        let g = guard("go test ./...", &path, &d.path().join("cache")).unwrap();
        assert_eq!(g.layout(), Layout::Testing);
    }

    /// Fails closed: an overlay that does not apply in either layout, or a
    /// release the guard was not reviewed against (MINOR 6), keeps
    /// `-count=1`.
    #[test]
    fn no_guard_unless_seen_to_apply_on_a_reviewed_release() {
        let (d, path) = toolchain("go1.27.3", Fake::None);
        assert!(guard("go test ./...", &path, &d.path().join("cache")).is_none());
        for v in ["go1.28.0", "go1.25.4", "go1.27rc2"] {
            let (d, path) = toolchain(v, Fake::Package);
            assert!(
                guard("go test ./...", &path, &d.path().join("cache")).is_none(),
                "{v}"
            );
        }
        let (d, path) = toolchain("go1.26.2", Fake::Package);
        assert_eq!(
            guard("go test ./...", &path, &d.path().join("cache"))
                .unwrap()
                .layout(),
            Layout::Package
        );
    }
}
