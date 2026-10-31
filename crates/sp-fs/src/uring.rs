//! FUSE over io_uring: the ring transport for `CasFs`.
//!
//! The mount and the INIT handshake are done here (fuser's session cannot
//! ask for `FUSE_OVER_IO_URING`), then one thread per possible CPU, pinned
//! to it, registers `DEPTH` ring entries and serves commit-and-fetch
//! forever; a request raised on CPU n is answered on CPU n, which is what
//! makes the round trip cheap. Forgets and interrupts still arrive over the
//! device, and so do requests before every queue is registered, so a
//! device reader runs alongside with the same model.
//!
//! Every request decodes to one `CasFs` call and encodes its answer, as the
//! fuser transport does, so the two cannot log differently.
//!
//! Needs root (mount(2) without fusermount3), `enable_uring` on the fuse
//! module and a kernel that offers the ring at INIT; `available` checks the
//! first two before anything is mounted, and a mount that fails afterwards
//! is undone before `serve` falls back to the classic transport.

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread::JoinHandle;

use fuser::{BackingId, FileAttr, FileType};
use io_uring::{IoUring, cqueue, opcode, squeue, types};

use crate::FsError;
use crate::serve::{Backing, CasFs, Opened, Req, TTL};

// include/uapi/linux/fuse.h
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
const FUSE_PARALLEL_DIROPS: u32 = 1 << 18;
const FUSE_MAX_PAGES: u32 = 1 << 22;
const FUSE_CACHE_SYMLINKS: u32 = 1 << 23;
const FUSE_NO_OPENDIR_SUPPORT: u32 = 1 << 24;
const FUSE_INIT_EXT: u32 = 1 << 30;
/// Bits 37 and 41 of the 64-bit flag word live in `flags2`.
const FUSE_PASSTHROUGH_2: u32 = 1 << (37 - 32);
const FUSE_OVER_IO_URING_2: u32 = 1 << (41 - 32);
const FOPEN_KEEP_CACHE: u32 = 1 << 1;
const FOPEN_PASSTHROUGH: u32 = 1 << 7;
const O_ACCMODE: u32 = 3;

const IN_HEADER: usize = 40;
const OUT_HEADER: usize = 16;
/// Read and readdir replies fit this; writes never happen (read-only).
const MAX_WRITE: u32 = 128 * 1024;
const MAX_PAGES: u16 = 32;
/// The kernel's per-entry payload minimum: max(8192, max_write, max_pages * 4096).
const PAYLOAD: usize = 128 * 1024;
/// fuse_uring_req_header: in_out[128], op_in[128], fuse_uring_ent_in_out (32).
const RING_HEADER: usize = 288;
const OP_IN: usize = 128;
const ENT_IN_OUT: usize = 256;
const IORING_OP_URING_CMD: u8 = 46;
const FUSE_IO_URING_CMD_REGISTER: u32 = 1;
const FUSE_IO_URING_CMD_COMMIT_AND_FETCH: u32 = 2;
/// Ring entries per queue.
const DEPTH: usize = 4;

/// Why the ring cannot be used on this host, checked before mounting.
pub(crate) fn available() -> Result<(), &'static str> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    #[allow(unsafe_code)] // mount(2) needs root; fusermount3 is the fallback
    if unsafe { libc::geteuid() } != 0 {
        return Err("not root");
    }
    match std::fs::read_to_string("/sys/module/fuse/parameters/enable_uring") {
        Ok(v) if v.trim() == "Y" => {}
        Ok(_) => return Err("fuse.enable_uring is off"),
        Err(_) => return Err("the fuse module has no enable_uring (no CONFIG_FUSE_IO_URING)"),
    }
    IoUring::<squeue::Entry128, cqueue::Entry32>::builder()
        .build(2)
        .map_err(|_| "io_uring with 128-byte SQEs unavailable")?;
    Ok(())
}

/// A ring-served mount: the device, its queue threads and the reader.
pub(crate) struct Session {
    mountpoint: PathBuf,
    dev: Arc<File>,
    threads: Vec<JoinHandle<()>>,
}

