//! The observer's model and its two transports.
//!
//! `CasFs` is the read-only view of a tree with its access log, request
//! counters and the passthrough decision. It knows nothing about how
//! requests arrive: the fuser `Filesystem` impl below and the io_uring
//! transport in `uring.rs` both decode a request, call one model method and
//! encode its answer, so the two transports cannot disagree about what is
//! logged. Which transport serves a mount is decided once, in `serve`,
//! before the step starts.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use fuser::{
    BackgroundSession, BackingId, Config, Errno, FileAttr, FileHandle, FileType, Filesystem,
    FopenFlags, Generation, INodeNo, InitFlags, KernelConfig, LockOwner, MountOption, OpenAccMode,
    OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen, ReplyXattr, Request,
    SessionACL,
};
use sp_cas::{Cas, Entry, Kind, ROOT, Tree, child_path};
use sp_manifest::Access;

use crate::uring::{self, MountError};
use crate::{FsError, ObserverStats, SchedStats};

/// Entries never change during a step, so the kernel may cache them freely.
/// Caching only removes repeat requests, which carry no new information.
pub(crate) const TTL: Duration = Duration::from_secs(3600);
const ROOT_INO: u64 = 1;

/// How requests reach the observer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// The ring when the host offers it (`enable_uring`, root, io_uring, a
    /// FUSE minor the ring transport is tested against), else the classic
    /// device loop through fuser.
    Auto,
    /// The classic `/dev/fuse` read/write loop (fuser).
    Classic,
    /// FUSE over io_uring; an error if the host cannot.
    Uring,
}

impl Transport {
    /// `STILLPASS_FUSE_TRANSPORT`: `classic`, `uring`, else auto.
    pub fn from_env() -> Transport {
        match std::env::var("STILLPASS_FUSE_TRANSPORT").as_deref() {
            Ok("classic") => Transport::Classic,
            Ok("uring") => Transport::Uring,
            _ => Transport::Auto,
        }
    }
}

/// Failures the ring transport can be made to suffer, so tests can prove
/// each one ends in a fallback (before the step) or a Stillpass error
/// (during it), never in a hang or a recorded result. Only reachable with
/// the `fault-injection` feature (`Options::fault`,
/// `STILLPASS_TEST_URING_FAULT`); release builds cannot be told to fail.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Queue 0 registers with a queue id the kernel refuses.
    Register,
    /// The last queue never registers, so the ring never becomes ready.
    Unready,
    /// Once serving, after this many ring requests are answered, the queue
    /// thread that receives the next one panics before answering it.
    QueuePanic(u64),
    /// Once serving, after this many ring requests are answered, the queue
    /// thread that receives the next one sees its completion as failed
    /// (-EIO).
    QueueError(u64),
}

#[cfg(feature = "fault-injection")]
impl Fault {
    /// `STILLPASS_TEST_URING_FAULT`: `register`, `unready`, `panic[:N]`,
    /// `error[:N]` (N requests answered first, default 0).
    fn from_env() -> Option<Fault> {
        let v = std::env::var("STILLPASS_TEST_URING_FAULT").ok()?;
        let (kind, after) = match v.split_once(':') {
            Some((k, n)) => (k.to_string(), n.parse().ok()?),
            None => (v, 0),
        };
        match kind.as_str() {
            "register" => Some(Fault::Register),
            "unready" => Some(Fault::Unready),
            "panic" => Some(Fault::QueuePanic(after)),
            "error" => Some(Fault::QueueError(after)),
            _ => None,
        }
    }
}

/// How a mount is served. `Options::from_env` is what `serve` uses.
#[derive(Clone, Debug)]
pub struct Options {
    pub transport: Transport,
    /// Kernel passthrough when the kernel grants it
    /// (`STILLPASS_NO_PASSTHROUGH=1` turns it off, for bench attribution).
    pub passthrough: bool,
    /// Pin each ring queue thread to the CPU whose requests it serves
    /// (`STILLPASS_URING_PIN=0` turns it off, for measurement).
    pub uring_pin: bool,
    /// Ring entries per queue (`STILLPASS_URING_DEPTH`, 1-64, default 4).
    pub uring_depth: usize,
    /// Nice value of the ring queue threads (`STILLPASS_URING_NICE`,
    /// -20..=19, default -10). A queue thread is pinned to the CPU whose
    /// requests it serves, so on a loaded CPU it waits behind the step's
    /// own processes; a lower nice value lets it run first. It runs for
    /// microseconds per request.
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
        let d = Options::default();
        Options {
            transport: Transport::from_env(),
            passthrough: std::env::var_os("STILLPASS_NO_PASSTHROUGH").is_none(),
            uring_pin: !std::env::var("STILLPASS_URING_PIN").is_ok_and(|v| v == "0"),
            uring_depth: std::env::var("STILLPASS_URING_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|n| (1..=64).contains(n))
                .unwrap_or(d.uring_depth),
            uring_nice: std::env::var("STILLPASS_URING_NICE")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|n| (-20..=19).contains(n))
                .unwrap_or(d.uring_nice),
            #[cfg(feature = "fault-injection")]
            fault: Fault::from_env(),
        }
    }

    /// The failure to inject; always `None` without `fault-injection`.
    pub(crate) fn fault(&self) -> Option<Fault> {
        #[cfg(feature = "fault-injection")]
        return self.fault;
        #[cfg(not(feature = "fault-injection"))]
        None
    }
}

