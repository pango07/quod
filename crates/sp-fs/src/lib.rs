//! The observer. A read-only FUSE filesystem that serves a CAS tree and logs
//! every lookup, getattr, readdir, open, read and readlink as manifest
//! accesses, plus the kernel overlay that captures a step's writes.
//!
//! Safety invariant: over-record, never under-record. When an operation's
//! classification is unclear, record the most specific kind that is
//! certainly true, or record both.

#[cfg(target_os = "linux")]
mod serve;
#[cfg(target_os = "linux")]
pub use serve::{Served, serve};

#[cfg(target_os = "linux")]
mod overlay;
#[cfg(target_os = "linux")]
pub use overlay::{Overlay, overlay, syncfs};

/// How many requests of each kind the kernel sent the observer so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObserverStats {
    pub lookup: u64,
    pub lookup_absent: u64,
    pub getattr: u64,
    pub readlink: u64,
    pub open: u64,
    pub read: u64,
    pub read_bytes: u64,
    pub readdir: u64,
    /// Reads of open files go kernel-to-blob and never arrive here.
    pub passthrough: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("{what}: {source}")]
    Io {
        what: String,
        source: std::io::Error,
    },
    #[error(transparent)]
    Cas(#[from] sp_cas::CasError),
    #[error("the observer needs Linux (FUSE and overlayfs)")]
    Unsupported,
}

impl FsError {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))] // only the Linux observer builds paths
    pub(crate) fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> FsError {
        let what = what.into();
        move |source| FsError::Io { what, source }
    }
}

/// Off Linux the observer cannot run; these keep the CLI compiling on a
/// developer's Mac and fail with a clear error.
#[cfg(not(target_os = "linux"))]
mod unsupported {
    use std::path::Path;

    use sp_cas::{Cas, Entry, Tree};
    use sp_manifest::Access;

    use crate::FsError;

    pub struct Served;
    pub struct Overlay;

    pub fn serve(_: &Tree, _: &Cas, _: &Path) -> Result<Served, FsError> {
        Err(FsError::Unsupported)
    }

    pub fn overlay(_: &Served, _: &Path, _: &Path, _: &Path) -> Result<Overlay, FsError> {
        Err(FsError::Unsupported)
    }

    pub fn syncfs(_: &Path) -> std::io::Result<()> {
        Err(std::io::ErrorKind::Unsupported.into())
    }

    impl Served {
        pub fn mountpoint(&self) -> &Path {
            Path::new("/")
        }

        pub fn stats(&self) -> crate::ObserverStats {
            crate::ObserverStats::default()
        }

        pub fn unmount(self) -> Result<Vec<Access>, FsError> {
            Err(FsError::Unsupported)
        }
    }

    impl Overlay {
        pub fn merged(&self) -> &Path {
            Path::new("/")
        }

        pub fn finish(self, _: &Tree, _: &Cas) -> Result<Vec<Entry>, FsError> {
            Err(FsError::Unsupported)
        }
    }
}
#[cfg(not(target_os = "linux"))]
pub use unsupported::{Overlay, Served, overlay, serve, syncfs};
