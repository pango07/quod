//! Dependency trees live in the state dir, not the checkout.
//!
//! A step's env puts Quod's own directories under
//! `/workspace/.quod/`: the toolchain profile's dependency trees (Go's
//! module cache, Python's venv) and its scratch (a build cache, HOME). A
//! network step (step A, the dependency install) used to fill them in the
//! user's checkout. Now that whole directory lives at
//! `<state>/deps/<repo>/<image>/<toolchain>` (or under QUOD_DEPS_DIR):
//! per repo (its root commit, so clones and copies share it), per pin (the
//! image) and per profile. A network step gets it bound at
//! `/workspace/.quod` with its env unchanged, so a tree whose files
//! name their own path (a venv's scripts) is built where it is used, and
//! the profile's download caches are bound on top as before. An observed
//! step gets the profile's dependency trees (`seed_dirs` under
//! `.quod/`) grafted into its tree at those paths, so the observer, the
//! sealed snapshots, the manifest and warm state see exactly what they saw
//! when the trees sat in the repo; the scratch never is. A dependency tree
//! the tool keeps at the repo's own top level (Node's `node_modules`) is
//! the repo's, and stays there.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use quod_cas::{Cas, Hash, Tree};
use quod_key::{Profile, Toolchain};
use quod_sandbox::Mount;

use crate::state::State;

/// Quod's directory at the top of the workspace.
const DOT: &str = ".quod";

/// The `.quod` directory of one repo under one pin and profile.
pub struct Deps {
    pub root: PathBuf,
}