/// The requester, as the kernel reports it with every request. The seam for
/// per-process attribution: every recorded access passes through here.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Req {
    pub pid: u32,
}

struct Node {
    entry: Entry,
    size: u64,
    parent: u64,
    /// Children sorted bytewise by name: (name, ino).
    children: Vec<(String, u64)>,
}

/// What the step has touched. Deduplicated at insert; a step can issue
/// millions of reads of the same few thousand paths.
#[derive(Default)]
struct Log {
    present: HashSet<(u64, Kind)>,
    absent: HashSet<String>,
    /// The processes that asked. Attribution per process is later work;
    /// the set is what the seam carries today.
    pids: HashSet<u32>,
    /// I/O errors served to the step. Any makes the run a Stillpass error:
    /// the step saw a broken filesystem, not its inputs.
    io_errors: Vec<String>,
}

/// Request counts, reported at unmount so the bench can see what the step
/// actually asked the observer for.
pub(crate) struct Stats {
    lookup: AtomicU64,
    lookup_absent: AtomicU64,
    getattr: AtomicU64,
    readlink: AtomicU64,
    open: AtomicU64,
    read: AtomicU64,
    read_bytes: AtomicU64,
    readdir: AtomicU64,
    passthrough: AtomicBool,
    /// Set only once every ring queue registered and a probe request came
    /// back over the ring (see `uring::mount`).
    uring: AtomicBool,
    /// The ring was tried for this mount and abandoned before the step.
    uring_fallback: AtomicBool,
    /// Ring transport: requests answered over the ring, requests answered
    /// over the device (forgets and interrupts not counted, they have no
    /// reply), completion batches, batches that found every entry of the
    /// queue busy, and userspace time spent answering ring requests.
    pub(crate) ring_requests: AtomicU64,
    pub(crate) device_requests: AtomicU64,
    pub(crate) ring_batches: AtomicU64,
    pub(crate) ring_full_batches: AtomicU64,
    pub(crate) ring_serve_ns: AtomicU64,
    /// The highest (least favoured) nice value any ring queue thread
    /// actually runs at, `i32::MIN` until one reports, and whether lowering
    /// it was refused (no CAP_SYS_NICE).
    pub(crate) ring_nice: AtomicI32,
    pub(crate) ring_nice_denied: AtomicBool,
    /// The FUSE connection of a ring mount abandoned before serving (0:
    /// none), which `uring::mount` saw destroyed.
    abandoned_conn: AtomicU64,
    /// Times a serving thread found a model lock held by another.
    contended: AtomicU64,
}

impl Default for Stats {
    fn default() -> Stats {
        Stats {
            lookup: AtomicU64::new(0),
            lookup_absent: AtomicU64::new(0),
            getattr: AtomicU64::new(0),
            readlink: AtomicU64::new(0),
            open: AtomicU64::new(0),
            read: AtomicU64::new(0),
            read_bytes: AtomicU64::new(0),
            readdir: AtomicU64::new(0),
            passthrough: AtomicBool::new(false),
            uring: AtomicBool::new(false),
            uring_fallback: AtomicBool::new(false),
            ring_requests: AtomicU64::new(0),
            device_requests: AtomicU64::new(0),
            ring_batches: AtomicU64::new(0),
            ring_full_batches: AtomicU64::new(0),
            ring_serve_ns: AtomicU64::new(0),
            ring_nice: AtomicI32::new(i32::MIN),
            ring_nice_denied: AtomicBool::new(false),
            abandoned_conn: AtomicU64::new(0),
            contended: AtomicU64::new(0),
        }
    }
}

impl Stats {
    fn snapshot(&self) -> ObserverStats {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        ObserverStats {
            lookup: g(&self.lookup),
            lookup_absent: g(&self.lookup_absent),
            getattr: g(&self.getattr),
            readlink: g(&self.readlink),
            open: g(&self.open),
            read: g(&self.read),
            read_bytes: g(&self.read_bytes),
            readdir: g(&self.readdir),
            passthrough: self.passthrough.load(Ordering::Relaxed),
            uring: self.uring.load(Ordering::Relaxed),
            uring_fallback: self.uring_fallback.load(Ordering::Relaxed),
            ring_requests: g(&self.ring_requests),
            device_requests: g(&self.device_requests),
            ring_batches: g(&self.ring_batches),
            ring_full_batches: g(&self.ring_full_batches),
            ring_serve_us: g(&self.ring_serve_ns) / 1000,
            ring_nice: Some(self.ring_nice.load(Ordering::Relaxed)).filter(|&n| n != i32::MIN),
            ring_nice_denied: self.ring_nice_denied.load(Ordering::Relaxed),
            ring_abandoned_conn: Some(g(&self.abandoned_conn)).filter(|&c| c != 0),
            lock_contended: g(&self.contended),
        }
    }

