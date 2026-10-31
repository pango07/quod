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
pub use overlay::{Overlay, overlay};

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

    impl Served {
        pub fn mountpoint(&self) -> &Path {
            Path::new("/")
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
pub use unsupported::{Overlay, Served, overlay, serve};