impl Deps {
    /// `<state>/deps/<repo>/<image>/<toolchain>`, or under QUOD_DEPS_DIR
    /// when set and not empty (a cache shared by state dirs: the acceptance
    /// suite's temp states, a CI box's persistent disk).
    pub fn new(state: &State, repo_id: &str, image: &Hash, toolchain: Toolchain) -> Deps {
        let base = std::env::var_os("QUOD_DEPS_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| state.dir.join("deps"));
        Deps {
            root: base.join(repo_id).join(image.hex()).join(toolchain.name()),
        }
    }

    pub fn dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

/// The repo's identity across clones and copies: `git:<root commit>`, or
/// `path:<hash of the canonical path>` outside a git history.
pub fn repo_id(repo: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--max-parents=0", "HEAD"])
        .output();
    if let Ok(out) = out
        && out.status.success()
    {
        let mut roots: Vec<&str> = std::str::from_utf8(&out.stdout)
            .unwrap_or("")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        roots.sort_unstable();
        if let Some(r) = roots.first() {
            return format!("git:{r}");
        }
    }
    format!(
        "path:{}",
        &Hash::of(repo.to_string_lossy().as_bytes()).hex()[..16]
    )
}

/// The git toplevel containing `from`, if any.
pub fn toplevel(from: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(from)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// The profile's dependency trees kept under `.quod/` (its seed
/// dirs there), as names: what an observed step is served from the state
/// dir. Scratch (`ephemeral`) and download caches (`setup_cache`) are not.
fn served(profile: &Profile) -> Vec<&'static str> {
    profile
        .seed_dirs
        .iter()
        .filter_map(|d| d.strip_prefix(".quod/"))
        .filter(|n| !n.is_empty() && !n.contains('/'))
        .collect()
}

/// The tree an observed step sees: the checkout without `.git` and without
/// its `.quod/` (Quod's own, now kept in the state dir), plus each
/// of the profile's dependency trees grafted in from the state dir at
/// `.quod/<name>`. A tree an older flow left in the checkout is
/// adopted first.
pub fn import_workspace(
    cas: &Cas,
    repo: &Path,
    deps: &Deps,
    profile: &Profile,
) -> anyhow::Result<Tree> {
    // The checkout's .git (served separately, gitdir) and Quod's own
    // directory are not walked or hashed at all. Both imports read again
    // only the files that changed since the last (quod_cas's stat cache).
    let mut tree = cas
        .import_dir_cached(repo, &[".git", DOT])
        .context("import workspace")?;
    for name in served(profile) {
        let dir = deps.dir(name);
        adopt_legacy(&repo.join(DOT).join(name), &dir)?;
        if dir.is_dir() {
            let sub = cas
                .import_dir_cached(&dir, &[])
                .with_context(|| format!("import {} from {}", name, dir.display()))?;
            tree = tree.merged_at(&format!("{DOT}/{name}"), &sub);
        }
    }
    Ok(tree)
}

/// A dependency tree an older flow left in the checkout is adopted once:
/// moved (or, across filesystems, copied and removed) into its new place,
/// so step A need not fetch it again. Safe against a concurrent first run
/// doing the same: whoever renames first wins and the other finds `new` in
/// place; a copy goes to a fresh temporary name first (so `cp -a` can never
/// nest into a `new` that appeared meanwhile) and is renamed into place
/// only if `new` is still absent.
fn adopt_legacy(old: &Path, new: &Path) -> anyhow::Result<()> {
    if new.exists() || !old.is_dir() {
        return Ok(());
    }
    let parent = new.parent().expect("deps dir has a parent");
    std::fs::create_dir_all(parent)?;
    match std::fs::rename(old, new) {
        Ok(()) => {}
        // A concurrent run adopted it (old is gone, or new is now full).
        Err(_) if new.exists() => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            let tmp = parent.join(format!(
                ".adopt.{}.{}",
                new.file_name().unwrap_or_default().to_string_lossy(),
                ulid::Ulid::generate()
            ));
            let st = Command::new("cp")
                .arg("-a")
                .arg(old)
                .arg(&tmp)
                .status()
                .context("cp -a legacy cache")?;
            if !st.success() {
                let _ = quod_cas::remove_any(&tmp);
                bail!("copy legacy cache {} to {}", old.display(), tmp.display());
            }
            if std::fs::rename(&tmp, new).is_err() {
                // Someone else's copy landed first; keep theirs.
                quod_cas::remove_any(&tmp)?;
                if !new.exists() {
                    bail!(
                        "could not move {} into place at {}",
                        tmp.display(),
                        new.display()
                    );
                }
                return Ok(());
            }
            quod_cas::remove_any(old)?;
        }
        Err(e) => {
            return Err(e).with_context(|| format!("move {} to {}", old.display(), new.display()));
        }
    }
    // The checkout's `.quod/` goes too once nothing is left in it.
    if let Some(parent) = old.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    tracing::info!(target: "quod", from = %old.display(), to = %new.display(), "adopted a legacy cache from the checkout");
    say!(
        "QUOD  moved {} out of the checkout to {}",
        old.display(),
        new.display()
    );
    Ok(())
}

/// For a network step: the state dir's `.quod` bound at
/// `/workspace/.quod`, writable. Adopts any dependency tree an older
/// flow left in the checkout first, so step A updates it instead of
/// fetching it again, and makes the mount point (bubblewrap cannot create
/// one through the repo's bind); `after_network_step` removes it.
pub fn network_mount(deps: &Deps, repo: &Path, profile: &Profile) -> anyhow::Result<Mount> {
    for name in served(profile) {
        adopt_legacy(&repo.join(DOT).join(name), &deps.dir(name))?;
    }
    std::fs::create_dir_all(&deps.root)
        .with_context(|| format!("create {}", deps.root.display()))?;
    std::fs::create_dir_all(repo.join(DOT))?;
    Ok(Mount {
        src: deps.root.clone(),
        dst: Path::new(quod_key::WORKSPACE).join(DOT),
        writable: true,
    })
}

/// Where the mount point for a workspace-relative bind lies on the host
/// during a network step: under the state dir's `.quod` when the path
/// is under `.quod/` (that directory is bound over the checkout's),
/// in the checkout otherwise.
pub fn mount_point(deps: &Deps, repo: &Path, rel: &str) -> PathBuf {
    match rel.strip_prefix(".quod/") {
        Some(under) => deps.root.join(under),
        None => repo.join(rel),
    }
}

/// After a network step: pins uv's cache stamps (`quod_hooks::uvstamp`)
/// in the dependency trees the profile serves to later steps (Python's
/// venv); returns how many files were rewritten.
pub fn pin_install_stamps(deps: &Deps, profile: &Profile) -> anyhow::Result<usize> {
    let mut n = 0;
    for name in served(profile) {
        let dir = deps.dir(name);
        if dir.is_dir() {
            n += quod_hooks::uvstamp::pin_uv_cache_stamps(&dir)?;
        }
    }
    Ok(n)
}

/// After a network step: the checkout keeps nothing of Quod's but the
/// mount point, removed when empty (a `.quod/` with anything of the
/// user's in it stays).
pub fn after_network_step(repo: &Path) {
    let _ = std::fs::remove_dir(repo.join(DOT));
}

#[cfg(test)]
mod tests {
    use super::*;
    use quod_key::Toolchain;

