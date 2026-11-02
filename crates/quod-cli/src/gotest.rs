//! Go's test-result cache in a warm step, under Quod's guard
//! (`quod_key::gotest` has the why). This half finds the toolchain the step
//! will run, makes the guard's files from that toolchain's own testdeps, and
//! gives an execution that must not trust go test's cache (an audit, a
//! re-proof after a quarantine) an expired one.

use std::path::{Path, PathBuf};

use anyhow::Context;
use quod_cas::Hash;
use quod_key::gotest;

/// The toolchain's testdeps, as the step's go command will find it.
#[derive(Clone, Debug)]
pub(crate) struct GoTestGuard {
    /// GOROOT as the go command may spell it: the PATH entry's parent and
    /// its real path (cmd/go finds GOROOT from its own executable).
    goroots: Vec<String>,
    deps_go: String,
}

impl GoTestGuard {
    /// The guard for a warm step on the host root, when it applies: the
    /// script runs `go test`, the env leaves the toolchain alone, and the
    /// go first on the step's PATH has a testdeps the guard knows how to
    /// change. Otherwise None, and the step keeps `-count=1`.
    pub(crate) fn for_step(
        script: &str,
        extra_env: &[(String, String)],
        profile_env: &[(&str, &str)],
    ) -> Option<GoTestGuard> {
        if !gotest::runs_go_test(script) || !gotest::env_allows_guard(extra_env) {
            return None;
        }
        let path = extra_env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.as_str())
            .or_else(|| {
                profile_env
                    .iter()
                    .find(|(k, _)| *k == "PATH")
                    .map(|(_, v)| *v)
            })?;
        let go = path
            .split(':')
            .map(|d| Path::new(d).join("go"))
            .find(|p| p.is_file())?;
        let real = go.canonicalize().ok()?;
        let mut goroots = Vec::new();
        for g in [&go, &real] {
            let root = g.parent()?.parent()?.to_str()?.to_string();
            if !goroots.contains(&root) {
                goroots.push(root);
            }
        }
        let deps_go =
            std::fs::read_to_string(real.parent()?.parent()?.join(gotest::TESTDEPS)).ok()?;
        let guard = GoTestGuard { goroots, deps_go };
        // A testdeps the patch does not know keeps the step on -count=1.
        match gotest::guard_files(&guard.goroots, &guard.deps_go, "b3:0", quod_key::WORKSPACE) {
            Ok(_) => Some(guard),
            Err(why) => {
                tracing::info!(target: "quod", %why, "go test cache guard unavailable; -count=1");
                None
            }
        }
    }

    /// Writes the guard's files for the step `key` into `dir` (bound at
    /// `gotest::SANDBOX_DIR`).
    pub(crate) fn write(&self, dir: &Path, key: &Hash) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let files = gotest::guard_files(
            &self.goroots,
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

/// What a warm Go step's execution does about go test's result cache.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GoTests<'a> {
    /// The guard's files go in the sandbox (the key says so: GOFLAGS).
    pub(crate) guard: Option<&'a GoTestGuard>,
    /// go test may not serve a result from its cache in this execution: an
    /// audit or a re-proof checks what the tests do now. Results it stores
    /// stay usable (they are fresh).
    pub(crate) expire: bool,
}

/// A `testexpire.txt` (as `go clean -testcache` writes it) that expires
/// every cached test result older than now, in `dir`; bound over the warm
/// cache's own for one execution, so the shared cache is untouched. The
/// warm cache gets an empty one as the mount point if it has none, which go
/// test ignores.
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
}
