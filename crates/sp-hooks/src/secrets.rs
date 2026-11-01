//! Secrets a step is given: salted in its key, masked in everything stored.
//!
//! The hook is never told which values are secrets (the runner masks its
//! own list in the logs it uploads, and gives the hook raw output). What it
//! can see: GitHub-issued tokens (withheld from keyed steps altogether, see
//! `step::withheld`) and variables whose names say they hold a secret
//! (`step::secret_like`). Those values go into the key as a salted hash
//! (per state directory, so a stored key hash cannot be checked against
//! guessed values elsewhere), and every log blob is masked before it is
//! written. A step whose outputs contain one of them is not recorded and
//! its outputs never reach the store.

use std::io::Read;
use std::path::{Path, PathBuf};

use aho_corasick::AhoCorasick;
use anyhow::Context;
use sp_cas::Hash;

/// The state's secret salt, created on first use (32 random bytes, 0600).
pub fn salt(state_dir: &Path) -> anyhow::Result<[u8; 32]> {
    let p = state_dir.join("salt");
    if let Ok(b) = std::fs::read(&p)
        && let Ok(s) = <[u8; 32]>::try_from(b.as_slice())
    {
        return Ok(s);
    }
    let mut s = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut s))
        .context("read /dev/urandom")?;
    std::fs::create_dir_all(state_dir)?;
    let tmp = state_dir.join(format!("salt.tmp.{}", ulid::Ulid::generate()));
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&s)?;
        f.sync_all()?;
    }
    // Two first uses at once: the first rename wins, both read it back.
    if std::fs::hard_link(&tmp, &p).is_err() {
        let _ = std::fs::remove_file(&tmp);
        let b = std::fs::read(&p)?;
        return <[u8; 32]>::try_from(b.as_slice())
            .map_err(|_| anyhow::anyhow!("corrupt {}", p.display()));
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(s)
}

/// What the key holds instead of a secret value.
pub fn salted(salt: &[u8; 32], name: &str, value: &str) -> String {
    let mut h = blake3::Hasher::new_keyed(salt);
    h.update(name.as_bytes());
    h.update(&[0]);
    h.update(value.as_bytes());
    format!("stillpass:secret:{}", Hash::from_blake3(h.finalize()))
}

/// The byte strings to mask for one secret value: the value and each of
/// its lines (the runner masks multi-line secrets line by line), longest
/// first, ignoring fragments too short to mean anything.
pub fn masks_for(value: &str) -> Vec<Vec<u8>> {
    let mut m: Vec<Vec<u8>> = std::iter::once(value)
        .chain(value.lines())
        .map(str::trim_end)
        .filter(|s| s.len() >= 4)
        .map(|s| s.as_bytes().to_vec())
        .collect();
    m.sort_by_key(|b| std::cmp::Reverse(b.len()));
    m.dedup();
    m
}

/// `bytes` with every occurrence of a mask replaced by `***`.
pub fn mask(bytes: &[u8], masks: &[Vec<u8>]) -> Vec<u8> {
    if masks.is_empty() {
        return bytes.to_vec();
    }
    let ac = AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::LeftmostLongest)
        .build(masks)
        .expect("mask patterns");
    let mut out = Vec::with_capacity(bytes.len());
    ac.replace_all_with_bytes(bytes, &mut out, |_, _, dst| {
        dst.extend_from_slice(b"***");
        true
    });
    out
}

/// The first entry under `dir` that carries a mask: in a file's content,
/// in any entry's name (a step can write a secret as a file name), or in a
/// symlink's target.
pub fn file_containing(dir: &Path, masks: &[Vec<u8>]) -> anyhow::Result<Option<PathBuf>> {
    use std::os::unix::ffi::OsStrExt;
    if masks.is_empty() || !dir.exists() {
        return Ok(None);
    }
    let ac = AhoCorasick::new(masks).context("mask patterns")?;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).with_context(|| format!("read {}", d.display()))? {
            let e = e?;
            if ac.find(e.file_name().as_bytes()).is_some() {
                return Ok(Some(e.path()));
            }
            let t = e.file_type()?;
            if t.is_dir() {
                stack.push(e.path());
            } else if t.is_symlink() {
                let target = std::fs::read_link(e.path())?;
                if ac.find(target.as_os_str().as_bytes()).is_some() {
                    return Ok(Some(e.path()));
                }
            } else if t.is_file() {
                let f = std::fs::File::open(e.path())?;
                if ac.stream_find_iter(f).next().is_some() {
                    return Ok(Some(e.path()));
                }
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salt_is_stable_and_salting_hides_the_value() {
        let tmp = tempfile::tempdir().unwrap();
        let a = salt(tmp.path()).unwrap();
        assert_eq!(salt(tmp.path()).unwrap(), a);
        let other = salt(&tmp.path().join("other")).unwrap();
        assert_ne!(a, other);
        let s = salted(&a, "NPM_TOKEN", "hunter2hunter2");
        assert!(s.starts_with("stillpass:secret:b3:") && !s.contains("hunter2"));
        assert_ne!(s, salted(&other, "NPM_TOKEN", "hunter2hunter2"));
        assert_ne!(s, salted(&a, "NPM_TOKEN", "hunter3hunter3"));
    }

    #[test]
    fn masking() {
        let m = masks_for("line-one-secret\nline-two-secret");
        let out = mask(
            b"a line-one-secret b line-two-secret\nline-one-secret\nline-two-secret",
            &m,
        );
        assert_eq!(out, b"a *** b ***\n***");
        assert_eq!(mask(b"abc", &masks_for("ab")), b"abc", "too short to mask");
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        std::fs::write(tmp.path().join("a/b/f"), "x line-two-secret y").unwrap();
        std::fs::write(tmp.path().join("a/g"), "nothing").unwrap();
        assert_eq!(
            file_containing(tmp.path(), &m).unwrap(),
            Some(tmp.path().join("a/b/f"))
        );
        assert_eq!(
            file_containing(tmp.path(), &masks_for("absent-secret")).unwrap(),
            None
        );
        // A secret as a file name, or as a symlink's target.
        let named = tempfile::tempdir().unwrap();
        std::fs::write(named.path().join("x-line-one-secret.txt"), "nothing").unwrap();
        assert_eq!(
            file_containing(named.path(), &m).unwrap(),
            Some(named.path().join("x-line-one-secret.txt"))
        );
        let linked = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/tmp/line-two-secret", linked.path().join("l")).unwrap();
        assert_eq!(
            file_containing(linked.path(), &m).unwrap(),
            Some(linked.path().join("l"))
        );
    }
}
