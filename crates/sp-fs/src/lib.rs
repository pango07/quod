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
mod uring;
#[cfg(target_os = "linux")]
pub use serve::{Fault, Options, Served, Transport, serve, serve_with};

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
    /// Requests arrive over io_uring rather than the /dev/fuse read loop.
    /// Set only after every queue registered and a request came back over
    /// the ring.
    pub uring: bool,
    /// The ring was tried for this mount and abandoned before the step
    /// started; the classic transport served all of it.
    pub uring_fallback: bool,
    /// Ring transport only: requests answered over the ring and over the
    /// device (forgets and interrupts, which have no reply, not counted),
    /// completion batches, batches in which every entry of the queue was
    /// busy, and userspace time spent answering ring requests.
    pub ring_requests: u64,
    pub device_requests: u64,
    pub ring_batches: u64,
    pub ring_full_batches: u64,
    pub ring_serve_us: u64,
}

impl ObserverStats {
    /// The transport that served the mount, as the run event and the
    /// ledger name it: `fuse-uring` or `fuse` (classic, including a ring
    /// that fell back).
    pub fn transport(&self) -> &'static str {
        if self.uring { "fuse-uring" } else { "fuse" }
    }
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

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Transport {
        Auto,
        Classic,
        Uring,
    }

    impl Transport {
        pub fn from_env() -> Transport {
            Transport::Auto
        }
    }

    #[doc(hidden)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Fault {
        Register,
        Unready,
        QueuePanic(u64),
        QueueError(u64),
    }

    #[derive(Clone, Debug)]
    pub struct Options {
        pub transport: Transport,
        pub passthrough: bool,
        pub uring_pin: bool,
        pub uring_depth: usize,
        #[doc(hidden)]
        pub fault: Option<Fault>,
    }

    impl Default for Options {
        fn default() -> Options {
            Options {
                transport: Transport::Auto,
                passthrough: true,
                uring_pin: true,
                uring_depth: 4,
                fault: None,
            }
        }
    }

    impl Options {
        pub fn from_env() -> Options {
            Options::default()
        }
    }

    pub fn serve(_: &Tree, _: &Cas, _: &Path) -> Result<Served, FsError> {
        Err(FsError::Unsupported)
    }

    pub fn serve_with(_: &Tree, _: &Cas, _: &Path, _: &Options) -> Result<Served, FsError> {
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
pub use unsupported::{
    Fault, Options, Overlay, Served, Transport, overlay, serve, serve_with, syncfs,
};