impl Session {
    /// Unmounts and waits for every thread: the kernel ends their
    /// outstanding commands and reads when the connection goes.
    pub(crate) fn umount_and_join(self) -> io::Result<()> {
        umount(&self.mountpoint)?;
        for t in self.threads {
            let _ = t.join();
        }
        drop(self.dev);
        Ok(())
    }
}

/// Mounts `fs` at `mountpoint` over the ring. On any failure nothing stays
/// mounted and the model is untouched except for `set_granted`.
pub(crate) fn mount(fs: Arc<CasFs>, mountpoint: &Path) -> Result<Session, FsError> {
    let io = |what: &str| FsError::io(format!("{what} ({})", mountpoint.display()));
    let dev = Arc::new(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/fuse")
            .map_err(io("open /dev/fuse"))?,
    );
    let (uid, gid) = fs.uid_gid();
    do_mount(dev.as_raw_fd(), mountpoint, uid, gid).map_err(io("mount FUSE over io_uring"))?;
    let undo = |e: FsError| {
        let _ = umount(mountpoint);
        e
    };
    let minor = init(&dev, &fs).map_err(|e| undo(io("FUSE init")(e)))?;
    check_sqe_layout().map_err(|e| undo(io("io_uring layout")(e)))?;

    let nq = possible_cpus().map_err(|e| undo(io("cpu count")(e)))?;
    let ready = Arc::new(Barrier::new(nq + 1));
    let (fail_tx, fail_rx) = mpsc::channel::<io::Error>();
    let mut threads = Vec::with_capacity(nq + 1);
    for qid in 0..nq {
        let (fs, dev, ready, fail_tx) = (fs.clone(), dev.clone(), ready.clone(), fail_tx.clone());
        threads.push(std::thread::spawn(move || {
            if let Err(e) = queue(&fs, &dev, qid as u16, minor, &ready) {
                let _ = fail_tx.send(e);
            }
        }));
    }
    drop(fail_tx);
    ready.wait();
    // Every queue registered its entries without error; the kernel switches
    // to the ring once each queue has one. Until then, and for forgets and
    // interrupts always, requests come over the device.
    if let Ok(e) = fail_rx.try_recv() {
        let _ = umount(mountpoint);
        for t in threads {
            let _ = t.join();
        }
        return Err(io("io_uring queue")(e));
    }
    let (fs2, dev2) = (fs.clone(), dev.clone());
    threads.push(std::thread::spawn(move || device_loop(&fs2, &dev2, minor)));
    tracing::info!(target: "stillpass", queues = nq, depth = DEPTH, "FUSE over io_uring");
    Ok(Session {
        mountpoint: mountpoint.to_path_buf(),
        dev,
        threads,
    })
}

fn do_mount(fd: i32, mountpoint: &Path, uid: u32, gid: u32) -> io::Result<()> {
    let target = CString::new(mountpoint.as_os_str().as_bytes())?;
    // allow_other: the sandbox runs as a different uid.
    let opts = CString::new(format!(
        "fd={fd},rootmode=40000,user_id={uid},group_id={gid},allow_other,default_permissions,max_read={MAX_WRITE}"
    ))?;
    // SAFETY: every pointer is a valid NUL-terminated string that outlives the call.
    #[allow(unsafe_code)] // mount(2) has no safe std wrapper
    let rc = unsafe {
        libc::mount(
            c"stillpass".as_ptr(),
            target.as_ptr(),
            c"fuse.cas".as_ptr(),
            libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
            opts.as_ptr().cast(),
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn umount(mountpoint: &Path) -> io::Result<()> {
    let target = CString::new(mountpoint.as_os_str().as_bytes())?;
    // SAFETY: target is a valid NUL-terminated path.
    #[allow(unsafe_code)] // umount2(2) has no safe std wrapper
    let rc = unsafe { libc::umount2(target.as_ptr(), 0) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_ne_bytes(b[off..off + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_ne_bytes(b[off..off + 8].try_into().unwrap())
}
fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_ne_bytes());
}
fn put64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_ne_bytes());
}

/// Reads one request from the device, retrying what FUSE says to retry.
fn read_dev(dev: &File, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match (&*dev).read(buf) {
            Ok(n) => return Ok(n),
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(libc::EINTR) | Some(libc::EAGAIN) | Some(libc::ENOENT)
                ) => {}
            Err(e) => return Err(e),
        }
    }
}