    /// Forgets what an abandoned ring setup counted.
    fn reset_ring(&self) {
        for a in [
            &self.ring_requests,
            &self.device_requests,
            &self.ring_batches,
            &self.ring_full_batches,
            &self.ring_serve_ns,
        ] {
            a.store(0, Ordering::Relaxed);
        }
        self.ring_nice.store(i32::MIN, Ordering::Relaxed);
        self.ring_nice_denied.store(false, Ordering::Relaxed);
    }
}

/// A blob registered with the kernel as a passthrough backing file. fuser
/// closes its ids on drop; raw ids registered by the ring transport are
/// released with the connection.
pub(crate) enum Backing {
    Fuser(BackingId),
    Raw(u32),
}

impl Backing {
    pub(crate) fn raw(&self) -> Option<u32> {
        match self {
            Backing::Raw(id) => Some(*id),
            Backing::Fuser(_) => None,
        }
    }
}

/// How a transport registers a blob for passthrough.
pub(crate) type Register<'a> = &'a dyn Fn(&File) -> io::Result<Backing>;

/// An open file handle as the model hands it to a transport.
pub(crate) struct Opened {
    pub fh: u64,
    /// `Some` when the kernel is to read the blob directly.
    pub backing: Option<Arc<Backing>>,
}

/// One directory entry for a readdir reply.
pub(crate) struct DirEnt<'a> {
    pub ino: u64,
    pub off: u64,
    pub kind: FileType,
    pub name: &'a str,
}

pub(crate) struct CasFs {
    /// Index `ino - 1`. Inode numbers come from the tree's sorted order, so
    /// they are stable across lookups by construction (overlayfs needs that).
    nodes: Vec<Node>,
    cas: Cas,
    log: Arc<Mutex<Log>>,
    open: Mutex<HashMap<u64, Arc<File>>>,
    next_fh: AtomicU64,
    uid: u32,
    gid: u32,
    /// One backing id per inode, kept until unmount: the kernel refuses
    /// mixing backing files on one inode, and the id outliving every open
    /// is what fuser's docs ask for.
    backing: Mutex<HashMap<u64, Arc<Backing>>>,
    /// Whether the kernel granted FUSE_PASSTHROUGH at init.
    granted: AtomicBool,
    /// Whether opens use passthrough, decided at the first open for the
    /// whole mount (`None` until then). The kernel refuses to open an
    /// inode cached while it has passthrough handles, and vice versa, with
    /// an error this process never sees; switching modes mid-mount could
    /// therefore hide a failure from the log.
    mode: Mutex<Option<bool>>,
    /// Whether to ask the kernel for passthrough at all.
    want_passthrough: bool,
    stats: Arc<Stats>,
}

impl CasFs {
    fn new(
        tree: &Tree,
        cas: Cas,
        log: Arc<Mutex<Log>>,
        stats: Arc<Stats>,
        want_passthrough: bool,
    ) -> Result<CasFs, FsError> {
        let mut index: HashMap<&str, u64> = HashMap::new();
        let mut nodes = Vec::with_capacity(tree.len());
        // The root first, so it gets inode 1; then the rest in path order.
        let root = tree.get(ROOT).expect("tree without a root");
        for e in std::iter::once(root).chain(tree.iter().filter(|e| e.p != ROOT)) {
            let size = match e.k {
                Kind::File => {
                    let h = e.h.expect("file entry without hash");
                    std::fs::metadata(cas.blob_path(&h))
                        .map_err(FsError::io(format!("blob {h} for {}", e.p)))?
                        .len()
                }
                Kind::Symlink => e.t.as_ref().map_or(0, |t| t.len() as u64),
                _ => 0,
            };
            index.insert(e.p.as_str(), nodes.len() as u64 + 1);
            nodes.push(Node {
                entry: e.clone(),
                size,
                parent: ROOT_INO,
                children: Vec::new(),
            });
        }
        for i in 1..nodes.len() {
            let (parent, name) = sp_cas::parent_and_name(&nodes[i].entry.p);
            let (parent, name) = (index[parent], name.to_string());
            nodes[i].parent = parent;
            nodes[parent as usize - 1]
                .children
                .push((name, i as u64 + 1));
        }
        for n in &mut nodes {
            n.children
                .sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        }
        // SAFETY: getuid and getgid have no preconditions and cannot fail.
        #[allow(unsafe_code)] // the only way to ask the kernel who we are
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        Ok(CasFs {
            nodes,
            cas,
            log,
            open: Mutex::new(HashMap::new()),
            next_fh: AtomicU64::new(1),
            uid,
            gid,
            backing: Mutex::new(HashMap::new()),
            granted: AtomicBool::new(false),
            mode: Mutex::new(None),
            want_passthrough,
            stats,
        })
    }

    pub(crate) fn uid_gid(&self) -> (u32, u32) {
        (self.uid, self.gid)
    }

    pub(crate) fn file_count(&self) -> u64 {
        self.nodes.len() as u64
    }

    /// Whether the step wants passthrough at all (the bench's attribution
    /// series turns it off with STILLPASS_NO_PASSTHROUGH=1).
    pub(crate) fn passthrough_wanted(&self) -> bool {
        self.want_passthrough
    }

    pub(crate) fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Called by the transport after INIT: did the kernel grant passthrough.
    pub(crate) fn set_granted(&self, granted: bool) {
        self.granted.store(granted, Ordering::Relaxed);
        tracing::info!(target: "stillpass", passthrough_granted = granted, "observer init");
    }

