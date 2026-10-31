use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use fuser::{
    BackgroundSession, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, LockOwner, MountOption, OpenAccMode, OpenFlags, ReplyAttr, ReplyData,
    ReplyDirectory, ReplyEntry, ReplyOpen, ReplyXattr, Request, SessionACL,
};
use sp_cas::{Cas, Entry, Kind, ROOT, Tree, child_path};
use sp_manifest::Access;

use crate::FsError;

/// Entries never change during a step, so the kernel may cache them freely.
/// Caching only removes repeat requests, which carry no new information.
const TTL: Duration = Duration::from_secs(3600);
const ROOT_INO: u64 = 1;

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
}

struct CasFs {
    /// Index `ino - 1`. Inode numbers come from the tree's sorted order, so
    /// they are stable across lookups by construction (overlayfs needs that).
    nodes: Vec<Node>,
    cas: Cas,
    log: Arc<Mutex<Log>>,
    open: Mutex<HashMap<u64, Arc<File>>>,
    next_fh: AtomicU64,
    uid: u32,
    gid: u32,
}

impl CasFs {
    fn new(tree: &Tree, cas: Cas, log: Arc<Mutex<Log>>) -> Result<CasFs, FsError> {
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
        })
    }

    fn node(&self, ino: INodeNo) -> Option<&Node> {
        self.nodes.get((ino.0 as usize).wrapping_sub(1))
    }

    fn record(&self, ino: u64, kind: Kind) {
        self.log.lock().unwrap().present.insert((ino, kind));
    }

    fn record_absent(&self, path: String) {
        self.log.lock().unwrap().absent.insert(path);
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
}

impl Filesystem for CasFs {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let Some(p) = self.node(parent) else {
            return reply.error(Errno::ENOENT);
        };
        let found = name.to_str().and_then(|name| {
            p.children
                .binary_search_by(|(n, _)| n.as_bytes().cmp(name.as_bytes()))
                .ok()
                .map(|i| p.children[i].1)
        });
        match found {
            Some(ino) => {
                let n = &self.nodes[ino as usize - 1];
                self.record(ino, n.entry.k);
                reply.entry(&TTL, &self.attr(ino, n), Generation(0));
            }
            None => {
                // A name that isn't UTF-8 can never be in a tree (import
                // refuses them), so its lossy spelling is a safe record.
                let name = String::from_utf8_lossy(name.as_bytes());
                self.record_absent(child_path(&p.entry.p, &name));
                reply.error(Errno::ENOENT);
            }
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        let Some(n) = self.node(ino) else {
            return reply.error(Errno::ENOENT);
        };
        self.record(ino.0, n.entry.k);
        reply.attr(&TTL, &self.attr(ino.0, n));
    }

    fn readlink(&self, _req: &Request, ino: INodeNo, reply: ReplyData) {
        let Some(n) = self.node(ino) else {
            return reply.error(Errno::ENOENT);
        };
        match (&n.entry.k, &n.entry.t) {
            (Kind::Symlink, Some(t)) => {
                self.record(ino.0, Kind::Symlink);
                reply.data(t.as_bytes());
            }
            _ => reply.error(Errno::EINVAL),
        }
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let Some(n) = self.node(ino) else {
            return reply.error(Errno::ENOENT);
        };
        if n.entry.k != Kind::File {
            return reply.error(Errno::EISDIR);
        }
        if flags.acc_mode() != OpenAccMode::O_RDONLY {
            return reply.error(Errno::EROFS);
        }
        self.record(ino.0, Kind::File);
        let file = match self.cas.open_blob(&n.entry.h.unwrap()) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(path = %n.entry.p, error = %e, "blob missing while serving");
                return reply.error(Errno::EIO);
            }
        };
        let fh = self.next_fh.fetch_add(1, Ordering::Relaxed);
        self.open.lock().unwrap().insert(fh, Arc::new(file));
        reply.opened(FileHandle(fh), FopenFlags::FOPEN_KEEP_CACHE);
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        self.record(ino.0, Kind::File);
        let Some(file) = self.open.lock().unwrap().get(&fh.0).cloned() else {
            return reply.error(Errno::EBADF);
        };
        let mut buf = vec![0u8; size as usize];
        let mut filled = 0;
        while filled < buf.len() {
            match file.read_at(&mut buf[filled..], offset + filled as u64) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return reply.error(Errno::EIO),
            }
        }
        reply.data(&buf[..filled]);
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
        self.open.lock().unwrap().remove(&fh.0);
        reply.ok();
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        match self.node(ino) {
            Some(n) if n.entry.k == Kind::Dir => reply.opened(FileHandle(0), FopenFlags::empty()),
            Some(_) => reply.error(Errno::ENOTDIR),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let Some(n) = self.node(ino) else {
            return reply.error(Errno::ENOENT);
        };
        self.record(ino.0, Kind::Dir);
        let dots = [(".".to_string(), ino.0), ("..".to_string(), n.parent)];
        let all = dots.iter().chain(n.children.iter());
        for (i, (name, child)) in all.enumerate().skip(offset as usize) {
            let kind = match self.nodes[*child as usize - 1].entry.k {
                Kind::Dir => FileType::Directory,
                Kind::Symlink => FileType::Symlink,
                _ => FileType::RegularFile,
            };
            if reply.add(INodeNo(*child), i as u64 + 1, kind, name) {
                break;
            }
        }
        reply.ok();
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

/// A mounted, logging, read-only view of a tree.
pub struct Served {
    session: Option<BackgroundSession>,
    log: Arc<Mutex<Log>>,
    paths: Vec<(String, Kind)>,
    mountpoint: PathBuf,
}

/// Mounts `tree` read-only at `mountpoint`, serving content from `cas`.
pub fn serve(tree: &Tree, cas: &Cas, mountpoint: &Path) -> Result<Served, FsError> {
    let log = Arc::new(Mutex::new(Log::default()));
    let fs = CasFs::new(tree, cas.clone(), log.clone())?;
    let paths = fs
        .nodes
        .iter()
        .map(|n| (n.entry.p.clone(), n.entry.k))
        .collect();
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
    let session = fuser::spawn_mount(fs, mountpoint, &config).map_err(FsError::io(format!(
        "mount FUSE at {}",
        mountpoint.display()
    )))?;
    Ok(Served {
        session: Some(session),
        log,
        paths,
        mountpoint: mountpoint.to_path_buf(),
    })
}

impl Served {
    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    /// Everything logged so far, deduplicated, without clearing it.
    pub fn accesses(&self) -> Vec<Access> {
        let log = self.log.lock().unwrap();
        let mut out: Vec<Access> = log
            .present
            .iter()
            .map(|(ino, kind)| Access {
                path: self.paths[*ino as usize - 1].0.clone(),
                kind: *kind,
            })
            .chain(log.absent.iter().map(|p| Access {
                path: p.clone(),
                kind: Kind::Absent,
            }))
            .collect();
        out.sort();
        out
    }

    /// Takes the log, leaving it empty.
    pub fn drain(&self) -> Vec<Access> {
        let out = self.accesses();
        *self.log.lock().unwrap() = Log::default();
        out
    }

    /// Unmounts and returns the final log.
    pub fn unmount(mut self) -> Result<Vec<Access>, FsError> {
        let out = self.accesses();
        if let Some(s) = self.session.take() {
            s.umount_and_join().map_err(FsError::io(format!(
                "unmount {}",
                self.mountpoint.display()
            )))?;
        }
        Ok(out)
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        if let Some(s) = self.session.take() {
            let _ = s.umount_and_join();
        }
    }
}