fn write_reply(dev: &File, unique: u64, error: i32, data: &[u8]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(OUT_HEADER + data.len());
    put32(&mut buf, (OUT_HEADER + data.len()) as u32);
    buf.extend_from_slice(&error.to_ne_bytes());
    put64(&mut buf, unique);
    buf.extend_from_slice(data);
    (&*dev).write_all(&buf)
}

/// The INIT handshake. Returns the kernel's minor version (reply sizes
/// depend on it). Fails when the kernel does not offer the ring.
fn init(dev: &File, fs: &CasFs) -> io::Result<u32> {
    let mut buf = vec![0u8; 8192];
    let n = read_dev(dev, &mut buf)?;
    if n < IN_HEADER + 20 || u32_at(&buf, 4) != FUSE_INIT {
        return Err(io::Error::other("the first request is not INIT"));
    }
    let unique = u64_at(&buf, 8);
    let arg = &buf[IN_HEADER..n];
    let (major, minor) = (u32_at(arg, 0), u32_at(arg, 4));
    let max_readahead = u32_at(arg, 8);
    let (flags, flags2) = (u32_at(arg, 12), u32_at(arg, 16));
    if major != 7 {
        let _ = write_reply(dev, unique, -libc::EPROTO, &[]);
        return Err(io::Error::other(format!("FUSE major {major}")));
    }
    if flags2 & FUSE_OVER_IO_URING_2 == 0 {
        // Decline the connection; the caller unmounts and falls back.
        let _ = write_reply(dev, unique, -libc::EPROTO, &[]);
        return Err(io::Error::other(
            "the kernel does not offer FUSE over io_uring",
        ));
    }
    // The same capabilities the fuser transport asks for (see serve.rs).
    let want = FUSE_ASYNC_READ
        | FUSE_PARALLEL_DIROPS
        | FUSE_MAX_PAGES
        | FUSE_CACHE_SYMLINKS
        | FUSE_NO_OPENDIR_SUPPORT
        | FUSE_INIT_EXT;
    let passthrough = CasFs::passthrough_wanted() && flags2 & FUSE_PASSTHROUGH_2 != 0;
    let want2 = FUSE_OVER_IO_URING_2 | if passthrough { FUSE_PASSTHROUGH_2 } else { 0 };
    let mut out = Vec::with_capacity(64);
    put32(&mut out, 7);
    put32(&mut out, minor.min(47));
    put32(&mut out, max_readahead);
    put32(&mut out, want & flags);
    out.extend_from_slice(&16u16.to_ne_bytes()); // max_background
    out.extend_from_slice(&12u16.to_ne_bytes()); // congestion_threshold
    put32(&mut out, MAX_WRITE);
    put32(&mut out, 1); // time_gran
    out.extend_from_slice(&MAX_PAGES.to_ne_bytes());
    out.extend_from_slice(&0u16.to_ne_bytes()); // map_alignment
    put32(&mut out, want2 & flags2);
    // max_stack_depth 1: blobs on a plain filesystem, overlayfs may stack on
    // this mount (the kernel's maximum is 2).
    put32(&mut out, if passthrough { 1 } else { 0 });
    out.resize(64, 0);
    write_reply(dev, unique, 0, &out)?;
    fs.set_granted(passthrough, true);
    Ok(minor)
}

/// One request's answer: errno (0 for success) and the out-args.
type Reply = Option<(i32, Vec<u8>)>;

