//! Prototype, not part of the observer: a minimal read-only FUSE server for
//! a CAS tree with two transports, the classic `/dev/fuse` read/write loop
//! and FUSE over io_uring (Linux 6.14+, `enable_uring`), to measure what the
//! transport alone does to the request round trip on a given kernel. Same
//! server code, same replies, only the transport differs.
//!
//!     sp-uring-probe --repo DIR --mount DIR --transport uring --run 'xargs -a list stat'
//!
//! Both transports serve reads in userspace (no passthrough) and advertise
//! `FUSE_NO_OPEN_SUPPORT`, so the comparison is about lookups, getattrs,
//! readdirs and first reads: the requests sealing leaves on the observer.
#![allow(unsafe_code)] // prototype: raw FUSE ABI, mount(2), io_uring SQEs

use std::collections::HashMap;
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::time::Instant;

use anyhow::{Context, bail};
use clap::{Parser, ValueEnum};
use io_uring::{IoUring, cqueue, opcode, squeue, types};
use sp_cas::{Cas, Entry, Kind, ROOT, Tree};

#[derive(Parser)]
#[command(about = "Minimal FUSE server for a CAS tree: io_uring vs /dev/fuse transport probe")]
struct Args {
    /// The tree to serve (imported into a scratch CAS first).
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    mount: PathBuf,
    #[arg(long, value_enum, default_value_t = Transport::Uring)]
    transport: Transport,
    /// Ring entries per queue (uring).
    #[arg(long, default_value_t = 4)]
    depth: usize,
    /// Reader threads (classic).
    #[arg(long, default_value_t = 16)]
    threads: usize,
    /// Run this shell command inside the mount, print its wall time, unmount.
    #[arg(long)]
    run: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Transport {
    Classic,
    Uring,
}

// FUSE ABI (include/uapi/linux/fuse.h).
const FUSE_LOOKUP: u32 = 1;
const FUSE_FORGET: u32 = 2;
const FUSE_GETATTR: u32 = 3;
const FUSE_READLINK: u32 = 5;
const FUSE_OPEN: u32 = 14;
const FUSE_READ: u32 = 15;
const FUSE_STATFS: u32 = 17;
const FUSE_RELEASE: u32 = 18;
const FUSE_GETXATTR: u32 = 22;
const FUSE_LISTXATTR: u32 = 23;
const FUSE_FLUSH: u32 = 25;
const FUSE_INIT: u32 = 26;
const FUSE_OPENDIR: u32 = 27;
const FUSE_READDIR: u32 = 28;
const FUSE_RELEASEDIR: u32 = 29;
const FUSE_ACCESS: u32 = 34;
const FUSE_INTERRUPT: u32 = 36;
const FUSE_DESTROY: u32 = 38;
const FUSE_BATCH_FORGET: u32 = 42;

const FUSE_ASYNC_READ: u32 = 1 << 0;
const FUSE_NO_OPEN_SUPPORT: u32 = 1 << 17;
const FUSE_PARALLEL_DIROPS: u32 = 1 << 18;
const FUSE_MAX_PAGES: u32 = 1 << 22;
const FUSE_NO_OPENDIR_SUPPORT: u32 = 1 << 24;
const FUSE_INIT_EXT: u32 = 1 << 30;
/// Bit 41 of the 64-bit flag word: bit 9 of `flags2`.
const FUSE_OVER_IO_URING_2: u32 = 1 << 9;

const IN_HEADER: usize = 40;
const OUT_HEADER: usize = 16;
const MAX_WRITE: u32 = 128 * 1024;
const MAX_PAGES: u16 = 32;
/// The kernel's payload minimum: max(FUSE_MIN_READ_BUFFER, max_write, max_pages * PAGE_SIZE).
const PAYLOAD: usize = 128 * 1024;
/// fuse_uring_req_header: in_out[128] + op_in[128] + fuse_uring_ent_in_out (32).
const RING_HEADER: usize = 288;
const OP_IN_OFF: usize = 128;
const ENT_IN_OUT_OFF: usize = 256;
const IORING_OP_URING_CMD: u8 = 46;
const FUSE_IO_URING_CMD_REGISTER: u32 = 1;
const FUSE_IO_URING_CMD_COMMIT_AND_FETCH: u32 = 2;
/// FUSE_DEV_IOC_CLONE: _IOR(229, 0, uint32_t).
const FUSE_DEV_IOC_CLONE: libc::c_ulong = 0x8004_e500;

const TTL_SECS: u64 = 3600;

struct Node {
    entry: Entry,
    size: u64,
    parent: u64,
    children: Vec<(String, u64)>,
}

struct Fs {
    nodes: Vec<Node>,
    cas: Cas,
    uid: u32,
    gid: u32,
}

fn le_u32(b: &[u8]) -> u32 {
    u32::from_ne_bytes(b[..4].try_into().unwrap())
}
fn le_u64(b: &[u8]) -> u64 {
    u64::from_ne_bytes(b[..8].try_into().unwrap())
}
fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_ne_bytes());
}
fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_ne_bytes());
}