    /// Called by the ring transport once every queue is registered and a
    /// request has come back over the ring; never earlier.
    pub(crate) fn set_uring(&self) {
        self.stats.uring.store(true, Ordering::Relaxed);
    }

    /// The ring was abandoned before serving anything: the classic
    /// transport takes over the whole mount.
    fn ring_abandoned(&self, conn: Option<u64>) {
        self.stats.uring.store(false, Ordering::Relaxed);
        self.stats.uring_fallback.store(true, Ordering::Relaxed);
        self.stats.reset_ring();
        self.stats
            .abandoned_conn
            .store(conn.unwrap_or(0), Ordering::Relaxed);
    }

    /// Whether anything has been recorded yet (a fallback must find none).
    fn log_is_empty(&self) -> bool {
        let log = lock(&self.log);
        log.present.is_empty() && log.absent.is_empty() && log.io_errors.is_empty()
    }

    /// Kernel passthrough (Linux 6.9+): once a file is open, the kernel reads
    /// its blob directly and no read request reaches this process. The open
    /// is still a request, and the open is what the log needs.
    ///
    /// Decided here, at the first open, for the whole mount: if the kernel
    /// granted it and this first blob registers, every later open must
    /// register too or the step gets EIO (logged, so the run is a Stillpass
    /// error); if the first registration fails, the mount serves reads in
    /// userspace throughout. No passthrough handle exists yet at that point,
    /// so neither choice can conflict with an open file.
    fn passthrough(&self, ino: u64, n: &Node, register: Register) -> bool {
        let mut mode = self.locked(&self.mode);
        if let Some(m) = *mode {
            return m;
        }
        let m = self.granted.load(Ordering::Relaxed)
            && match self.backing_id(ino, n, register) {
                Ok(_) => true,
                Err(e) => {
                    tracing::warn!(error = %e, "passthrough unavailable; serving reads in userspace");
                    false
                }
            };
        *mode = Some(m);
        self.stats.passthrough.store(m, Ordering::Relaxed);
        m
    }

