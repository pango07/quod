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
pub use overlay::{Overlay, Tmpfs, overlay, overlay_dir, syncfs, tmpfs};

/// The modification time served for a regular file: a function of its
/// content alone, so it changes exactly when the content does. A constant
/// mtime let a cache keyed by (path, mtime[, size]) that was written under
/// observation (@babel/register, `eslint --cache`, a timestamp pyc) trust a
/// stale entry once it came back into a tree: the edited source showed the
/// same mtime. Seconds carry 28 bits of the content hash, so a tool that
/// compares only whole seconds still tells contents apart (equal odds
/// 2^-28), and nanoseconds 30 more. The base, 2000-01-01, keeps every served
/// file in the past and older than anything a step writes. `CasFs::attr`
/// applies it, and both transports encode the times that returns. Sealed
/// snapshot files get the same times, so a file looks the same wherever it
/// is served.
pub fn content_mtime(content: &sp_cas::Hash) -> std::time::SystemTime {
    let b = content.as_bytes();
    let secs = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) & ((1 << 28) - 1);
    let nanos = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) % 1_000_000_000;
    std::time::UNIX_EPOCH + std::time::Duration::new(946_684_800 + u64::from(secs), nanos)
}

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
    /// Ring transport only: the nice value the queue threads actually run
    /// at (the least favoured of them), and whether lowering it was refused
    /// for lack of CAP_SYS_NICE.
    pub ring_nice: Option<i32>,
    pub ring_nice_denied: bool,
    /// After a fallback: the FUSE connection id (its directory name in
    /// fusectl) of the ring mount that was abandoned before the classic
    /// transport mounted. Before falling back, `serve` verified that every
    /// ring thread ended, this connection was aborted and the ring's
    /// private fusectl mount no longer listed it (by id and inode, so not a
    /// reused id), the mount was unmounted outright, and the path held what
    /// it held before.
    pub ring_abandoned_conn: Option<u64>,
    /// Times a serving thread found a model lock held by another.
    pub lock_contended: u64,
}

impl ObserverStats {
    /// The transport that served the mount, as the run event and the
    /// ledger name it: `fuse-uring` or `fuse` (classic, including a ring
    /// that fell back).
    pub fn transport(&self) -> &'static str {
        if self.uring { "fuse-uring" } else { "fuse" }
    }

    /// The transport with how it ran, for the run's observer string:
    /// `fuse-uring+nice-10`, `fuse-uring+nice0(denied)` when lowering the
    /// queue threads' nice value was refused, `fuse+uring-fallback` when
    /// the ring was tried and abandoned, or `fuse`.
    pub fn transport_detail(&self) -> String {
        let mut s = self.transport().to_string();
        if self.uring {
            if let Some(n) = self.ring_nice {
                s += &format!("+nice{n}");
            }
            if self.ring_nice_denied {
                s += "(denied)";
            }
        }
        if self.uring_fallback {
            s += "+uring-fallback";
        }
        s
    }
}

/// What the scheduler did with the threads serving requests (fuser's
/// `fuser-N`, the ring's `sp-ring-*`), from /proc/self/task/*/schedstat:
/// time on a CPU, time runnable but waiting for one, and timeslices. Ring
/// queue threads are pinned, so their wait is the latency a loaded CPU adds
/// to every request it raised.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SchedStats {
    pub threads: u64,
    pub run_ms: u64,
    pub wait_ms: u64,
    pub slices: u64,
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
    /// A serving thread panicked. The step saw EIO from then on and was
    /// aborted; nothing it did can be recorded.
    #[error(
        "the observer failed while serving the step ({what}); the step was aborted and nothing was recorded"
    )]
    Fatal { what: String },
    #[error("the observer needs Linux (FUSE and overlayfs)")]
    Unsupported,
}

/// The environment variable that injects a fault into the observer, for
/// tests of the failure path only: `observer-panic:<path>` makes the lookup
/// of that workspace-relative path panic in the serving thread, on either
/// transport. Read only in builds with the `fault-injection` feature, like
/// the ring's STILLPASS_TEST_URING_FAULT.
pub const FAULT_ENV: &str = "STILLPASS_FAULT";

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
        pub uring_nice: i32,
        #[cfg(feature = "fault-injection")]
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
                uring_nice: -10,
                #[cfg(feature = "fault-injection")]
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

    pub fn overlay_dir(_: &Path, _: &Path, _: &Path, _: &Path) -> Result<Overlay, FsError> {
        Err(FsError::Unsupported)
    }

    pub struct Tmpfs;

    impl Tmpfs {
        pub fn dir(&self) -> &Path {
            Path::new("")
        }

        pub fn unmount(self) -> Result<(), FsError> {
            Err(FsError::Unsupported)
        }
    }

    pub fn tmpfs(_: &Path, _: &str) -> Result<Tmpfs, FsError> {
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

        pub fn sched(&self) -> crate::SchedStats {
            crate::SchedStats::default()
        }

        pub fn abort_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
            Default::default()
        }

        pub fn fatal(&self) -> Option<String> {
            None
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

        pub fn unmount(&mut self) -> Result<(), FsError> {
            Err(FsError::Unsupported)
        }
    }
}
#[cfg(not(target_os = "linux"))]
pub use unsupported::{
    Fault, Options, Overlay, Served, Tmpfs, Transport, overlay, overlay_dir, serve, serve_with,
    syncfs, tmpfs,
};

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use sp_cas::Hash;

    use super::content_mtime;

    #[test]
    fn content_mtime_changes_exactly_with_content() {
        let a = content_mtime(&Hash::of(b"module.exports = 1;\n"));
        assert_eq!(a, content_mtime(&Hash::of(b"module.exports = 1;\n")));
        let secs = |t: std::time::SystemTime| t.duration_since(UNIX_EPOCH).unwrap().as_secs();
        // Same size, different content: different whole seconds too.
        let b = content_mtime(&Hash::of(b"module.exports = 2;\n"));
        assert_ne!(secs(a), secs(b));
        // Always in 2000-01-01 .. 2008-07, never the observer's old epoch.
        for i in 0..1000u32 {
            let t = content_mtime(&Hash::of(&i.to_le_bytes()));
            let d = t.duration_since(UNIX_EPOCH).unwrap();
            assert!(d >= Duration::from_secs(946_684_800));
            assert!(d < Duration::from_secs(946_684_800 + (1 << 28)));
        }
    }
}