impl Fs {
    fn new(tree: &Tree, cas: Cas) -> anyhow::Result<Fs> {
        let mut index: HashMap<&str, u64> = HashMap::new();
        let mut nodes = Vec::with_capacity(tree.len());
        let root = tree.get(ROOT).context("tree without a root")?;
        for e in std::iter::once(root).chain(tree.iter().filter(|e| e.p != ROOT)) {
            let size = match e.k {
                Kind::File => std::fs::metadata(cas.blob_path(&e.h.unwrap()))?.len(),
                Kind::Symlink => e.t.as_ref().map_or(0, |t| t.len() as u64),
                _ => 0,
            };
            index.insert(e.p.as_str(), nodes.len() as u64 + 1);
            nodes.push(Node {
                entry: e.clone(),
                size,
                parent: 1,
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
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        Ok(Fs {
            nodes,
            cas,
            uid,
            gid,
        })
    }

    fn node(&self, ino: u64) -> Option<&Node> {
        self.nodes.get((ino as usize).wrapping_sub(1))
    }

    /// fuse_attr, 88 bytes.
    fn attr(&self, ino: u64, n: &Node, out: &mut Vec<u8>) {
        let (mode, nlink) = match n.entry.k {
            Kind::Dir => (libc::S_IFDIR | 0o755, 2),
            Kind::Symlink => (libc::S_IFLNK | 0o777, 1),
            _ => (libc::S_IFREG | n.entry.m.unwrap_or(0o644), 1),
        };
        put_u64(out, ino);
        put_u64(out, n.size);
        put_u64(out, n.size.div_ceil(512));
        put_u64(out, 0); // atime
        put_u64(out, 0); // mtime
        put_u64(out, 0); // ctime
        put_u32(out, 0);
        put_u32(out, 0);
        put_u32(out, 0);
        put_u32(out, mode);
        put_u32(out, nlink);
        put_u32(out, self.uid);
        put_u32(out, self.gid);
        put_u32(out, 0); // rdev
        put_u32(out, 4096); // blksize
        put_u32(out, 0); // flags
    }

    /// fuse_entry_out, 128 bytes.
    fn entry_out(&self, ino: u64, n: &Node, out: &mut Vec<u8>) {
        put_u64(out, ino);
        put_u64(out, 0); // generation
        put_u64(out, TTL_SECS);
        put_u64(out, TTL_SECS);
        put_u32(out, 0);
        put_u32(out, 0);
        self.attr(ino, n, out);
    }

    /// Handles one request. `op` is the op header (the first in-arg) and
    /// `payload` the in-args after it: over the ring they are two buffers
    /// (LOOKUP has a zero-size header and its name in the payload); the
    /// classic loop passes the bytes after the in-header for both.
    /// Returns (errno, out-args) or None when the op has no reply.
    fn handle(
        &self,
        opcode: u32,
        nodeid: u64,
        op: &[u8],
        payload: &[u8],
    ) -> Option<(i32, Vec<u8>)> {
        let mut out = Vec::new();
        let err = |e: i32| Some((-e, Vec::new()));
        match opcode {
            FUSE_LOOKUP => {
                let Some(p) = self.node(nodeid) else {
                    return err(libc::ENOENT);
                };
                let name = &payload[..payload
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(payload.len())];
                let found = p
                    .children
                    .binary_search_by(|(n, _)| n.as_bytes().cmp(name))
                    .ok()
                    .map(|i| p.children[i].1);
                match found {
                    Some(ino) => self.entry_out(ino, &self.nodes[ino as usize - 1], &mut out),
                    None => return err(libc::ENOENT),
                }
            }
            FUSE_GETATTR => {
                let Some(n) = self.node(nodeid) else {
                    return err(libc::ENOENT);
                };
                put_u64(&mut out, TTL_SECS);
                put_u32(&mut out, 0);
                put_u32(&mut out, 0);
                self.attr(nodeid, n, &mut out);
            }
            FUSE_READLINK => match self.node(nodeid) {
                Some(n) if n.entry.k == Kind::Symlink => {
                    out.extend_from_slice(n.entry.t.as_deref().unwrap_or("").as_bytes());
                }
                _ => return err(libc::EINVAL),
            },
            FUSE_OPEN | FUSE_OPENDIR => {
                // Not sent with NO_OPEN/NO_OPENDIR_SUPPORT; fh 0, no flags.
                put_u64(&mut out, 0);
                put_u32(&mut out, 0);
                put_u32(&mut out, 0);
            }
            FUSE_READ => {
                let Some(n) = self.node(nodeid) else {
                    return err(libc::ENOENT);
                };
                let (offset, size) = (le_u64(&op[8..]), le_u32(&op[16..]) as usize);
                let Some(h) = n.entry.h else {
                    return err(libc::EISDIR);
                };
                let Ok(f) = self.cas.open_blob(&h) else {
                    return err(libc::EIO);
                };
                out.resize(size, 0);
                let mut filled = 0;
                while filled < size {
                    match f.read_at(&mut out[filled..], offset + filled as u64) {
                        Ok(0) => break,
                        Ok(k) => filled += k,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => return err(libc::EIO),
                    }
                }
                out.truncate(filled);
            }
            FUSE_READDIR => {
                let Some(n) = self.node(nodeid) else {
                    return err(libc::ENOENT);
                };
                let (offset, size) = (le_u64(&op[8..]) as usize, le_u32(&op[16..]) as usize);
                let dots = [(".".to_string(), nodeid), ("..".to_string(), n.parent)];
                for (i, (name, child)) in dots
                    .iter()
                    .chain(n.children.iter())
                    .enumerate()
                    .skip(offset)
                {
                    let reclen = (24 + name.len()).div_ceil(8) * 8;
                    if out.len() + reclen > size {
                        break;
                    }
                    let kind = match self.nodes[*child as usize - 1].entry.k {
                        Kind::Dir => libc::DT_DIR,
                        Kind::Symlink => libc::DT_LNK,
                        _ => libc::DT_REG,
                    } as u32;
                    put_u64(&mut out, *child);
                    put_u64(&mut out, i as u64 + 1);
                    put_u32(&mut out, name.len() as u32);
                    put_u32(&mut out, kind);
                    out.extend_from_slice(name.as_bytes());
                    out.resize(out.len().div_ceil(8) * 8, 0);
                }
            }
            FUSE_STATFS => {
                let mut st = vec![0u8; 80];
                st[24..32].copy_from_slice(&(self.nodes.len() as u64).to_ne_bytes());
                st[40..44].copy_from_slice(&4096u32.to_ne_bytes());
                st[44..48].copy_from_slice(&255u32.to_ne_bytes());
                st[48..52].copy_from_slice(&4096u32.to_ne_bytes());
                out = st;
            }
            FUSE_RELEASE | FUSE_RELEASEDIR | FUSE_DESTROY => {}
            FUSE_GETXATTR | FUSE_LISTXATTR => return err(libc::ENOTSUP),
            FUSE_FLUSH | FUSE_ACCESS => return err(libc::ENOSYS),
            FUSE_FORGET | FUSE_BATCH_FORGET | FUSE_INTERRUPT => return None,
            _ => return err(libc::ENOSYS),
        }
        Some((0, out))
    }
}

fn mount(fd: RawFd, mountpoint: &Path) -> anyhow::Result<()> {
    let target = CString::new(mountpoint.as_os_str().as_bytes())?;
    let opts = CString::new(format!(
        "fd={fd},rootmode=40000,user_id=0,group_id=0,allow_other,default_permissions,max_read={MAX_WRITE}"
    ))?;
    let rc = unsafe {
        libc::mount(
            c"stillpass-probe".as_ptr(),
            target.as_ptr(),
            c"fuse".as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV | libc::MS_RDONLY,
            opts.as_ptr().cast(),
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error()).context("mount");
    }
    Ok(())
}

fn umount(mountpoint: &Path) {
    let target = CString::new(mountpoint.as_os_str().as_bytes()).unwrap();
    unsafe { libc::umount2(target.as_ptr(), libc::MNT_DETACH) };
}

fn read_fd(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EINTR) | Some(libc::EAGAIN) | Some(libc::ENOENT) => continue,
            _ => return Err(e),
        }
    }
}

