//! The import's stat cache: a file's hash by its identity, so importing a
//! directory again reads only the files that changed since.
//!
//! The identity is the file's device, inode, size, mode, mtime and ctime.
//! Writing a file's data or metadata moves its ctime, which no caller can
//! set back (utimensat sets atime and mtime only), and replacing it gives a
//! new inode, so a file with the identity it had when it was hashed holds
//! the bytes that were hashed. Two guards close the gaps. File times are
//! coarse (the kernel's clock tick), so a file written within the same tick
//! after it was read could keep its identity: an entry is kept only for a
//! file whose times are more than `SETTLE` old when it is hashed (git's
//! racy-index rule); a younger one is hashed again next time. And the
//! identity is taken before the read and checked after it: a file that
//! changed while it was read is not kept.
//!
//! The settle time is measured from when the read starts, not from when it
//! ends: a read longer than SETTLE would otherwise keep a file written just
//! before it.
//!
//! None of this holds where the client does not own the file's identity: a
//! network or FUSE file system (NFS, 9p, SMB, Ceph, AFS, FUSE) reports
//! times and inode numbers its server or daemon chooses, and NFS's
//! attribute cache can report an identity that is already stale. Files
//! there are always hashed (`StatCache::trusts`).
//!
//! One cache file per imported root (and skip list), under the store, so the
//! two go together. Losing or corrupting it costs a full import, never a
//! wrong hash: anything that does not parse is ignored.

use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::Hash;

/// How old a file's times must be for its hash to be kept.
pub(crate) const SETTLE: Duration = Duration::from_secs(2);

const MAGIC: &[u8] = b"quod import stat cache v1\n";

/// What says a file is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ident {
    dev: u64,
    ino: u64,
    size: u64,
    mode: u32,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl Ident {
    pub(crate) fn of(m: &Metadata) -> Ident {
        Ident {
            dev: m.dev(),
            ino: m.ino(),
            size: m.size(),
            mode: m.mode(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
        }
    }

    /// Both times are more than SETTLE before `now`.
    pub(crate) fn settled(&self, now: SystemTime) -> bool {
        let Ok(now) = now.duration_since(UNIX_EPOCH) else {
            return false;
        };
        let at = |(s, ns): (i64, i64)| {
            u64::try_from(s)
                .ok()
                .map(|s| Duration::new(s, ns.clamp(0, 999_999_999) as u32))
        };
        [self.mtime, self.ctime]
            .into_iter()
            .all(|t| at(t).is_some_and(|t| t + SETTLE < now))
    }

    fn write(&self, out: &mut Vec<u8>) {
        for v in [self.dev, self.ino, self.size, u64::from(self.mode)] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for (s, ns) in [self.mtime, self.ctime] {
            out.extend_from_slice(&s.to_le_bytes());
            out.extend_from_slice(&ns.to_le_bytes());
        }
    }

    fn read(b: &mut &[u8]) -> Option<Ident> {
        let mut u = || take::<8>(b).map(u64::from_le_bytes);
        let (dev, ino, size, mode) = (u()?, u()?, u()?, u32::try_from(u()?).ok()?);
        let mut i = || take::<8>(b).map(i64::from_le_bytes);
        Some(Ident {
            dev,
            ino,
            size,
            mode,
            mtime: (i()?, i()?),
            ctime: (i()?, i()?),
        })
    }
}

fn take<const N: usize>(b: &mut &[u8]) -> Option<[u8; N]> {
    let (head, rest) = b.split_first_chunk::<N>()?;
    *b = rest;
    Some(*head)
}

/// File systems (statfs f_type) whose file identity the stat cache does not
/// trust: what they report is the server's or the daemon's.
const UNTRUSTED_FS: &[(i64, &str)] = &[
    (0x6969, "nfs"),
    (0x0102_1997, "9p"),
    (0x6573_5546, "fuse"),
    (0xff53_4d42, "cifs"),
    (0xfe53_4d42, "smb2"),
    (0x517b, "smb"),
    (0x00c3_6400, "ceph"),
    (0x5346_414f, "afs"),
];

/// Whether files on a file system of type `magic` keep their hash by
/// identity.
pub(crate) fn trusted_fs(magic: i64) -> bool {
    !UNTRUSTED_FS.iter().any(|(m, _)| *m == magic)
}

/// The statfs f_type of the file system holding `path`.
#[cfg(target_os = "linux")]
fn fs_magic(path: &std::path::Path) -> Option<i64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statfs is plain old data; all zeroes is a valid value.
    #[allow(unsafe_code)] // statfs(2) has no safe std wrapper
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: a valid NUL-terminated path and a statfs to fill, both
    // outliving the call.
    #[allow(unsafe_code)]
    let rc = unsafe { libc::statfs(c.as_ptr(), &mut st) };
    #[allow(clippy::unnecessary_cast)] // f_type's width differs by arch
    (rc == 0).then_some(st.f_type as i64)
}

/// Elsewhere the cache is never trusted.
#[cfg(not(target_os = "linux"))]
fn fs_magic(_: &std::path::Path) -> Option<i64> {
    None
}