    /// `lock`, counting the times another serving thread held it (lock
    /// contention between serving threads is request latency).
    fn locked<'a, T>(&self, m: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
        match m.try_lock() {
            Ok(g) => g,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                self.stats.contended.fetch_add(1, Ordering::Relaxed);
                lock(m)
            }
        }
    }

    fn node(&self, ino: u64) -> Option<&Node> {
        self.nodes.get((ino as usize).wrapping_sub(1))
    }

    /// Every access the log keeps goes through here (and `record_absent`),
    /// with the requester attached.
    fn record(&self, req: &Req, ino: u64, kind: Kind) {
        let mut log = self.locked(&self.log);
        log.present.insert((ino, kind));
        log.pids.insert(req.pid);
    }

    fn record_absent(&self, req: &Req, path: String) {
        let mut log = self.locked(&self.log);
        log.absent.insert(path);
        log.pids.insert(req.pid);
    }

    /// Something the step saw, or may have seen, that is not its inputs.
    /// Any makes the run a Stillpass error at unmount.
    pub(crate) fn io_error(&self, what: String) {
        tracing::error!(error = %what, "I/O error served to the step");
        lock(&self.log).io_errors.push(what);
    }

    fn attr(&self, ino: u64, n: &Node) -> FileAttr {
        let (kind, perm, nlink) = match n.entry.k {
            Kind::Dir => (FileType::Directory, 0o755, 2),
            Kind::Symlink => (FileType::Symlink, 0o777, 1),
            _ => (FileType::RegularFile, n.entry.m.unwrap_or(0o644) as u16, 1),
        };
        FileAttr {
            ino: INodeNo(ino),
            size: n.size,
            blocks: n.size.div_ceil(512),
            // Fixed times: the tree has none, and mtime is never an input.
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind,
            perm,
            nlink,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// The backing id for a file's blob, registered with the kernel on first
    /// use. The kernel takes its own reference to the blob, so the fd is
    /// closed again right away.
    fn backing_id(&self, ino: u64, n: &Node, register: Register) -> Result<Arc<Backing>, String> {
        if let Some(id) = self.locked(&self.backing).get(&ino) {
            return Ok(id.clone());
        }
        let file = self
            .cas
            .open_blob(&n.entry.h.unwrap())
            .map_err(|e| format!("open {}: {e}", n.entry.p))?;
        let id = register(&file)
            .map_err(|e| format!("register blob of {} for passthrough: {e}", n.entry.p))?;
        Ok(self
            .locked(&self.backing)
            .entry(ino)
            .or_insert_with(|| Arc::new(id))
            .clone())
    }

    // The operations. Each records what the access proves and returns what
    // the transport encodes; errors are errno values.

    pub(crate) fn lookup(
        &self,
        req: &Req,
        parent: u64,
        name: &[u8],
    ) -> Result<(u64, FileAttr), i32> {
        self.stats.lookup.fetch_add(1, Ordering::Relaxed);
        let p = self.node(parent).ok_or(libc::ENOENT)?;
        let found = std::str::from_utf8(name).ok().and_then(|name| {
            p.children
                .binary_search_by(|(n, _)| n.as_bytes().cmp(name.as_bytes()))
                .ok()
                .map(|i| p.children[i].1)
        });
        match found {
            Some(ino) => {
                let n = &self.nodes[ino as usize - 1];
                self.record(req, ino, n.entry.k);
                Ok((ino, self.attr(ino, n)))
            }
            None => {
                self.stats.lookup_absent.fetch_add(1, Ordering::Relaxed);
                // A name that isn't UTF-8 can never be in a tree (import
                // refuses them), so its lossy spelling is a safe record.
                let name = String::from_utf8_lossy(name);
                self.record_absent(req, child_path(&p.entry.p, &name));
                Err(libc::ENOENT)
            }
        }
    }

    pub(crate) fn getattr(&self, req: &Req, ino: u64) -> Result<FileAttr, i32> {
        self.stats.getattr.fetch_add(1, Ordering::Relaxed);
        let n = self.node(ino).ok_or(libc::ENOENT)?;
        self.record(req, ino, n.entry.k);
        Ok(self.attr(ino, n))
    }

    pub(crate) fn readlink(&self, req: &Req, ino: u64) -> Result<&str, i32> {
        self.stats.readlink.fetch_add(1, Ordering::Relaxed);
        let n = self.node(ino).ok_or(libc::ENOENT)?;
        match (&n.entry.k, &n.entry.t) {
            (Kind::Symlink, Some(t)) => {
                self.record(req, ino, Kind::Symlink);
                Ok(t)
            }
            _ => Err(libc::EINVAL),
        }
    }

    pub(crate) fn open(
        &self,
        req: &Req,
        ino: u64,
        read_only: bool,
        register: Register,
    ) -> Result<Opened, i32> {
        self.stats.open.fetch_add(1, Ordering::Relaxed);
        let n = self.node(ino).ok_or(libc::ENOENT)?;
        if n.entry.k != Kind::File {
            return Err(libc::EISDIR);
        }
        if !read_only {
            return Err(libc::EROFS);
        }
        self.record(req, ino, Kind::File);
        let fh = self.next_fh.fetch_add(1, Ordering::Relaxed);
        if self.passthrough(ino, n, register) {
            return match self.backing_id(ino, n, register) {
                Ok(backing) => Ok(Opened {
                    fh,
                    backing: Some(backing),
                }),
                Err(e) => {
                    // Never fall back mid-mount (see `passthrough`): the
                    // step sees an I/O error and the run is not recorded.
                    self.io_error(format!("open {}: {e}", n.entry.p));
                    Err(libc::EIO)
                }
            };
        }
        let file = match self.cas.open_blob(&n.entry.h.unwrap()) {
            Ok(f) => f,
            Err(e) => {
                self.io_error(format!("open {}: {e}", n.entry.p));
                return Err(libc::EIO);
            }
        };
        self.locked(&self.open).insert(fh, Arc::new(file));
        Ok(Opened { fh, backing: None })
    }

    pub(crate) fn read(
        &self,
        req: &Req,
        ino: u64,
        fh: u64,
        offset: u64,
        size: u32,
    ) -> Result<Vec<u8>, i32> {
        self.stats.read.fetch_add(1, Ordering::Relaxed);
        self.record(req, ino, Kind::File);
        let n = self.node(ino).ok_or(libc::ENOENT)?;
        let opened = self.locked(&self.open).get(&fh).cloned();
        let file = match opened {
            Some(f) => f,
            // A handle this process holds no fd for (a passthrough handle
            // the kernel chose to read through anyway, or a handle-less read
            // on a transport without open support): serve it from the blob.
            None => match n.entry.h.map(|h| self.cas.open_blob(&h)) {
                Some(Ok(f)) => Arc::new(f),
                Some(Err(e)) => {
                    self.io_error(format!("read {}: {e}", n.entry.p));
                    return Err(libc::EIO);
                }
                None => return Err(libc::EBADF),
            },
        };
        let mut buf = vec![0u8; size as usize];
        let mut filled = 0;
        while filled < buf.len() {
            match file.read_at(&mut buf[filled..], offset + filled as u64) {
                Ok(0) => break,
                Ok(k) => filled += k,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    self.io_error(format!("read {}: {e}", n.entry.p));
                    return Err(libc::EIO);
                }
            }
        }
        buf.truncate(filled);
        self.stats
            .read_bytes
            .fetch_add(filled as u64, Ordering::Relaxed);
        Ok(buf)
    }

    pub(crate) fn release(&self, fh: u64) {
        self.locked(&self.open).remove(&fh);
    }

    pub(crate) fn opendir(&self, ino: u64) -> Result<(), i32> {
        match self.node(ino) {
            Some(n) if n.entry.k == Kind::Dir => Ok(()),
            Some(_) => Err(libc::ENOTDIR),
            None => Err(libc::ENOENT),
        }
    }

    /// The entries of `ino` from `offset` on (`.` and `..` first); the
    /// transport takes as many as fit its reply.
    pub(crate) fn readdir(&self, req: &Req, ino: u64, offset: u64) -> Result<Vec<DirEnt<'_>>, i32> {
        self.stats.readdir.fetch_add(1, Ordering::Relaxed);
        let n = self.node(ino).ok_or(libc::ENOENT)?;
        self.record(req, ino, Kind::Dir);
        let dots = [(".", ino), ("..", n.parent)];
        Ok(dots
            .iter()
            .map(|(name, child)| (*name, *child))
            .chain(
                n.children
                    .iter()
                    .map(|(name, child)| (name.as_str(), *child)),
            )
            .enumerate()
            .skip(offset as usize)
            .map(|(i, (name, child))| DirEnt {
                ino: child,
                off: i as u64 + 1,
                kind: match self.nodes[child as usize - 1].entry.k {
                    Kind::Dir => FileType::Directory,
                    Kind::Symlink => FileType::Symlink,
                    _ => FileType::RegularFile,
                },
                name,
            })
            .collect())
    }

    /// Not an input. Doubles as a probe: `stat -f <mount>` logs the request
    /// counts so far.
    pub(crate) fn statfs(&self) {
        let s = self.stats.snapshot();
        tracing::info!(target: "stillpass", lookup = s.lookup, lookup_absent = s.lookup_absent, getattr = s.getattr, open = s.open, read = s.read, readdir = s.readdir, "observer statfs");
    }
}

