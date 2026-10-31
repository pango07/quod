//! Kernel overlayfs with the FUSE mount as lower and a host directory as
//! upper. At step end the upper dir is the step's output, diffed against
//! the lower tree.

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

use sp_cas::{Cas, Entry, Hash, Kind, ROOT, Tree, child_path};

use crate::{FsError, Served};

/// A mounted overlay. `finish` unmounts it and returns the outputs.
pub struct Overlay {
    upper: PathBuf,
    merged: PathBuf,
    mounted: bool,
}

/// Mounts overlayfs at `merged` with `served` as lower. Needs root or a user
/// namespace with CAP_SYS_ADMIN.
pub fn overlay(
    served: &Served,
    upper: &Path,
    work: &Path,
    merged: &Path,
) -> Result<Overlay, FsError> {
    for d in [upper, work, merged] {
        fs::create_dir_all(d).map_err(FsError::io(format!("create {}", d.display())))?;
    }
    let opts = format!(
        // Pinned so the upper dir is plain files: no metacopy (a chmod would
        // copy up an empty file), no redirects (a renamed dir would arrive
        // without contents), no index.
        "lowerdir={},upperdir={},workdir={},redirect_dir=off,metacopy=off,index=off",
        served.mountpoint().display(),
        upper.display(),
        work.display()
    );
    let c = |s: &[u8]| {
        CString::new(s).map_err(|_| FsError::Io {
            what: "overlay path contains NUL".into(),
            source: std::io::ErrorKind::InvalidInput.into(),
        })
    };
    let (src, target, fstype, data) = (
        c(b"overlay")?,
        c(merged.as_os_str().as_bytes())?,
        c(b"overlay")?,
        c(opts.as_bytes())?,
    );
    // SAFETY: every pointer is a valid NUL-terminated string that outlives the call.
    #[allow(unsafe_code)] // mount(2) has no safe std wrapper
    let rc = unsafe {
        libc::mount(
            src.as_ptr(),
            target.as_ptr(),
            fstype.as_ptr(),
            0,
            data.as_ptr().cast(),
        )
    };
    if rc != 0 {
        return Err(FsError::Io {
            what: format!("mount overlay at {} ({opts})", merged.display()),
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(Overlay {
        upper: upper.to_path_buf(),
        merged: merged.to_path_buf(),
        mounted: true,
    })
}

impl Overlay {
    pub fn merged(&self) -> &Path {
        &self.merged
    }

    /// Unmounts, then walks the upper dir: new and modified files and
    /// symlinks become `f` and `l` outputs (files stored in the CAS),
    /// whiteouts become `x`, directories absent from the lower tree become
    /// `d`, and an opaque directory becomes `x` followed by its contents.
    /// Renames are not reconstructed: they are an `x` plus a new path.
    pub fn finish(mut self, lower: &Tree, cas: &Cas) -> Result<Vec<Entry>, FsError> {
        self.unmount()?;
        let mut out = Vec::new();
        let mut files = Vec::new();
        walk(&self.upper, ROOT, lower, &mut out, &mut files)?;
        // Store every output file in parallel, then fill in the hashes.
        let paths: Vec<PathBuf> = files.iter().map(|(_, p)| p.clone()).collect();
        for ((i, _), (h, _)) in files.iter().zip(cas.put_files(&paths)?) {
            out[*i].h = Some(h);
        }
        Ok(out)
    }

    fn unmount(&mut self) -> Result<(), FsError> {
        if !self.mounted {
            return Ok(());
        }
        let target = CString::new(self.merged.as_os_str().as_bytes()).expect("checked at mount");
        // SAFETY: target is a valid NUL-terminated path.
        #[allow(unsafe_code)] // umount2(2) has no safe std wrapper
        let rc = unsafe { libc::umount2(target.as_ptr(), 0) };
        if rc != 0 {
            return Err(FsError::Io {
                what: format!("unmount overlay {}", self.merged.display()),
                source: std::io::Error::last_os_error(),
            });
        }
        self.mounted = false;
        Ok(())
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = self.unmount();
    }
}

/// Collects outputs into `out`; regular files get a placeholder hash and
/// their (index in `out`, upper path) go to `files` for storing.
fn walk(
    dir: &Path,
    rel: &str,
    lower: &Tree,
    out: &mut Vec<Entry>,
    files: &mut Vec<(usize, PathBuf)>,
) -> Result<(), FsError> {
    let mut items: Vec<_> = fs::read_dir(dir)
        .map_err(FsError::io(format!("read {}", dir.display())))?
        .collect::<Result<_, _>>()
        .map_err(FsError::io(format!("read {}", dir.display())))?;
    items.sort_by_key(|i| i.file_name());
    for item in items {
        let path = item.path();
        let name = item.file_name().into_string().map_err(|n| FsError::Io {
            what: format!("output name is not UTF-8: {n:?}"),
            source: std::io::ErrorKind::InvalidData.into(),
        })?;
        if !sp_cas::valid_name(&name) {
            return Err(FsError::Io {
                what: format!("step created a name listings cannot represent: {name:?}"),
                source: std::io::ErrorKind::InvalidData.into(),
            });
        }
        let p = child_path(rel, &name);
        let meta =
            fs::symlink_metadata(&path).map_err(FsError::io(format!("stat {}", path.display())))?;
        let ft = meta.file_type();
        if ft.is_char_device() && meta.rdev() == 0 {
            out.push(Entry::deleted(p));
        } else if ft.is_dir() {
            refuse_markers(&path)?;
            let opaque = is_opaque(&path)?;
            let existed = lower.get(&p).is_some_and(|e| e.k == Kind::Dir);
            if existed && opaque {
                out.push(Entry::deleted(p.clone()));
            }
            if !existed || opaque {
                if lower.get(&p).is_some_and(|e| e.k != Kind::Dir) {
                    // A file replaced by a directory.
                    out.push(Entry::deleted(p.clone()));
                }
                out.push(Entry::dir(p.clone(), Hash::of(b"")));
            }
            walk(&path, &p, lower, out, files)?;
        } else if ft.is_symlink() {
            let t = fs::read_link(&path)
                .map_err(FsError::io(format!("readlink {}", path.display())))?;
            let t = t.into_os_string().into_string().map_err(|t| FsError::Io {
                what: format!("symlink target is not UTF-8: {t:?}"),
                source: std::io::ErrorKind::InvalidData.into(),
            })?;
            replace_dir(lower, &p, out);
            out.push(Entry::symlink(p, t));
        } else if ft.is_file() {
            refuse_markers(&path)?;
            replace_dir(lower, &p, out);
            files.push((out.len(), path));
            out.push(Entry::file(p, Hash::of(b""), meta.mode()));
        } else {
            return Err(FsError::Io {
                what: format!("step created an unsupported file type at {p}"),
                source: std::io::ErrorKind::Unsupported.into(),
            });
        }
    }
    Ok(())
}

/// A lower directory replaced by a file or symlink must be removed first.
fn replace_dir(lower: &Tree, p: &str, out: &mut Vec<Entry>) {
    if lower.get(p).is_some_and(|e| e.k == Kind::Dir) {
        out.push(Entry::deleted(p.to_string()));
    }
}

/// Reads one xattr. `Ok(None)` when absent or unsupported; any other error
/// is an error, never a silent "no".
fn xattr(path: &Path, name: &str) -> Result<Option<Vec<u8>>, FsError> {
    let (Ok(cpath), Ok(cname)) = (
        CString::new(path.as_os_str().as_bytes()),
        CString::new(name),
    ) else {
        return Ok(None);
    };
    let mut buf = [0u8; 256];
    // SAFETY: cpath and cname are valid C strings; buf is valid for its length.
    #[allow(unsafe_code)] // lgetxattr(2) has no safe std wrapper
    let n = unsafe {
        libc::lgetxattr(
            cpath.as_ptr(),
            cname.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    if n >= 0 {
        return Ok(Some(buf[..n as usize].to_vec()));
    }
    let e = std::io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::ENODATA) | Some(libc::ENOTSUP) => Ok(None),
        _ => Err(FsError::Io {
            what: format!("read xattr {name} on {}", path.display()),
            source: e,
        }),
    }
}

/// Overlay markers this diff cannot interpret. Both xattr namespaces are
/// checked (trusted.* as root, user.* with userxattr).
fn refuse_markers(path: &Path) -> Result<(), FsError> {
    for ns in ["trusted", "user"] {
        for m in ["redirect", "metacopy"] {
            let name = format!("{ns}.overlay.{m}");
            if xattr(path, &name)?.is_some() {
                return Err(FsError::Io {
                    what: format!(
                        "overlay marker {name} on {} (redirect_dir/metacopy must be off)",
                        path.display()
                    ),
                    source: std::io::ErrorKind::Unsupported.into(),
                });
            }
        }
    }
    Ok(())
}

fn is_opaque(dir: &Path) -> Result<bool, FsError> {
    for name in ["trusted.overlay.opaque", "user.overlay.opaque"] {
        if xattr(dir, name)?.is_some_and(|v| v == b"y") {
            return Ok(true);
        }
    }
    Ok(false)
}