/// Decodes one request and asks the model. `op` is the op header (the
/// first in-arg), `payload` the in-args after it; over the device both are
/// the bytes after the in-header. `None`: the op has no reply.
fn handle(fs: &CasFs, dev: &File, minor: u32, hdr: &[u8], op: &[u8], payload: &[u8]) -> Reply {
    let opcode = u32_at(hdr, 4);
    let nodeid = u64_at(hdr, 16);
    let req = Req {
        pid: u32_at(hdr, 32),
    };
    let ok = |data: Vec<u8>| Some((0, data));
    let err = |e: i32| Some((-e, Vec::new()));
    match opcode {
        FUSE_LOOKUP => {
            let name = &payload[..payload
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(payload.len())];
            match fs.lookup(&req, nodeid, name) {
                Ok((ino, attr)) => ok(entry_out(ino, &attr)),
                Err(e) => err(e),
            }
        }
        FUSE_GETATTR => match fs.getattr(&req, nodeid) {
            Ok(attr) => {
                let mut out = Vec::with_capacity(104);
                put64(&mut out, TTL.as_secs());
                put32(&mut out, 0);
                put32(&mut out, 0);
                put_attr(&mut out, &attr);
                ok(out)
            }
            Err(e) => err(e),
        },
        FUSE_READLINK => match fs.readlink(&req, nodeid) {
            Ok(t) => ok(t.as_bytes().to_vec()),
            Err(e) => err(e),
        },
        FUSE_OPEN => {
            let read_only = u32_at(op, 0) & O_ACCMODE == libc::O_RDONLY as u32;
            let register = |file: &File| BackingId::create_raw(dev, file).map(Backing::Raw);
            match fs.open(&req, nodeid, read_only, &register) {
                Ok(Opened { fh, backing }) => {
                    let mut out = Vec::with_capacity(24);
                    put64(&mut out, fh);
                    match backing.as_deref().and_then(Backing::raw) {
                        Some(id) => {
                            put32(&mut out, FOPEN_PASSTHROUGH);
                            put32(&mut out, id);
                            if minor >= 47 {
                                put64(&mut out, id as u64); // backing_id_64
                            }
                        }
                        None => {
                            put32(&mut out, FOPEN_KEEP_CACHE);
                            put32(&mut out, 0);
                            if minor >= 47 {
                                put64(&mut out, 0);
                            }
                        }
                    }
                    ok(out)
                }
                Err(e) => err(e),
            }
        }
        FUSE_READ => {
            let (fh, offset, size) = (u64_at(op, 0), u64_at(op, 8), u32_at(op, 16));
            match fs.read(&req, nodeid, fh, offset, size.min(PAYLOAD as u32)) {
                Ok(data) => ok(data),
                Err(e) => err(e),
            }
        }
        FUSE_RELEASE => {
            fs.release(u64_at(op, 0));
            ok(Vec::new())
        }
        FUSE_OPENDIR => match fs.opendir(nodeid) {
            Ok(()) => {
                let mut out = vec![0u8; if minor >= 47 { 24 } else { 16 }];
                out.truncate(if minor >= 47 { 24 } else { 16 });
                ok(out)
            }
            Err(e) => err(e),
        },
        FUSE_READDIR => {
            let (offset, size) = (u64_at(op, 8), u32_at(op, 16) as usize);
            match fs.readdir(&req, nodeid, offset) {
                Ok(entries) => {
                    let mut out = Vec::new();
                    for d in entries {
                        let reclen = (24 + d.name.len()).div_ceil(8) * 8;
                        if out.len() + reclen > size.min(PAYLOAD) {
                            break;
                        }
                        put64(&mut out, d.ino);
                        put64(&mut out, d.off);
                        put32(&mut out, d.name.len() as u32);
                        put32(&mut out, dirent_type(d.kind));
                        out.extend_from_slice(d.name.as_bytes());
                        out.resize(out.len().div_ceil(8) * 8, 0);
                    }
                    ok(out)
                }
                Err(e) => err(e),
            }
        }
        FUSE_RELEASEDIR | FUSE_DESTROY => ok(Vec::new()),
        FUSE_STATFS => {
            fs.statfs();
            let mut out = vec![0u8; 80];
            out[24..32].copy_from_slice(&fs.file_count().to_ne_bytes());
            out[40..44].copy_from_slice(&4096u32.to_ne_bytes());
            out[44..48].copy_from_slice(&255u32.to_ne_bytes());
            out[48..52].copy_from_slice(&4096u32.to_ne_bytes());
            ok(out)
        }
        // Overlayfs probes lower xattrs; it accepts ENOTSUP cleanly.
        FUSE_GETXATTR | FUSE_LISTXATTR => err(libc::ENOTSUP),
        FUSE_FLUSH | FUSE_ACCESS => err(libc::ENOSYS),
        FUSE_FORGET | FUSE_BATCH_FORGET | FUSE_INTERRUPT => None,
        _ => err(libc::ENOSYS),
    }
}