fn errno(e: i32) -> Errno {
    match e {
        libc::ENOENT => Errno::ENOENT,
        libc::EISDIR => Errno::EISDIR,
        libc::EROFS => Errno::EROFS,
        libc::EBADF => Errno::EBADF,
        libc::EINVAL => Errno::EINVAL,
        libc::ENOTDIR => Errno::ENOTDIR,
        libc::ENOTSUP => Errno::ENOTSUP,
        _ => Errno::EIO,
    }
}

fn req(r: &Request) -> Req {
    Req { pid: r.pid() }
}

/// The classic transport: fuser decodes requests from `/dev/fuse` and
/// encodes the model's answers. The model is shared (`Arc`) so a mount the
/// ring transport gave up on can be served classically by the same model.
struct Classic(Arc<CasFs>);

impl Filesystem for Classic {
    fn init(&mut self, _req: &Request, config: &mut KernelConfig) -> io::Result<()> {
        // Each is independent; a kernel without one just keeps the default.
        // PARALLEL_DIROPS: lookups in one directory need not serialize.
        // CACHE_SYMLINKS: the kernel keeps a target after the first readlink.
        // NO_OPENDIR_SUPPORT: readdir is what the log needs; skip the opendir
        // round trip.
        for cap in [
            InitFlags::FUSE_PARALLEL_DIROPS,
            InitFlags::FUSE_CACHE_SYMLINKS,
            InitFlags::FUSE_NO_OPENDIR_SUPPORT,
        ] {
            let _ = config.add_capabilities(cap);
        }
        let granted = self.0.passthrough_wanted()
            && config.add_capabilities(InitFlags::FUSE_PASSTHROUGH).is_ok();
        if granted {
            // 1: blobs live on a plain filesystem (depth 0), and overlayfs
            // may stack on this mount (depth 2 is the kernel's maximum).
            config
                .set_max_stack_depth(1)
                .expect("1 is within the kernel's stacking limit");
        }
        self.0.set_granted(granted);
        Ok(())
    }