fn write_reply(fd: RawFd, unique: u64, error: i32, data: &[u8]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(OUT_HEADER + data.len());
    put_u32(&mut buf, (OUT_HEADER + data.len()) as u32);
    buf.extend_from_slice(&error.to_ne_bytes());
    put_u64(&mut buf, unique);
    buf.extend_from_slice(data);
    let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The INIT handshake over the device; returns whether the kernel offered
/// FUSE over io_uring and we took it.
fn init(fd: RawFd, uring: bool) -> anyhow::Result<bool> {
    let mut buf = vec![0u8; 8192];
    let n = read_fd(fd, &mut buf)?;
    let opcode = le_u32(&buf[4..]);
    let unique = le_u64(&buf[8..]);
    if opcode != FUSE_INIT {
        bail!("first request is opcode {opcode}, not INIT");
    }
    let arg = &buf[IN_HEADER..n];
    let (major, minor) = (le_u32(arg), le_u32(&arg[4..]));
    let max_readahead = le_u32(&arg[8..]);
    let (flags, flags2) = (le_u32(&arg[12..]), le_u32(&arg[16..]));
    if major != 7 {
        bail!("kernel FUSE major {major}");
    }
    let want = FUSE_ASYNC_READ
        | FUSE_NO_OPEN_SUPPORT
        | FUSE_PARALLEL_DIROPS
        | FUSE_MAX_PAGES
        | FUSE_NO_OPENDIR_SUPPORT
        | FUSE_INIT_EXT;
    let offered_uring = flags2 & FUSE_OVER_IO_URING_2 != 0;
    let want2 = if uring { FUSE_OVER_IO_URING_2 } else { 0 };
    let mut out = Vec::with_capacity(64);
    put_u32(&mut out, 7);
    put_u32(&mut out, minor.min(47));
    put_u32(&mut out, max_readahead);
    put_u32(&mut out, want & flags);
    out.extend_from_slice(&16u16.to_ne_bytes()); // max_background
    out.extend_from_slice(&12u16.to_ne_bytes()); // congestion_threshold
    put_u32(&mut out, MAX_WRITE);
    put_u32(&mut out, 1); // time_gran
    out.extend_from_slice(&MAX_PAGES.to_ne_bytes());
    out.extend_from_slice(&0u16.to_ne_bytes()); // map_alignment
    put_u32(&mut out, want2 & flags2);
    put_u32(&mut out, 0); // max_stack_depth
    out.resize(64, 0);
    write_reply(fd, unique, 0, &out)?;
    eprintln!(
        "init: kernel 7.{minor}, flags {flags:#x} flags2 {flags2:#x}, io_uring offered {offered_uring}, taken {}",
        uring && offered_uring
    );
    if uring && !offered_uring {
        bail!("the kernel does not offer FUSE over io_uring (enable_uring?)");
    }
    Ok(uring && offered_uring)
}

/// The classic loop: read a request, reply with write.
fn classic_loop(fs: &Fs, fd: RawFd) {
    let mut buf = vec![0u8; PAYLOAD + 4096];
    loop {
        let n = match read_fd(fd, &mut buf) {
            Ok(n) => n,
            Err(_) => return, // ENODEV: unmounted
        };
        if n < IN_HEADER {
            continue;
        }
        let opcode = le_u32(&buf[4..]);
        let unique = le_u64(&buf[8..]);
        let nodeid = le_u64(&buf[16..]);
        let rest = &buf[IN_HEADER..n];
        if let Some((error, data)) = fs.handle(opcode, nodeid, rest, rest) {
            let _ = write_reply(fd, unique, error, &data);
        }
    }
}

fn clone_fd(fd: RawFd) -> anyhow::Result<OwnedFd> {
    let dev = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/fuse")?;
    let mut src = fd as u32;
    let rc = unsafe { libc::ioctl(dev.as_raw_fd(), FUSE_DEV_IOC_CLONE, &mut src) };
    if rc != 0 {
        return Err(io::Error::last_os_error()).context("FUSE_DEV_IOC_CLONE");
    }
    Ok(dev.into())
}

/// One ring entry: the kernel writes a request into `header` (in-header,
/// op header, ring entry in/out) and `payload`; the reply goes back the
/// same way.
struct Ent {
    header: Box<[u8]>,
    payload: Box<[u8]>,
    iov: Box<[libc::iovec; 2]>,
}

impl Ent {
    fn new() -> Ent {
        let mut header = vec![0u8; RING_HEADER].into_boxed_slice();
        let mut payload = vec![0u8; PAYLOAD].into_boxed_slice();
        let iov = Box::new([
            libc::iovec {
                iov_base: header.as_mut_ptr().cast(),
                iov_len: header.len(),
            },
            libc::iovec {
                iov_base: payload.as_mut_ptr().cast(),
                iov_len: payload.len(),
            },
        ]);
        Ent {
            header,
            payload,
            iov,
        }
    }
}

/// An IORING_OP_URING_CMD SQE for the fuse device: `sqe->addr` the two
/// iovecs, `sqe->len` 2, the 80-byte command a `fuse_uring_cmd_req`.
fn fuse_sqe(
    fd: RawFd,
    cmd_op: u32,
    qid: u16,
    commit_id: u64,
    ent: &Ent,
    ud: u64,
) -> squeue::Entry128 {
    let mut raw = [0u8; 128];
    raw[0] = IORING_OP_URING_CMD;
    raw[4..8].copy_from_slice(&fd.to_ne_bytes());
    raw[8..12].copy_from_slice(&cmd_op.to_ne_bytes());
    raw[16..24].copy_from_slice(&(ent.iov.as_ptr() as u64).to_ne_bytes());
    raw[24..28].copy_from_slice(&2u32.to_ne_bytes());
    raw[32..40].copy_from_slice(&ud.to_ne_bytes());
    // struct fuse_uring_cmd_req { flags, commit_id, qid, padding[6] } at 48.
    raw[56..64].copy_from_slice(&commit_id.to_ne_bytes());
    raw[64..66].copy_from_slice(&qid.to_ne_bytes());
    unsafe { std::mem::transmute::<[u8; 128], squeue::Entry128>(raw) }
}

/// Checks that transmuting bytes into an Entry128 lands where the crate's
/// own builder puts the opcode and cmd_op.
fn check_sqe_layout(fd: RawFd) -> anyhow::Result<()> {
    let built = opcode::UringCmd80::new(types::Fd(fd), 0x1234).build();
    let bytes = unsafe { std::mem::transmute::<squeue::Entry128, [u8; 128]>(built) };
    if bytes[0] != IORING_OP_URING_CMD || le_u32(&bytes[8..]) != 0x1234 {
        bail!("io_uring Entry128 layout differs from the raw sqe assumption");
    }
    Ok(())
}

fn pin_to_cpu(cpu: usize) {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

/// One queue: pinned to its CPU, `depth` entries registered, then
/// commit-and-fetch forever.
fn uring_queue(
    fs: Arc<Fs>,
    fd: RawFd,
    qid: u16,
    depth: usize,
    ready: Arc<Barrier>,
) -> anyhow::Result<()> {
    pin_to_cpu(qid as usize);
    // Declared before the ring so it outlives it: the kernel may still
    // reference the registered buffers while the ring tears down.
    let mut ents: Vec<Ent> = (0..depth).map(|_| Ent::new()).collect();
    let mut ring: IoUring<squeue::Entry128, cqueue::Entry32> =
        IoUring::builder().build(depth as u32)?;
    for (i, ent) in ents.iter().enumerate() {
        let sqe = fuse_sqe(fd, FUSE_IO_URING_CMD_REGISTER, qid, 0, ent, i as u64);
        unsafe { ring.submission().push(&sqe) }.map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    ring.submit()?;
    ready.wait();
    loop {
        ring.submit_and_wait(1)?;
        let done: Vec<(u64, i32)> = ring
            .completion()
            .map(|c| (c.user_data(), c.result()))
            .collect();
        for (ud, res) in done {
            if res < 0 {
                match -res {
                    libc::ENOTCONN | libc::ECANCELED | libc::ENODEV => return Ok(()),
                    e => bail!("queue {qid}: cqe error {}", io::Error::from_raw_os_error(e)),
                }
            }
            let ent = &mut ents[ud as usize];
            let (opcode, unique, nodeid) = (
                le_u32(&ent.header[4..]),
                le_u64(&ent.header[8..]),
                le_u64(&ent.header[16..]),
            );
            let commit_id = le_u64(&ent.header[ENT_IN_OUT_OFF + 8..]);
            let payload_sz = (le_u32(&ent.header[ENT_IN_OUT_OFF + 16..]) as usize).min(PAYLOAD);
            let reply = fs
                .handle(
                    opcode,
                    nodeid,
                    &ent.header[OP_IN_OFF..ENT_IN_OUT_OFF],
                    &ent.payload[..payload_sz],
                )
                .unwrap_or((0, Vec::new()));
            let (error, data) = reply;
            // Out header into in_out, out-args into the payload.
            ent.header[0..4].copy_from_slice(&((OUT_HEADER + data.len()) as u32).to_ne_bytes());
            ent.header[4..8].copy_from_slice(&error.to_ne_bytes());
            ent.header[8..16].copy_from_slice(&unique.to_ne_bytes());
            ent.payload[..data.len()].copy_from_slice(&data);
            ent.header[ENT_IN_OUT_OFF + 16..ENT_IN_OUT_OFF + 20]
                .copy_from_slice(&(data.len() as u32).to_ne_bytes());
            let sqe = fuse_sqe(
                fd,
                FUSE_IO_URING_CMD_COMMIT_AND_FETCH,
                qid,
                commit_id,
                ent,
                ud,
            );
            unsafe { ring.submission().push(&sqe) }.map_err(|e| anyhow::anyhow!("{e}"))?;
        }
    }
}

fn possible_cpus() -> anyhow::Result<usize> {
    let s = std::fs::read_to_string("/sys/devices/system/cpu/possible")?;
    let last = s.trim().rsplit('-').next().context("cpu range")?;
    Ok(last.parse::<usize>()? + 1)
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let scratch = tempfile_dir()?;
    let cas = Cas::open(scratch.join("cas"))?;
    let t = Instant::now();
    let tree = cas.import_dir(&args.repo)?.without_top_level(".git");
    eprintln!(
        "imported {} entries in {:.2}s",
        tree.len(),
        t.elapsed().as_secs_f64()
    );
    let fs = Arc::new(Fs::new(&tree, cas)?);

    std::fs::create_dir_all(&args.mount)?;
    let dev = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/fuse")
        .context("open /dev/fuse")?;
    let fd = dev.as_raw_fd();
    mount(fd, &args.mount)?;
    let uring = args.transport == Transport::Uring;
    let uring = match init(fd, uring) {
        Ok(u) => u,
        Err(e) => {
            umount(&args.mount);
            return Err(e);
        }
    };

    if uring {
        check_sqe_layout(fd)?;
        let nq = possible_cpus()?;
        let ready = Arc::new(Barrier::new(nq + 1));
        for qid in 0..nq {
            let (fs, ready) = (fs.clone(), ready.clone());
            std::thread::spawn(move || {
                if let Err(e) = uring_queue(fs, fd, qid as u16, args.depth, ready) {
                    eprintln!("queue {qid}: {e}");
                    std::process::exit(2);
                }
            });
        }
        ready.wait();
        // Forgets and interrupts still arrive over the device.
        let fs2 = fs.clone();
        std::thread::spawn(move || classic_loop(&fs2, fd));
        eprintln!("io_uring: {nq} queues x {} entries registered", args.depth);
    } else {
        for _ in 0..args.threads {
            let fs2 = fs.clone();
            let cfd = clone_fd(fd)?;
            std::thread::spawn(move || {
                classic_loop(&fs2, cfd.as_raw_fd());
                drop(cfd);
            });
        }
        eprintln!("classic: {} reader threads", args.threads);
    }
    // Let the ring become ready before the first request.
    std::thread::sleep(std::time::Duration::from_millis(200));

    if let Some(cmd) = &args.run {
        let t = Instant::now();
        let st = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&args.mount)
            .status()?;
        println!(
            "{} {cmd}: {:.3}s (exit {})",
            if uring { "uring" } else { "classic" },
            t.elapsed().as_secs_f64(),
            st.code().unwrap_or(-1)
        );
        umount(&args.mount);
        let _ = sp_cas::remove_any(&scratch);
        std::process::exit(0);
    }
    eprintln!("mounted at {}; Ctrl-C to stop", args.mount.display());
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

fn tempfile_dir() -> anyhow::Result<PathBuf> {
    let d = std::env::temp_dir().join(format!("sp-uring-probe-{}", std::process::id()));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

// Keep the device open for the process lifetime (threads hold raw fds).
#[allow(dead_code)]
fn keep(_: OwnedFd) {}
#[allow(dead_code)]
fn from_raw(fd: RawFd) -> OwnedFd {
    unsafe { OwnedFd::from_raw_fd(fd) }
}