/// fuse_attr, 88 bytes.
fn put_attr(out: &mut Vec<u8>, a: &FileAttr) {
    let mode = match a.kind {
        FileType::Directory => libc::S_IFDIR,
        FileType::Symlink => libc::S_IFLNK,
        _ => libc::S_IFREG,
    } | a.perm as u32;
    put64(out, a.ino.0);
    put64(out, a.size);
    put64(out, a.blocks);
    put64(out, 0); // atime
    put64(out, 0); // mtime
    put64(out, 0); // ctime
    put32(out, 0);
    put32(out, 0);
    put32(out, 0);
    put32(out, mode);
    put32(out, a.nlink);
    put32(out, a.uid);
    put32(out, a.gid);
    put32(out, a.rdev);
    put32(out, a.blksize);
    put32(out, 0);
}

/// fuse_entry_out, 128 bytes.
fn entry_out(ino: u64, a: &FileAttr) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    put64(&mut out, ino);
    put64(&mut out, 0); // generation
    put64(&mut out, TTL.as_secs()); // entry_valid
    put64(&mut out, TTL.as_secs()); // attr_valid
    put32(&mut out, 0);
    put32(&mut out, 0);
    put_attr(&mut out, a);
    out
}

fn dirent_type(kind: FileType) -> u32 {
    (match kind {
        FileType::Directory => libc::DT_DIR,
        FileType::Symlink => libc::DT_LNK,
        _ => libc::DT_REG,
    }) as u32
}

/// The device loop: requests the kernel does not put on the ring (forgets,
/// interrupts, and everything before the ring is ready). Ends when the
/// connection is gone.
fn device_loop(fs: &CasFs, dev: &File, minor: u32) {
    let mut buf = vec![0u8; PAYLOAD + 4096];
    loop {
        let n = match read_dev(dev, &mut buf) {
            Ok(n) => n,
            Err(_) => return,
        };
        if n < IN_HEADER {
            continue;
        }
        let (hdr, rest) = buf[..n].split_at(IN_HEADER);
        if let Some((error, data)) = handle(fs, dev, minor, hdr, rest, rest) {
            let _ = write_reply(dev, u64_at(hdr, 8), error, &data);
        }
    }
}

/// One ring entry: the kernel writes a request into `header` (in-header,
/// op header, ring entry in/out) and `payload`; the reply goes back the
/// same way. The iovecs point into them for the lifetime of the queue.
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

/// An IORING_OP_URING_CMD SQE for the fuse device: `addr` the two iovecs,
/// `len` 2, the 80-byte command a `fuse_uring_cmd_req { flags, commit_id,
/// qid }`. Built raw because the io-uring crate's builder cannot set `len`.
fn fuse_sqe(
    fd: i32,
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
    raw[56..64].copy_from_slice(&commit_id.to_ne_bytes());
    raw[64..66].copy_from_slice(&qid.to_ne_bytes());
    // SAFETY: Entry128 is repr(C) over io_uring_sqe (64 bytes) and 64 more
    // bytes of command data, 128 bytes with no invalid bit patterns;
    // check_sqe_layout verified the opcode and cmd_op positions against
    // the crate's own builder before the first queue started.
    #[allow(unsafe_code)] // the crate has no raw constructor
    unsafe {
        std::mem::transmute::<[u8; 128], squeue::Entry128>(raw)
    }
}

/// Checks that bytes transmuted into an Entry128 land where the crate's own
/// builder puts the opcode and cmd_op.
fn check_sqe_layout() -> io::Result<()> {
    let built = opcode::UringCmd80::new(types::Fd(0), 0x1234).build();
    // SAFETY: the reverse of fuse_sqe's transmute; same size, plain bytes.
    #[allow(unsafe_code)] // reading the crate's layout back
    let bytes = unsafe { std::mem::transmute::<squeue::Entry128, [u8; 128]>(built) };
    if bytes[0] != IORING_OP_URING_CMD || u32_at(&bytes, 8) != 0x1234 {
        return Err(io::Error::other(
            "io_uring SQE layout differs from the raw assumption",
        ));
    }
    Ok(())
}