    fn lookup(&self, r: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        match self.0.lookup(&req(r), parent.0, name.as_bytes()) {
            Ok((_, attr)) => reply.entry(&TTL, &attr, Generation(0)),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn getattr(&self, r: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.0.getattr(&req(r), ino.0) {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn readlink(&self, r: &Request, ino: INodeNo, reply: ReplyData) {
        match self.0.readlink(&req(r), ino.0) {
            Ok(t) => reply.data(t.as_bytes()),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn open(&self, r: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let register = |file: &File| reply.open_backing(file).map(Backing::Fuser);
        let read_only = flags.acc_mode() == OpenAccMode::O_RDONLY;
        match self.0.open(&req(r), ino.0, read_only, &register) {
            Ok(Opened {
                fh,
                backing: Some(b),
            }) => match &*b {
                Backing::Fuser(id) => {
                    reply.opened_passthrough(FileHandle(fh), FopenFlags::empty(), id)
                }
                Backing::Raw(_) => unreachable!("the fuser transport registers fuser ids"),
            },
            Ok(Opened { fh, backing: None }) => {
                reply.opened(FileHandle(fh), FopenFlags::FOPEN_KEEP_CACHE)
            }
            Err(e) => reply.error(errno(e)),
        }
    }

    fn read(
        &self,
        r: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        match self.0.read(&req(r), ino.0, fh.0, offset, size) {
            Ok(data) => reply.data(&data),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        self.0.release(fh.0);
        reply.ok();
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        match self.0.opendir(ino.0) {
            Ok(()) => reply.opened(FileHandle(0), FopenFlags::empty()),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn readdir(
        &self,
        r: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        match self.0.readdir(&req(r), ino.0, offset) {
            Ok(entries) => {
                for d in entries {
                    if reply.add(INodeNo(d.ino), d.off, d.kind, d.name) {
                        break;
                    }
                }
                reply.ok();
            }
            Err(e) => reply.error(errno(e)),
        }
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: fuser::ReplyStatfs) {
        self.0.statfs();
        reply.statfs(0, 0, 0, self.0.file_count(), 0, 4096, 255, 4096);
    }

    fn getxattr(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _name: &OsStr,
        _size: u32,
        reply: ReplyXattr,
    ) {
        // Overlayfs probes lower xattrs; it accepts ENOTSUP cleanly.
        reply.error(Errno::ENOTSUP);
    }

    fn listxattr(&self, _req: &Request, _ino: INodeNo, _size: u32, reply: ReplyXattr) {
        reply.error(Errno::ENOTSUP);
    }
}

enum Session {
    Fuser(BackgroundSession),
    Uring(uring::Session),
}

impl Session {
    fn umount_and_join(self) -> io::Result<()> {
        match self {
            Session::Fuser(s) => s.umount_and_join(),
            Session::Uring(s) => s.umount_and_join(),
        }
    }
}

/// A mounted, logging, read-only view of a tree.
pub struct Served {
    session: Option<Session>,
    log: Arc<Mutex<Log>>,
    stats: Arc<Stats>,
    started: Instant,
    /// This process's (user, system) CPU time at mount, to report the
    /// delta: while mounted, the process does little besides serving.
    cpu_at_mount: (u64, u64),
    paths: Vec<(String, Kind)>,
    mountpoint: PathBuf,
}

/// Mounts `tree` read-only at `mountpoint`, serving content from `cas`,
/// with the options the environment names (`Options::from_env`).
pub fn serve(tree: &Tree, cas: &Cas, mountpoint: &Path) -> Result<Served, FsError> {
    serve_with(tree, cas, mountpoint, &Options::from_env())
}

/// `serve` with explicit options. The transport is chosen once, here,
/// before the step starts, for the whole mount: `Auto` takes the ring when
/// the host offers it and every queue comes up, else the classic loop; a
/// ring that fails to come up is torn down first (nothing served, nothing
/// recorded), so a mount is never served by both.
pub fn serve_with(
    tree: &Tree,
    cas: &Cas,
    mountpoint: &Path,
    opts: &Options,
) -> Result<Served, FsError> {
    let log = Arc::new(Mutex::new(Log::default()));
    let stats = Arc::new(Stats::default());
    let fs = Arc::new(CasFs::new(
        tree,
        cas.clone(),
        log.clone(),
        stats.clone(),
        opts.passthrough,
    )?);
    let paths = fs
        .nodes
        .iter()
        .map(|n| (n.entry.p.clone(), n.entry.k))
        .collect();
    let session = match opts.transport {
        Transport::Classic => Session::Fuser(mount_fuser(fs, mountpoint)?),
        Transport::Uring => {
            uring::available().map_err(|why| FsError::Io {
                what: format!("FUSE over io_uring at {}", mountpoint.display()),
                source: io::Error::other(why),
            })?;
            Session::Uring(uring::mount(fs, mountpoint, opts).map_err(MountError::into_inner)?)
        }
        Transport::Auto => match uring::available() {
            Ok(()) => match uring::mount(fs.clone(), mountpoint, opts) {
                Ok(s) => Session::Uring(s),
                // Only over a path the ring mount has fully left (`mount`
                // checked it: unmounted, not detached, connection gone,
                // every thread joined); never stacked on what it left.
                Err(MountError::Failed { error, conn }) => {
                    // Nothing was recorded (the step has not started).
                    if !fs.log_is_empty() {
                        return Err(FsError::Io {
                            what: format!(
                                "abandoned ring mount at {} logged accesses",
                                mountpoint.display()
                            ),
                            source: io::Error::other(error.to_string()),
                        });
                    }
                    tracing::warn!(target: "stillpass", error = %error, "FUSE over io_uring failed to come up; the classic transport serves this mount");
                    fs.ring_abandoned(conn);
                    Session::Fuser(mount_fuser(fs, mountpoint)?)
                }
                Err(MountError::NotTornDown(e)) => return Err(e),
            },
            Err(why) => {
                tracing::info!(target: "stillpass", why, "classic FUSE transport");
                Session::Fuser(mount_fuser(fs, mountpoint)?)
            }
        },
    };
    Ok(Served {
        session: Some(session),
        log,
        stats,
        started: Instant::now(),
        cpu_at_mount: cpu_ms(),
        paths,
        mountpoint: mountpoint.to_path_buf(),
    })
}

fn mount_fuser(fs: Arc<CasFs>, mountpoint: &Path) -> Result<BackgroundSession, FsError> {
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::FSName("stillpass".into()),
        MountOption::Subtype("cas".into()),
        MountOption::RO,
        MountOption::DefaultPermissions,
        MountOption::NoDev,
        MountOption::NoSuid,
    ];
    // allow_other: the sandbox runs as a different uid.
    config.acl = SessionACL::All;
    config.n_threads = Some(
        std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(16),
    );
    config.clone_fd = true;
    fuser::spawn_mount(Classic(fs), mountpoint, &config).map_err(FsError::io(format!(
        "mount FUSE at {}",
        mountpoint.display()
    )))
}

/// A lock that a panicking serving thread may have poisoned. The data is
/// still what it was (every critical section is a few inserts), and the
/// panic itself is recorded as a failure of the run.
pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Served {
    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    /// Request counts so far.
    pub fn stats(&self) -> ObserverStats {
        self.stats.snapshot()
    }

    /// The serving threads' scheduler statistics so far.
    pub fn sched(&self) -> SchedStats {
        serving_sched()
    }

    /// Unmounts, waits for in-flight requests, and returns the log,
    /// deduplicated and sorted. An error, and no log, when the step may
    /// have seen anything but its inputs: an I/O error served to it, a
    /// serving thread that died, a reply the kernel refused.
    ///
    /// A mount serves exactly one step. There is deliberately no way to
    /// drain the log and keep going: the kernel caches entries and
    /// attributes, so a second step on the same mount would read without
    /// any request reaching the log.
    pub fn unmount(mut self) -> Result<Vec<Access>, FsError> {
        let what = || format!("unmount {}", self.mountpoint.display());
        // Before the teardown joins the threads it reads.
        let sched = serving_sched();
        let unmounted = match self.session.take() {
            Some(s) => s.umount_and_join(),
            None => Ok(()),
        };
        let log = std::mem::take(&mut *lock(&self.log));
        self.report(log.pids.len(), sched);
        if !log.io_errors.is_empty() {
            return Err(FsError::Io {
                what: format!(
                    "the observer served {} I/O error(s) to the step, first: {}",
                    log.io_errors.len(),
                    log.io_errors[0]
                ),
                source: io::ErrorKind::Other.into(),
            });
        }
        unmounted.map_err(FsError::io(what()))?;
        let mut out: Vec<Access> = log
            .present
            .into_iter()
            .map(|(ino, kind)| Access {
                path: self.paths[ino as usize - 1].0.clone(),
                kind,
            })
            .chain(log.absent.into_iter().map(|path| Access {
                path,
                kind: Kind::Absent,
            }))
            .collect();
        out.sort();
        Ok(out)
    }

    /// One structured event with the request counts and this process's CPU
    /// time while mounted (nearly all of it is serving requests).
    fn report(&self, pids: usize, sched: SchedStats) {
        let s = self.stats.snapshot();
        let (user, sys) = cpu_ms();
        tracing::info!(
            target: "stillpass",
            mounted_ms = self.started.elapsed().as_millis() as u64,
            transport = s.transport(),
            uring_fallback = s.uring_fallback,
            pids,
            passthrough = s.passthrough,
            lookup = s.lookup,
            lookup_absent = s.lookup_absent,
            getattr = s.getattr,
            readlink = s.readlink,
            open = s.open,
            read = s.read,
            read_mb = s.read_bytes / (1 << 20),
            readdir = s.readdir,
            ring_requests = s.ring_requests,
            device_requests = s.device_requests,
            ring_batches = s.ring_batches,
            ring_full_batches = s.ring_full_batches,
            ring_serve_us = s.ring_serve_us,
            ring_nice = s.ring_nice,
            ring_nice_denied = s.ring_nice_denied,
            ring_abandoned_conn = s.ring_abandoned_conn,
            lock_contended = s.lock_contended,
            serve_user_ms = user.saturating_sub(self.cpu_at_mount.0),
            serve_sys_ms = sys.saturating_sub(self.cpu_at_mount.1),
            serve_threads = sched.threads,
            serve_run_ms = sched.run_ms,
            serve_runq_wait_ms = sched.wait_ms,
            serve_slices = sched.slices,
            "observer"
        );
    }
}

/// Sums /proc/self/task/*/schedstat over the threads that serve requests.
fn serving_sched() -> SchedStats {
    let (mut s, mut run_ns, mut wait_ns) = (SchedStats::default(), 0, 0);
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return s;
    };
    for t in tasks.flatten() {
        let comm = std::fs::read_to_string(t.path().join("comm")).unwrap_or_default();
        if !(comm.starts_with("fuser-") || comm.starts_with("sp-ring-")) {
            continue;
        }
        let Ok(st) = std::fs::read_to_string(t.path().join("schedstat")) else {
            continue;
        };
        let f: Vec<u64> = st
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        if let [run, wait, slices] = f[..] {
            s.threads += 1;
            run_ns += run;
            wait_ns += wait;
            s.slices += slices;
        }
    }
    s.run_ms = run_ns / 1_000_000;
    s.wait_ms = wait_ns / 1_000_000;
    s
}

/// This process's CPU time so far (user, system) in milliseconds.
fn cpu_ms() -> (u64, u64) {
    let mut ru: libc::rusage = unsafe_zeroed();
    // SAFETY: ru is a valid, writable rusage; RUSAGE_SELF cannot fail.
    #[allow(unsafe_code)] // getrusage(2) has no safe std wrapper
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    if rc != 0 {
        return (0, 0);
    }
    let ms = |t: libc::timeval| t.tv_sec as u64 * 1000 + t.tv_usec as u64 / 1000;
    (ms(ru.ru_utime), ms(ru.ru_stime))
}

fn unsafe_zeroed() -> libc::rusage {
    // SAFETY: rusage is a plain C struct of integers; all-zero is valid.
    #[allow(unsafe_code)] // no safe constructor for the libc struct
    unsafe {
        std::mem::zeroed()
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        if let Some(s) = self.session.take() {
            let _ = s.umount_and_join();
        }
    }
}