    #[test]
    fn served_trees_are_the_profiles_seed_dirs_under_quod() {
        assert_eq!(served(Toolchain::Go.profile()), ["gomod"]);
        assert_eq!(served(Toolchain::Python.profile()), ["venv"]);
        assert!(served(Toolchain::Node.profile()).is_empty());
    }

    #[test]
    fn network_mount_binds_the_state_dir_over_the_checkouts_quod() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let deps = Deps {
            root: tmp.path().join("deps/git:abc/img/go"),
        };
        let m = network_mount(&deps, &repo, Toolchain::Go.profile()).unwrap();
        assert_eq!(m.src, deps.root);
        assert!(m.src.is_dir() && m.writable);
        assert_eq!(m.dst.to_str(), Some("/workspace/.quod"));
        assert!(repo.join(".quod").is_dir(), "the mount point");
        assert_eq!(
            mount_point(&deps, &repo, ".quod/npm-cache"),
            deps.root.join("npm-cache")
        );
        assert_eq!(mount_point(&deps, &repo, "build"), repo.join("build"));
        after_network_step(&repo);
        assert!(!repo.join(".quod").exists(), "an empty mount point goes");
        std::fs::create_dir_all(repo.join(".quod/mine")).unwrap();
        after_network_step(&repo);
        assert!(repo.join(".quod/mine").exists(), "anything else stays");
    }

    #[test]
    fn import_grafts_the_dependency_tree_and_adopts_a_legacy_one() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".quod/gomod/x.y/m@v1")).unwrap();
        std::fs::create_dir_all(repo.join(".quod/gomod/cache/download")).unwrap();
        std::fs::create_dir_all(repo.join(".quod/gocache/00")).unwrap();
        std::fs::write(repo.join(".quod/gomod/x.y/m@v1/a.go"), "a").unwrap();
        std::fs::write(repo.join("go.mod"), "module x").unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let deps = Deps {
            root: tmp.path().join("deps/git:abc/img/go"),
        };
        let go = Toolchain::Go.profile();
        let t = import_workspace(&cas, &repo, &deps, go).unwrap();
        // The legacy module cache moved out of the checkout and is served
        // at its workspace path; the legacy build cache is simply dropped.
        assert!(!repo.join(".quod/gomod").exists());
        assert!(deps.dir("gomod").join("x.y/m@v1/a.go").is_file());
        assert!(t.get(".quod/gomod/x.y/m@v1/a.go").is_some());
        assert!(t.get(".quod/gomod/cache/download").is_some());
        assert!(t.get(".quod/gocache").is_none());
        assert!(t.get("go.mod").is_some());
        // Without any tree in the state dir the tree has no .quod.
        let bare = Deps {
            root: tmp.path().join("deps/other/img/go"),
        };
        let t = import_workspace(&cas, &repo, &bare, go).unwrap();
        assert!(t.get(".quod").is_none());
    }

    /// Adoption happens once and never nests: with the new place already
    /// there (a concurrent first run got there first), the old tree is left
    /// alone and nothing is copied into the new one.
    #[test]
    fn adoption_is_once_and_never_nests() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("repo/.quod/gomod");
        let new = tmp.path().join("deps/r/i/go/gomod");
        std::fs::create_dir_all(old.join("m@v1")).unwrap();
        std::fs::write(old.join("m@v1/a.go"), "a").unwrap();
        adopt_legacy(&old, &new).unwrap();
        assert!(new.join("m@v1/a.go").is_file() && !old.exists());
        // Again, with a new legacy copy appearing: the adopted one stays.
        std::fs::create_dir_all(old.join("m@v2")).unwrap();
        adopt_legacy(&old, &new).unwrap();
        assert!(old.join("m@v2").is_dir(), "left alone");
        assert!(
            !new.join("gomod").exists() && !new.join("m@v2").exists(),
            "no nesting"
        );
    }

    #[test]
    fn repo_id_outside_git_is_a_path_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let id = repo_id(tmp.path());
        assert!(id.starts_with("path:") && id.len() == 21, "{id}");
        assert_eq!(id, repo_id(tmp.path()));
    }
}