fn pin_to_cpu(cpu: usize) {
    // SAFETY: a zeroed cpu_set_t is a valid empty set; CPU_SET and
    // sched_setaffinity read and write only that set.
    #[allow(unsafe_code)] // no safe std affinity API
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

fn possible_cpus() -> io::Result<usize> {
    let s = std::fs::read_to_string("/sys/devices/system/cpu/possible")?;
    let last = s.trim().rsplit('-').next().unwrap_or("0");
    Ok(last.parse::<usize>().map_err(io::Error::other)? + 1)
}

/// One queue: pinned to its CPU, `DEPTH` entries registered, then
/// commit-and-fetch until the connection ends.
fn queue(fs: &CasFs, dev: &File, qid: u16, minor: u32, ready: &Barrier) -> io::Result<()> {
    pin_to_cpu(qid as usize);
    let fd = dev.as_raw_fd();
    let setup = || -> io::Result<(Vec<Ent>, IoUring<squeue::Entry128, cqueue::Entry32>)> {
        // The entries come first in the tuple and are declared first below,
        // so they outlive the ring: the kernel may still reference the
        // registered buffers while the ring tears down.
        let ents: Vec<Ent> = (0..DEPTH).map(|_| Ent::new()).collect();
        let mut ring = IoUring::builder().build(DEPTH as u32)?;
        for (i, ent) in ents.iter().enumerate() {
            let sqe = fuse_sqe(fd, FUSE_IO_URING_CMD_REGISTER, qid, 0, ent, i as u64);
            // SAFETY: the entry's buffers and iovecs live in `ents` for as
            // long as the ring, which this function owns.
            #[allow(unsafe_code)] // io_uring's push is unsafe by contract
            unsafe { ring.submission().push(&sqe) }.map_err(|_| io::Error::other("sq full"))?;
        }
        ring.submit()?;
        Ok((ents, ring))
    };
    let setup = setup();
    ready.wait();
    let (mut ents, mut ring) = setup?;
    loop {
        ring.submit_and_wait(1)?;
        let done: Vec<(u64, i32)> = ring
            .completion()
            .map(|c| (c.user_data(), c.result()))
            .collect();
        for (ud, res) in done {
            if res < 0 {
                // The connection is gone (unmount) or the command was
                // cancelled with it.
                return Ok(());
            }
            let ent = &mut ents[ud as usize];
            let unique = u64_at(&ent.header, 8);
            let commit_id = u64_at(&ent.header, ENT_IN_OUT + 8);
            let payload_sz = (u32_at(&ent.header, ENT_IN_OUT + 16) as usize).min(PAYLOAD);
            let (hdr, rest) = ent.header.split_at(OP_IN);
            let reply = handle(
                fs,
                dev,
                minor,
                hdr,
                &rest[..ENT_IN_OUT - OP_IN],
                &ent.payload[..payload_sz],
            );
            // Out header into in_out, out-args into the payload.
            let (error, data) = reply.unwrap_or((0, Vec::new()));
            ent.header[0..4].copy_from_slice(&((OUT_HEADER + data.len()) as u32).to_ne_bytes());
            ent.header[4..8].copy_from_slice(&error.to_ne_bytes());
            ent.header[8..16].copy_from_slice(&unique.to_ne_bytes());
            ent.payload[..data.len()].copy_from_slice(&data);
            ent.header[ENT_IN_OUT + 16..ENT_IN_OUT + 20]
                .copy_from_slice(&(data.len() as u32).to_ne_bytes());
            let sqe = fuse_sqe(
                fd,
                FUSE_IO_URING_CMD_COMMIT_AND_FETCH,
                qid,
                commit_id,
                ent,
                ud,
            );
            // SAFETY: as above; the entry outlives the ring.
            #[allow(unsafe_code)] // io_uring's push is unsafe by contract
            unsafe { ring.submission().push(&sqe) }.map_err(|_| io::Error::other("sq full"))?;
        }
    }
}