/// A root's cache: what the last import kept, and what this one keeps (only
/// paths it saw, so removed files drop out).
pub(crate) struct StatCache {
    path: PathBuf,
    old: HashMap<String, (Ident, Hash)>,
    new: Mutex<HashMap<String, (Ident, Hash)>>,
    /// Per device: whether its file system is trusted (`trusts`).
    devices: Mutex<HashMap<u64, bool>>,
}

impl StatCache {
    pub(crate) fn load(path: PathBuf) -> StatCache {
        let old = fs::read(&path)
            .ok()
            .and_then(|b| parse(&b))
            .unwrap_or_default();
        StatCache {
            path,
            old,
            new: Mutex::new(HashMap::new()),
            devices: Mutex::new(HashMap::new()),
        }
    }

    /// Whether a file with `ident`, at `path`, may keep its hash by
    /// identity: its file system is not one in `UNTRUSTED_FS` (asked once
    /// per device; a file system that cannot be asked is not trusted).
    pub(crate) fn trusts(&self, ident: &Ident, path: &std::path::Path) -> bool {
        let mut devices = self.devices.lock().unwrap();
        *devices
            .entry(ident.dev)
            .or_insert_with(|| fs_magic(path).is_some_and(trusted_fs))
    }

    /// The hash kept for `rel` if the file still has `ident`.
    pub(crate) fn get(&self, rel: &str, ident: &Ident) -> Option<Hash> {
        let (i, h) = self.old.get(rel)?;
        (i == ident).then_some(*h)
    }

    /// Keeps `rel`'s hash for the next import.
    pub(crate) fn keep(&self, rel: &str, ident: Ident, h: Hash) {
        self.new.lock().unwrap().insert(rel.to_string(), (ident, h));
    }

    /// Writes the cache if it changed (atomically: a temp file renamed over
    /// it). A failure only costs the next import time.
    pub(crate) fn save(self) {
        let new = self.new.into_inner().unwrap();
        if new == self.old {
            return;
        }
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&(new.len() as u64).to_le_bytes());
        for (rel, (ident, h)) in &new {
            out.extend_from_slice(&(rel.len() as u64).to_le_bytes());
            out.extend_from_slice(rel.as_bytes());
            ident.write(&mut out);
            out.extend_from_slice(h.as_bytes());
        }
        let Some(dir) = self.path.parent() else {
            return;
        };
        let tmp = dir.join(format!(
            ".{}.{}",
            self.path.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id()
        ));
        let ok = fs::create_dir_all(dir).is_ok()
            && fs::File::create(&tmp)
                .and_then(|mut f| f.write_all(&out))
                .is_ok()
            && fs::rename(&tmp, &self.path).is_ok();
        if !ok {
            let _ = fs::remove_file(&tmp);
        }
    }
}

fn parse(b: &[u8]) -> Option<HashMap<String, (Ident, Hash)>> {
    let mut b = b.strip_prefix(MAGIC)?;
    let n = u64::from_le_bytes(take::<8>(&mut b)?);
    let mut out = HashMap::with_capacity(n.min(1 << 20) as usize);
    for _ in 0..n {
        let len = usize::try_from(u64::from_le_bytes(take::<8>(&mut b)?)).ok()?;
        let (rel, rest) = b.split_at_checked(len)?;
        b = rest;
        let rel = std::str::from_utf8(rel).ok()?.to_string();
        let ident = Ident::read(&mut b)?;
        let h = Hash::from_bytes(take::<32>(&mut b)?);
        out.insert(rel, (ident, h));
    }
    b.is_empty().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cache_round_trips_and_garbage_is_empty() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        fs::write(&f, "x").unwrap();
        let ident = Ident::of(&fs::metadata(&f).unwrap());
        let path = d.path().join("c/cache");
        let c = StatCache::load(path.clone());
        assert_eq!(c.get("f", &ident), None);
        c.keep("f", ident, Hash::of(b"x"));
        c.save();
        let c = StatCache::load(path.clone());
        assert_eq!(c.get("f", &ident), Some(Hash::of(b"x")));
        let mut other = ident;
        other.ctime.1 += 1;
        assert_eq!(c.get("f", &other), None);
        fs::write(&path, b"quod import stat cache v1\n\x05").unwrap();
        assert!(StatCache::load(path).old.is_empty());
    }

    #[test]
    fn network_and_fuse_file_systems_are_not_trusted() {
        for (magic, name) in UNTRUSTED_FS {
            assert!(!trusted_fs(*magic), "{name}");
        }
        // ext4, xfs, btrfs, tmpfs, overlayfs.
        for magic in [0xef53, 0x5846_5342, 0x9123_683e, 0x0102_1994, 0x794c_7630] {
            assert!(trusted_fs(magic), "{magic:#x}");
        }
        // The test's own temp dir is a local file system (on Linux; the
        // cache is never trusted elsewhere).
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        fs::write(&f, "x").unwrap();
        let c = StatCache::load(d.path().join("cache"));
        let ident = Ident::of(&fs::metadata(&f).unwrap());
        assert_eq!(c.trusts(&ident, &f), cfg!(target_os = "linux"));
    }

    #[test]
    fn only_settled_files_are_kept() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        fs::write(&f, "x").unwrap();
        let ident = Ident::of(&fs::metadata(&f).unwrap());
        assert!(!ident.settled(SystemTime::now()));
        assert!(ident.settled(SystemTime::now() + Duration::from_secs(3)));
    }
}
