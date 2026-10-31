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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

use crate::uring;
use crate::{FsError, ObserverStats};

/// Entries never change during a step, so the kernel may cache them freely.
/// Caching only removes repeat requests, which carry no new information.
pub(crate) const TTL: Duration = Duration::from_secs(3600);
const ROOT_INO: u64 = 1;

/// How requests reach the observer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// The ring when the host offers it (`enable_uring`, root, io_uring),
    /// else the classic device loop through fuser.
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
#[derive(Default)]
struct Stats {
    lookup: AtomicU64,
    lookup_absent: AtomicU64,
    getattr: AtomicU64,
    readlink: AtomicU64,
    open: AtomicU64,
    read: AtomicU64,
    read_bytes: AtomicU64,
    readdir: AtomicU64,
    passthrough: AtomicBool,
    uring: AtomicBool,
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
        }
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
    stats: Arc<Stats>,
}

impl CasFs {
    fn new(
        tree: &Tree,
        cas: Cas,
        log: Arc<Mutex<Log>>,
        stats: Arc<Stats>,
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
    pub(crate) fn passthrough_wanted() -> bool {
        std::env::var_os("STILLPASS_NO_PASSTHROUGH").is_none()
    }

    /// Called by the transport after INIT: did the kernel grant passthrough.
    pub(crate) fn set_granted(&self, granted: bool, uring: bool) {
        self.granted.store(granted, Ordering::Relaxed);
        self.stats.uring.store(uring, Ordering::Relaxed);
        tracing::info!(target: "stillpass", passthrough_granted = granted, uring, "observer init");
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
        let mut mode = self.mode.lock().unwrap();
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

    fn node(&self, ino: u64) -> Option<&Node> {
        self.nodes.get((ino as usize).wrapping_sub(1))
    }

    /// Every access the log keeps goes through here (and `record_absent`),
    /// with the requester attached.
    fn record(&self, req: &Req, ino: u64, kind: Kind) {
        let mut log = self.log.lock().unwrap();
        log.present.insert((ino, kind));
        log.pids.insert(req.pid);
    }

    fn record_absent(&self, req: &Req, path: String) {
        let mut log = self.log.lock().unwrap();
        log.absent.insert(path);
        log.pids.insert(req.pid);
    }

    fn io_error(&self, what: String) {
        tracing::error!(error = %what, "I/O error served to the step");
        self.log.lock().unwrap().io_errors.push(what);
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
        if let Some(id) = self.backing.lock().unwrap().get(&ino) {
            return Ok(id.clone());
        }
        let file = self
            .cas
            .open_blob(&n.entry.h.unwrap())
            .map_err(|e| format!("open {}: {e}", n.entry.p))?;
        let id = register(&file)
            .map_err(|e| format!("register blob of {} for passthrough: {e}", n.entry.p))?;
        Ok(self
            .backing
            .lock()
            .unwrap()
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
        self.open.lock().unwrap().insert(fh, Arc::new(file));
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
        let opened = self.open.lock().unwrap().get(&fh).cloned();
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
        self.open.lock().unwrap().remove(&fh);
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
/// encodes the model's answers.
impl Filesystem for CasFs {
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
        let granted = CasFs::passthrough_wanted()
            && config.add_capabilities(InitFlags::FUSE_PASSTHROUGH).is_ok();
        if granted {
            // 1: blobs live on a plain filesystem (depth 0), and overlayfs
            // may stack on this mount (depth 2 is the kernel's maximum).
            config
                .set_max_stack_depth(1)
                .expect("1 is within the kernel's stacking limit");
        }
        self.set_granted(granted, false);
        Ok(())
    }

    fn lookup(&self, r: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        match CasFs::lookup(self, &req(r), parent.0, name.as_bytes()) {
            Ok((_, attr)) => reply.entry(&TTL, &attr, Generation(0)),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn getattr(&self, r: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match CasFs::getattr(self, &req(r), ino.0) {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn readlink(&self, r: &Request, ino: INodeNo, reply: ReplyData) {
        match CasFs::readlink(self, &req(r), ino.0) {
            Ok(t) => reply.data(t.as_bytes()),
            Err(e) => reply.error(errno(e)),
        }
    }

    fn open(&self, r: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let register = |file: &File| reply.open_backing(file).map(Backing::Fuser);
        let read_only = flags.acc_mode() == OpenAccMode::O_RDONLY;
        match CasFs::open(self, &req(r), ino.0, read_only, &register) {
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
        match CasFs::read(self, &req(r), ino.0, fh.0, offset, size) {
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
        CasFs::release(self, fh.0);
        reply.ok();
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        match CasFs::opendir(self, ino.0) {
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
        match CasFs::readdir(self, &req(r), ino.0, offset) {
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
        CasFs::statfs(self);
        reply.statfs(0, 0, 0, self.file_count(), 0, 4096, 255, 4096);
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
/// over the transport `STILLPASS_FUSE_TRANSPORT` names (auto by default).
pub fn serve(tree: &Tree, cas: &Cas, mountpoint: &Path) -> Result<Served, FsError> {
    serve_with(tree, cas, mountpoint, Transport::from_env())
}

/// `serve` over a chosen transport. `Auto` takes the ring when the host
/// offers it and falls back to the classic loop before the step starts.
pub fn serve_with(
    tree: &Tree,
    cas: &Cas,
    mountpoint: &Path,
    transport: Transport,
) -> Result<Served, FsError> {
    let log = Arc::new(Mutex::new(Log::default()));
    let stats = Arc::new(Stats::default());
    let fs = CasFs::new(tree, cas.clone(), log.clone(), stats.clone())?;
    let paths = fs
        .nodes
        .iter()
        .map(|n| (n.entry.p.clone(), n.entry.k))
        .collect();
    let session = match transport {
        Transport::Classic => Session::Fuser(mount_fuser(fs, mountpoint)?),
        Transport::Uring => Session::Uring(uring::mount(Arc::new(fs), mountpoint)?),
        Transport::Auto => match uring::available() {
            Ok(()) => {
                let fs = Arc::new(fs);
                match uring::mount(fs.clone(), mountpoint) {
                    Ok(s) => Session::Uring(s),
                    Err(e) => {
                        tracing::warn!(error = %e, "FUSE over io_uring unavailable; using the classic transport");
                        let fs = Arc::into_inner(fs).expect("the failed mount dropped its threads");
                        Session::Fuser(mount_fuser(fs, mountpoint)?)
                    }
                }
            }
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

fn mount_fuser(fs: CasFs, mountpoint: &Path) -> Result<BackgroundSession, FsError> {
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
    fuser::spawn_mount(fs, mountpoint, &config).map_err(FsError::io(format!(
        "mount FUSE at {}",
        mountpoint.display()
    )))
}

impl Served {
    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    /// Request counts so far.
    pub fn stats(&self) -> ObserverStats {
        self.stats.snapshot()
    }

    /// Unmounts, waits for in-flight requests, and returns the log,
    /// deduplicated and sorted.
    ///
    /// A mount serves exactly one step. There is deliberately no way to
    /// drain the log and keep going: the kernel caches entries and
    /// attributes, so a second step on the same mount would read without
    /// any request reaching the log.
    pub fn unmount(mut self) -> Result<Vec<Access>, FsError> {
        let what = || format!("unmount {}", self.mountpoint.display());
        match self.session.take() {
            Some(Session::Fuser(s)) => s.umount_and_join().map_err(FsError::io(what()))?,
            Some(Session::Uring(s)) => s.umount_and_join().map_err(FsError::io(what()))?,
            None => {}
        }
        let log = std::mem::take(&mut *self.log.lock().unwrap());
        self.report(log.pids.len());
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
    fn report(&self, pids: usize) {
        let s = self.stats.snapshot();
        let (user, sys) = cpu_ms();
        tracing::info!(
            target: "stillpass",
            mounted_ms = self.started.elapsed().as_millis() as u64,
            transport = s.transport(),
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
            serve_user_ms = user.saturating_sub(self.cpu_at_mount.0),
            serve_sys_ms = sys.saturating_sub(self.cpu_at_mount.1),
            "observer"
        );
    }
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
        match self.session.take() {
            Some(Session::Fuser(s)) => {
                let _ = s.umount_and_join();
            }
            Some(Session::Uring(s)) => {
                let _ = s.umount_and_join();
            }
            None => {}
        }
    }
}
