//! FUSE over io_uring: the ring transport for `CasFs`.
//!
//! The mount and the INIT handshake are done here (fuser's session cannot
//! ask for `FUSE_OVER_IO_URING`). Then one thread per possible CPU (the
//! kernel makes one queue per possible CPU and queues a request raised on
//! CPU n to queue n), pinned to that CPU, registers its ring entries and
//! answers commit-and-fetch until the connection ends. A device reader runs
//! alongside for what the kernel never puts on the ring (forgets,
//! interrupts).
//!
//! Coming up is all or nothing and happens before the step starts. Every
//! queue reports its registration (the kernel completes a refused one at
//! once with an error; an accepted one stays pending until a request
//! arrives), then a statfs probe must come back over the ring: the kernel
//! holds every request until each queue has an entry, so the probe returning
//! over the ring proves the ring is ready. Any failure aborts and unmounts
//! the mount and joins every thread; `serve` then mounts again with the
//! classic transport. Only then is the mount marked ring-served.
//!
//! Once serving, a queue thread or the device reader that fails (an error
//! completion, a panic, a reply the kernel refuses) records an I/O error,
//! which `Served::unmount` turns into a Stillpass error, and aborts the
//! connection: the requests a dead thread would have answered fail at once
//! instead of blocking the step forever.
//!
//! Every request decodes to one `CasFs` call and encodes its answer, as the
//! fuser transport does, so the two log the same accesses
//! (`transports_log_identically` in sp-cli's tests compares them).
//!
//! Needs root (mount(2) without fusermount3), `enable_uring` on the fuse
//! module and a kernel that offers the ring at INIT with a protocol minor
//! this file encodes for (`MIN_MINOR..=TESTED_MINOR`). `available` checks
//! the first two before anything is mounted.

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use fuser::{BackingId, FileAttr, FileType};
use io_uring::{IoUring, cqueue, opcode, squeue, types};

use crate::FsError;
use crate::serve::{Backing, CasFs, DirEnt, Fault, Opened, Options, Req, TTL, lock};

/// The FUSE protocol minor this transport's encoders are written for and
/// tested against: Linux 7.0's `FUSE_KERNEL_MINOR_VERSION`. A kernel that
/// speaks a newer minor gets the classic transport. Over the ring the kernel
/// never tells the daemon that it rejected a reply (the requester just gets
/// an error), so a layout change in a newer minor would be invisible here.
pub(crate) const TESTED_MINOR: u32 = 45;
/// FUSE over io_uring arrived in 7.42. From 7.42 to 7.45 none of the
/// structs this file encodes changed (include/uapi/linux/fuse.h changelog:
/// 7.43 adds `request_timeout` inside the 64-byte `fuse_init_out`, sent as
/// zero; 7.44 and 7.45 add notifications and copy_file_range_64).
const MIN_MINOR: u32 = 42;

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
/// The capabilities the fuser transport asks for (see serve.rs), plus what
/// the handshake itself needs.
const WANT: u32 = FUSE_ASYNC_READ
    | FUSE_PARALLEL_DIROPS
    | FUSE_MAX_PAGES
    | FUSE_CACHE_SYMLINKS
    | FUSE_NO_OPENDIR_SUPPORT
    | FUSE_INIT_EXT;
const FOPEN_KEEP_CACHE: u32 = 1 << 1;
const FOPEN_PASSTHROUGH: u32 = 1 << 7;
const O_ACCMODE: u32 = 3;

/// sizeof(struct fuse_in_header), sizeof(struct fuse_out_header).
const IN_HEADER: usize = 40;
const OUT_HEADER: usize = 16;
/// Reply sizes at 7.42-7.45: fuse_attr, fuse_entry_out, fuse_attr_out,
/// fuse_open_out, fuse_statfs_out, fuse_init_out; fuse_dirent's name offset.
const ATTR: usize = 88;
const ENTRY_OUT: usize = 40 + ATTR;
const ATTR_OUT: usize = 16 + ATTR;
const OPEN_OUT: usize = 16;
const STATFS_OUT: usize = 80;
const INIT_OUT: usize = 64;
const DIRENT_NAME: usize = 24;
/// Read and readdir replies fit this; writes never happen (read-only).
const MAX_WRITE: u32 = 128 * 1024;
const MAX_PAGES: u16 = 32;
/// The kernel's per-entry payload minimum: max(8192, max_write, max_pages * 4096).
const PAYLOAD: usize = 128 * 1024;
/// struct fuse_uring_req_header: in_out[128], op_in[128], then
/// struct fuse_uring_ent_in_out { flags, commit_id, payload_sz, padding, reserved }.
const RING_HEADER: usize = 288;
const OP_IN: usize = 128;
const ENT_IN_OUT: usize = 256;
const IORING_OP_URING_CMD: u8 = 46;
const FUSE_IO_URING_CMD_REGISTER: u32 = 1;
const FUSE_IO_URING_CMD_COMMIT_AND_FETCH: u32 = 2;
/// FUSE requires exactly two iovecs per entry (headers, payload).
const FUSE_URING_IOV_SEGS: u32 = 2;
/// Every queue reports its registration within this, or the ring is
/// abandoned (registration is a few syscalls; this is a backstop).
const SETUP_TIMEOUT: Duration = Duration::from_secs(30);
/// The readiness probe (one statfs) comes back within this, or the ring is
/// abandoned. A false timeout costs only the fallback to classic.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Queues registering, probe in flight: a failure means fallback.
    Setup,
    /// The step may be running: a failure is a Stillpass error.
    Serving,
    /// Teardown: errors from the connection going away are expected.
    Stopping,
}

struct Health {
    phase: Phase,
    failures: Vec<String>,
}

/// What the queue threads, the device reader and the session share.
struct Shared {
    fs: Arc<CasFs>,
    dev: File,
    mountpoint: PathBuf,
    health: Mutex<Health>,
    fault: Option<Fault>,
    /// The serving-time test hook, armed when serving starts (0 none, 1
    /// panic, 2 error completion), fired by the ring request that finds
    /// `countdown` at zero.
    armed: AtomicU8,
    countdown: AtomicU64,
    per_queue: Vec<AtomicU64>,
}

impl Shared {
    fn stopping(&self) -> bool {
        lock(&self.health).phase == Phase::Stopping
    }

    /// A serving thread failed. While serving: an I/O error for the run and
    /// an aborted connection, so nothing waits on the dead thread. During
    /// setup: noted for `mount`, which falls back. During teardown: ignored.
    /// Only the first failure counts; the abort makes every other thread
    /// fail after it.
    fn fail(&self, what: String) {
        let mut h = lock(&self.health);
        if h.phase == Phase::Stopping || !h.failures.is_empty() {
            return;
        }
        h.failures.push(what.clone());
        let serving = h.phase == Phase::Serving;
        drop(h);
        if serving {
            self.fs.io_error(format!("FUSE over io_uring: {what}"));
        } else {
            tracing::warn!(error = %what, "FUSE over io_uring failed during setup");
        }
        abort(&self.mountpoint);
    }

    /// Setup is over: from here a failure is the run's.
    fn enter_serving(&self) -> io::Result<()> {
        let mut h = lock(&self.health);
        if let Some(f) = h.failures.first() {
            return Err(io::Error::other(f.clone()));
        }
        h.phase = Phase::Serving;
        let (armed, after) = match self.fault {
            Some(Fault::QueuePanic(n)) => (1, n),
            Some(Fault::QueueError(n)) => (2, n),
            _ => (0, 0),
        };
        self.countdown.store(after, Ordering::Relaxed);
        self.armed.store(armed, Ordering::Relaxed);
        Ok(())
    }

    fn take_armed(&self) -> u8 {
        if self.armed.load(Ordering::Relaxed) == 0
            || self.countdown.fetch_sub(1, Ordering::Relaxed) != 0
        {
            return 0;
        }
        self.armed.swap(0, Ordering::Relaxed)
    }
}

/// A ring-served mount: its shared state and every thread it started.
pub(crate) struct Session {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl Session {
    /// Unmounts and waits for every thread. A plain unmount ends every
    /// queue's commands and the device read; if it fails (the mount is
    /// busy), or a failure already aborted the connection, the connection
    /// is aborted and the mount detached first, so no join can wait on a
    /// thread the kernel still holds.
    pub(crate) fn umount_and_join(self) -> io::Result<()> {
        let r = stop(&self.shared, false);
        join(self.threads);
        let per_queue: Vec<u64> = self
            .shared
            .per_queue
            .iter()
            .map(|n| n.load(Ordering::Relaxed))
            .collect();
        tracing::info!(target: "stillpass", per_queue = ?per_queue, "ring queues");
        r
    }
}

/// Ends serving (see `Session::umount_and_join`). `force`: abort and
/// detach without trying a plain unmount (a mount that never came up).
fn stop(sh: &Shared, force: bool) -> io::Result<()> {
    let failed = {
        let mut h = lock(&sh.health);
        h.phase = Phase::Stopping;
        !h.failures.is_empty()
    };
    if force || failed {
        force_unmount(&sh.mountpoint);
        return Ok(());
    }
    match umount2(&sh.mountpoint, 0) {
        Ok(()) => Ok(()),
        Err(e) => {
            force_unmount(&sh.mountpoint);
            Err(e)
        }
    }
}

fn join(threads: Vec<JoinHandle<()>>) {
    for t in threads {
        // Every thread catches its own panics; a join error cannot carry
        // anything the failure record does not already have.
        let _ = t.join();
    }
}

/// Aborts the FUSE connection: every request waiting on the mount, or
/// held by a thread that will never answer it, fails at once
/// (ECONNABORTED/ENOTCONN), and every queue's commands and the device read
/// end. umount2(MNT_FORCE) calls FUSE's umount_begin, which aborts the
/// connection before the busy check, so this works with the overlay still
/// stacked on the mount (the unmount itself may then fail; ignored).
fn abort(mountpoint: &Path) {
    let _ = umount2(mountpoint, libc::MNT_FORCE);
}

fn force_unmount(mountpoint: &Path) {
    abort(mountpoint);
    let _ = umount2(mountpoint, libc::MNT_DETACH);
}

/// A mount that has not come up yet. Dropped before `done`, it aborts and
/// detaches the mount and joins every thread it started, so a failed setup
/// leaves nothing mounted, nothing running and the model unreferenced.
struct Starting {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    done: bool,
}

impl Drop for Starting {
    fn drop(&mut self) {
        if !self.done {
            let _ = stop(&self.shared, true);
            join(std::mem::take(&mut self.threads));
        }
    }
}

/// Mounts `fs` at `mountpoint` over the ring. On any failure nothing stays
/// mounted, every thread is joined, and the model is untouched except for
/// `set_granted` (which the classic transport sets again).
pub(crate) fn mount(fs: Arc<CasFs>, mountpoint: &Path, opts: &Options) -> Result<Session, FsError> {
    let io = |what: &str| FsError::io(format!("{what} ({})", mountpoint.display()));
    let cpus = possible_cpus().map_err(io("possible CPUs"))?;
    let dev = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/fuse")
        .map_err(io("open /dev/fuse"))?;
    let (uid, gid) = fs.uid_gid();
    do_mount(dev.as_raw_fd(), mountpoint, uid, gid).map_err(io("mount FUSE over io_uring"))?;
    let nq = cpus.len();
    let mut st = Starting {
        shared: Arc::new(Shared {
            fs,
            dev,
            mountpoint: mountpoint.to_path_buf(),
            health: Mutex::new(Health {
                phase: Phase::Setup,
                failures: Vec::new(),
            }),
            fault: opts.fault,
            armed: AtomicU8::new(0),
            countdown: AtomicU64::new(0),
            per_queue: (0..nq).map(|_| AtomicU64::new(0)).collect(),
        }),
        threads: Vec::new(),
        done: false,
    };
    let sh = st.shared.clone();
    let passthrough = init(&sh.dev, sh.fs.passthrough_wanted()).map_err(io("FUSE init"))?;
    sh.fs.set_granted(passthrough);
    check_sqe_layout().map_err(io("io_uring SQE layout"))?;

    let (tx, rx) = mpsc::channel::<(u16, io::Result<()>)>();
    for qid in 0..nq {
        let cfg = QueueCfg {
            qid: qid as u16,
            // Queue n serves requests raised on CPU n (kernel: qid =
            // task_cpu). With a sparse possible mask some queue ids are not
            // CPUs; those queues get no traffic and stay unpinned.
            cpu: (opts.uring_pin && cpus.contains(&qid)).then_some(qid),
            depth: opts.uring_depth,
            nice: opts.uring_nice,
            register: !(opts.fault == Some(Fault::Unready) && qid + 1 == nq),
            register_qid: if opts.fault == Some(Fault::Register) && qid == 0 {
                u16::MAX
            } else {
                qid as u16
            },
        };
        let (sh, tx) = (sh.clone(), tx.clone());
        let t = std::thread::Builder::new()
            .name(format!("sp-ring-{qid}"))
            .spawn(move || queue_thread(&sh, cfg, tx))
            .map_err(io("spawn a ring queue thread"))?;
        st.threads.push(t);
    }
    drop(tx);
    // Exactly one report per queue. A thread that ends without one drops
    // its sender (so the channel disconnects); the timeout is a backstop.
    let deadline = Instant::now() + SETUP_TIMEOUT;
    for _ in 0..nq {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((_, Ok(()))) => {}
            Ok((qid, Err(e))) => return Err(io(&format!("register ring queue {qid}"))(e)),
            Err(RecvTimeoutError::Timeout) => {
                return Err(io("register ring queues")(io::Error::other(format!(
                    "not every queue reported within {SETUP_TIMEOUT:?}"
                ))));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io("register ring queues")(io::Error::other(
                    "a queue thread ended without reporting",
                )));
            }
        }
    }
    let dsh = sh.clone();
    let t = std::thread::Builder::new()
        .name("sp-ring-dev".into())
        .spawn(move || device_thread(&dsh))
        .map_err(io("spawn the device reader"))?;
    st.threads.push(t);
    probe(&mut st).map_err(io("ring readiness probe"))?;
    sh.enter_serving().map_err(io("ring setup"))?;
    sh.fs.set_uring();
    st.done = true;
    tracing::info!(target: "stillpass", queues = nq, depth = opts.uring_depth, pinned = opts.uring_pin, nice = opts.uring_nice, passthrough, "FUSE over io_uring");
    Ok(Session {
        shared: sh,
        threads: std::mem::take(&mut st.threads),
    })
}

/// One statfs of the mount from this process, which must come back over
/// the ring. The kernel holds every request until each queue has an entry,
/// so this proves the ring is ready; it also proves the kernel did not
/// switch the connection back to the device (it does when a registration
/// fails). statfs is not an input: nothing is recorded.
fn probe(st: &mut Starting) -> io::Result<()> {
    let stats = st.shared.fs.stats();
    let before = (
        stats.ring_requests.load(Ordering::Relaxed),
        stats.device_requests.load(Ordering::Relaxed),
    );
    let (tx, rx) = mpsc::channel();
    let mp = st.shared.mountpoint.clone();
    let t = std::thread::Builder::new()
        .name("sp-ring-probe".into())
        .spawn(move || {
            let _ = tx.send(statvfs(&mp));
        })?;
    // Joined with the rest; on a timeout the teardown's abort ends its wait.
    st.threads.push(t);
    match rx.recv_timeout(PROBE_TIMEOUT) {
        Ok(r) => r?,
        Err(_) => {
            return Err(io::Error::other(format!(
                "a statfs did not come back within {PROBE_TIMEOUT:?}: the ring never became ready"
            )));
        }
    }
    let ring = stats.ring_requests.load(Ordering::Relaxed) - before.0;
    let device = stats.device_requests.load(Ordering::Relaxed) - before.1;
    if ring != 1 || device != 0 {
        return Err(io::Error::other(format!(
            "the probe was not answered over the ring ({ring} ring, {device} device requests)"
        )));
    }
    Ok(())
}

fn statvfs(p: &Path) -> io::Result<()> {
    let c = CString::new(p.as_os_str().as_bytes())?;
    let mut st = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: c is a valid NUL-terminated path and st a writable statvfs;
    // the result is only checked for success, never read.
    #[allow(unsafe_code)] // statvfs(3) has no safe std wrapper
    let rc = unsafe { libc::statvfs(c.as_ptr(), st.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
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

fn umount2(mountpoint: &Path, flags: i32) -> io::Result<()> {
    let target = CString::new(mountpoint.as_os_str().as_bytes())?;
    // SAFETY: target is a valid NUL-terminated path.
    #[allow(unsafe_code)] // umount2(2) has no safe std wrapper
    let rc = unsafe { libc::umount2(target.as_ptr(), flags) };
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
fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_ne_bytes());
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

/// The INIT handshake. Returns whether the kernel granted passthrough.
/// Declines (and fails) unless the kernel offers the ring at a minor this
/// file encodes for.
fn init(dev: &File, want_passthrough: bool) -> io::Result<bool> {
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
    // Declining the connection leaves the mount unusable; the caller
    // unmounts it and falls back.
    let decline = |why: String| {
        let _ = write_reply(dev, unique, -libc::EPROTO, &[]);
        Err(io::Error::other(why))
    };
    if major != 7 {
        return decline(format!("the kernel speaks FUSE {major}.{minor}"));
    }
    if minor > TESTED_MINOR {
        return decline(format!(
            "the kernel speaks FUSE 7.{minor}; the ring transport is tested against 7.{TESTED_MINOR}"
        ));
    }
    if minor < MIN_MINOR || flags2 & FUSE_OVER_IO_URING_2 == 0 {
        return decline("the kernel does not offer FUSE over io_uring".into());
    }
    let passthrough = want_passthrough && flags2 & FUSE_PASSTHROUGH_2 != 0;
    let want2 = FUSE_OVER_IO_URING_2 | if passthrough { FUSE_PASSTHROUGH_2 } else { 0 };
    let out = init_out(
        minor.min(TESTED_MINOR),
        max_readahead,
        WANT & flags,
        want2 & flags2,
        passthrough,
    );
    write_reply(dev, unique, 0, &out)?;
    tracing::info!(target: "stillpass", minor, passthrough, "FUSE over io_uring init");
    Ok(passthrough)
}

/// struct fuse_init_out (64 bytes).
fn init_out(minor: u32, max_readahead: u32, flags: u32, flags2: u32, passthrough: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(INIT_OUT);
    put32(&mut out, 7);
    put32(&mut out, minor);
    put32(&mut out, max_readahead);
    put32(&mut out, flags);
    put16(&mut out, 16); // max_background
    put16(&mut out, 12); // congestion_threshold
    put32(&mut out, MAX_WRITE);
    put32(&mut out, 1); // time_gran
    put16(&mut out, MAX_PAGES);
    put16(&mut out, 0); // map_alignment
    put32(&mut out, flags2);
    // max_stack_depth 1: blobs on a plain filesystem, overlayfs may stack on
    // this mount (the kernel's maximum is 2).
    put32(&mut out, u32::from(passthrough));
    // request_timeout 0 (none) and unused[11].
    out.resize(INIT_OUT, 0);
    out
}

/// One request's answer: errno (0 for success) and the out-args.
type Reply = Option<(i32, Vec<u8>)>;

/// Decodes one request and asks the model. `hdr` is the in-header, `op` the
/// op header (the first in-arg), `payload` the in-args after it; over the
/// device `op` and `payload` are both the bytes after the in-header. `None`:
/// the op has no reply.
fn handle(fs: &CasFs, dev: &File, hdr: &[u8], op: &[u8], payload: &[u8]) -> Reply {
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
            Ok(attr) => ok(attr_out(&attr)),
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
                Ok(Opened { fh, backing }) => ok(match backing.as_deref().and_then(Backing::raw) {
                    Some(id) => open_out(fh, FOPEN_PASSTHROUGH, id),
                    None => open_out(fh, FOPEN_KEEP_CACHE, 0),
                }),
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
        // The same reply as the fuser transport's: handle 0, no flags.
        FUSE_OPENDIR => match fs.opendir(nodeid) {
            Ok(()) => ok(open_out(0, 0, 0)),
            Err(e) => err(e),
        },
        FUSE_READDIR => {
            let (offset, size) = (u64_at(op, 8), u32_at(op, 16) as usize);
            match fs.readdir(&req, nodeid, offset) {
                Ok(entries) => ok(dirents(&entries, size.min(PAYLOAD))),
                Err(e) => err(e),
            }
        }
        FUSE_RELEASEDIR | FUSE_DESTROY => ok(Vec::new()),
        FUSE_STATFS => {
            fs.statfs();
            ok(statfs_out(fs.file_count()))
        }
        // Overlayfs probes lower xattrs; it accepts ENOTSUP cleanly.
        FUSE_GETXATTR | FUSE_LISTXATTR => err(libc::ENOTSUP),
        FUSE_FLUSH | FUSE_ACCESS => err(libc::ENOSYS),
        FUSE_FORGET | FUSE_BATCH_FORGET | FUSE_INTERRUPT => None,
        _ => err(libc::ENOSYS),
    }
}

/// struct fuse_attr (88 bytes).
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
    put32(out, 0); // atimensec
    put32(out, 0); // mtimensec
    put32(out, 0); // ctimensec
    put32(out, mode);
    put32(out, a.nlink);
    put32(out, a.uid);
    put32(out, a.gid);
    put32(out, a.rdev);
    put32(out, a.blksize);
    put32(out, 0); // flags
}

/// struct fuse_entry_out (128 bytes).
fn entry_out(ino: u64, a: &FileAttr) -> Vec<u8> {
    let mut out = Vec::with_capacity(ENTRY_OUT);
    put64(&mut out, ino);
    put64(&mut out, 0); // generation
    put64(&mut out, TTL.as_secs()); // entry_valid
    put64(&mut out, TTL.as_secs()); // attr_valid
    put32(&mut out, 0); // entry_valid_nsec
    put32(&mut out, 0); // attr_valid_nsec
    put_attr(&mut out, a);
    out
}

/// struct fuse_attr_out (104 bytes).
fn attr_out(a: &FileAttr) -> Vec<u8> {
    let mut out = Vec::with_capacity(ATTR_OUT);
    put64(&mut out, TTL.as_secs()); // attr_valid
    put32(&mut out, 0); // attr_valid_nsec
    put32(&mut out, 0); // dummy
    put_attr(&mut out, a);
    out
}

/// struct fuse_open_out (16 bytes: fh, open_flags, backing_id).
fn open_out(fh: u64, open_flags: u32, backing_id: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(OPEN_OUT);
    put64(&mut out, fh);
    put32(&mut out, open_flags);
    put32(&mut out, backing_id);
    out
}

/// struct fuse_statfs_out (struct fuse_kstatfs, 80 bytes): what the fuser
/// transport replies, `statfs(0, 0, 0, files, 0, 4096, 255, 4096)`.
fn statfs_out(files: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(STATFS_OUT);
    for v in [0, 0, 0, files, 0] {
        put64(&mut out, v); // blocks, bfree, bavail, files, ffree
    }
    put32(&mut out, 4096); // bsize
    put32(&mut out, 255); // namelen
    put32(&mut out, 4096); // frsize
    out.resize(STATFS_OUT, 0); // padding, spare[6]
    out
}

/// struct fuse_dirent records, each 8-byte aligned, as many as fit `size`.
fn dirents(entries: &[DirEnt], size: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for d in entries {
        let reclen = (DIRENT_NAME + d.name.len()).div_ceil(8) * 8;
        if out.len() + reclen > size {
            break;
        }
        put64(&mut out, d.ino);
        put64(&mut out, d.off);
        put32(&mut out, d.name.len() as u32);
        put32(&mut out, dirent_type(d.kind));
        out.extend_from_slice(d.name.as_bytes());
        out.resize(out.len().div_ceil(8) * 8, 0);
    }
    out
}

fn dirent_type(kind: FileType) -> u32 {
    (match kind {
        FileType::Directory => libc::DT_DIR,
        FileType::Symlink => libc::DT_LNK,
        _ => libc::DT_REG,
    }) as u32
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".into())
}

/// The device reader: what the kernel does not put on the ring (forgets
/// and interrupts, which have no reply). Ends when the connection does.
fn device_thread(sh: &Shared) {
    let what = match catch_unwind(AssertUnwindSafe(|| device_loop(sh))) {
        Ok(Ok(())) => return,
        Ok(Err(e)) => format!("the device reader failed: {e}"),
        Err(p) => format!("the device reader panicked: {}", panic_text(&*p)),
    };
    sh.fail(what);
}

fn device_loop(sh: &Shared) -> io::Result<()> {
    let stats = sh.fs.stats();
    let mut buf = vec![0u8; PAYLOAD + 4096];
    loop {
        let n = match read_dev(&sh.dev, &mut buf) {
            Ok(n) => n,
            Err(_) if sh.stopping() => return Ok(()),
            Err(e) => return Err(e),
        };
        if n < IN_HEADER {
            return Err(io::Error::other(format!("a {n}-byte request")));
        }
        let (hdr, rest) = buf[..n].split_at(IN_HEADER);
        let opcode = u32_at(hdr, 4);
        let Some((error, data)) = handle(&sh.fs, &sh.dev, hdr, rest, rest) else {
            continue;
        };
        stats.device_requests.fetch_add(1, Ordering::Relaxed);
        if let Err(e) = write_reply(&sh.dev, u64_at(hdr, 8), error, &data) {
            if sh.stopping() {
                return Ok(());
            }
            // The requester got an error, not the model's answer.
            sh.fs.io_error(format!(
                "FUSE over io_uring: the kernel refused the reply to opcode {opcode}: {e}"
            ));
        }
    }
}

/// A heap buffer the kernel reads and writes behind Rust's back. Held as a
/// raw pointer from allocation to free: no Box or reference to it is moved
/// or kept while the kernel owns it, so nothing asserts uniqueness over
/// memory the kernel may be writing.
struct RawBuf {
    ptr: *mut u8,
    len: usize,
}

impl RawBuf {
    fn zeroed(len: usize) -> RawBuf {
        let b: Box<[u8]> = vec![0u8; len].into_boxed_slice();
        RawBuf {
            ptr: Box::into_raw(b).cast::<u8>(),
            len,
        }
    }
}

impl Drop for RawBuf {
    fn drop(&mut self) {
        // SAFETY: ptr and len are exactly what Box::into_raw of a boxed
        // slice of len bytes returned in `zeroed`; freed once, here.
        #[allow(unsafe_code)] // the matching free of Box::into_raw
        unsafe {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                self.ptr, self.len,
            )))
        }
    }
}

/// One ring entry: the kernel writes a request into `header` (in-header,
/// op header, ring entry in/out) and `payload`, and reads the reply from
/// them. `iov` points at both; REGISTER hands the kernel its address.
struct Ent {
    header: RawBuf,
    payload: RawBuf,
    iov: *mut [libc::iovec; 2],
}

impl Ent {
    fn new() -> Ent {
        let header = RawBuf::zeroed(RING_HEADER);
        let payload = RawBuf::zeroed(PAYLOAD);
        // The pointers are taken from the final allocations, which never
        // move (only the RawBuf handles do).
        let iov = Box::into_raw(Box::new([
            libc::iovec {
                iov_base: header.ptr.cast(),
                iov_len: header.len,
            },
            libc::iovec {
                iov_base: payload.ptr.cast(),
                iov_len: payload.len,
            },
        ]));
        Ent {
            header,
            payload,
            iov,
        }
    }

    /// The header and payload as slices.
    ///
    /// # Safety
    /// Only while the kernel has handed this entry to userspace (between its
    /// completion and the next commit of it), and the slices must be gone
    /// before that commit is submitted.
    #[allow(unsafe_code)] // see the Safety section
    unsafe fn buffers(&mut self) -> (&mut [u8], &mut [u8]) {
        // SAFETY: both are live allocations of exactly len bytes (RawBuf),
        // distinct from each other; the caller guarantees the kernel is not
        // touching them, and `&mut self` that no other slice of them exists.
        unsafe {
            (
                std::slice::from_raw_parts_mut(self.header.ptr, self.header.len),
                std::slice::from_raw_parts_mut(self.payload.ptr, self.payload.len),
            )
        }
    }
}

impl Drop for Ent {
    fn drop(&mut self) {
        // SAFETY: iov came from Box::into_raw in `new`; freed once, here.
        #[allow(unsafe_code)] // the matching free of Box::into_raw
        unsafe {
            drop(Box::from_raw(self.iov))
        }
    }
}

/// An IORING_OP_URING_CMD SQE for the fuse device: `addr` the entry's two
/// iovecs, `len` 2, the 80-byte command a
/// `struct fuse_uring_cmd_req { u64 flags; u64 commit_id; u16 qid; }`.
fn fuse_sqe(
    fd: i32,
    cmd_op: u32,
    qid: u16,
    commit_id: u64,
    ent: &Ent,
    ud: u64,
) -> squeue::Entry128 {
    let mut cmd = [0u8; 80];
    cmd[8..16].copy_from_slice(&commit_id.to_ne_bytes());
    cmd[16..18].copy_from_slice(&qid.to_ne_bytes());
    let sqe = opcode::UringCmd80::new(types::Fd(fd), cmd_op)
        .cmd(cmd)
        .addr(Some(ent.iov as u64))
        .build()
        .user_data(ud);
    with_len(sqe, FUSE_URING_IOV_SEGS)
}

/// Sets the SQE's `len` (byte 24 of `struct io_uring_sqe`), which the
/// crate's UringCmd80 builder has no setter for.
fn with_len(sqe: squeue::Entry128, len: u32) -> squeue::Entry128 {
    let mut raw = sqe_bytes(sqe);
    raw[24..28].copy_from_slice(&len.to_ne_bytes());
    // SAFETY: see `sqe_bytes`; the reverse direction, same reasoning.
    #[allow(unsafe_code)] // the crate has no raw constructor
    unsafe {
        std::mem::transmute::<[u8; 128], squeue::Entry128>(raw)
    }
}

/// The SQE's bytes.
fn sqe_bytes(sqe: squeue::Entry128) -> [u8; 128] {
    // SAFETY: transmute checks at compile time that both are 128 bytes.
    // Entry128 is #[repr(C)] over io_uring_sqe (64 bytes, #[repr(C)], every
    // field an integer or a union of integers, no padding) and [u8; 64], so
    // both types are valid for every bit pattern. Which byte means what is
    // the kernel ABI (include/uapi/linux/io_uring.h); `check_sqe_layout`
    // confirms, before any queue starts, that the crate keeps every field
    // this file sets through its builder (opcode, fd, cmd_op, addr,
    // user_data, the command) at the kernel's offset, i.e. that byte 24 is
    // the kernel's `len` and no other field. It does not check fields this
    // file never sets.
    #[allow(unsafe_code)] // reading the crate's struct as the kernel's bytes
    unsafe {
        std::mem::transmute::<squeue::Entry128, [u8; 128]>(sqe)
    }
}

/// Builds an SQE with distinct values in every field this file sets and
/// checks each lands at its kernel ABI offset (opcode 0, fd 4, cmd_op 8,
/// addr 16, len 24, user_data 32, the 80-byte command 48..128).
fn check_sqe_layout() -> io::Result<()> {
    let mut cmd = [0u8; 80];
    for (i, b) in cmd.iter_mut().enumerate() {
        *b = i as u8 + 1;
    }
    let sqe = opcode::UringCmd80::new(types::Fd(0x0a0b_0c0d), 0x1234_5678)
        .cmd(cmd)
        .addr(Some(0x1122_3344_5566_7788))
        .build()
        .user_data(0x99aa_bbcc_ddee_ff00);
    let raw = sqe_bytes(with_len(sqe, 0x0102_0304));
    let ok = raw[0] == IORING_OP_URING_CMD
        && u32_at(&raw, 4) == 0x0a0b_0c0d
        && u32_at(&raw, 8) == 0x1234_5678
        && u64_at(&raw, 16) == 0x1122_3344_5566_7788
        && u32_at(&raw, 24) == 0x0102_0304
        && u64_at(&raw, 32) == 0x99aa_bbcc_ddee_ff00
        && raw[48..128] == cmd;
    if !ok {
        return Err(io::Error::other(
            "the io-uring crate's SQE layout differs from the kernel's",
        ));
    }
    Ok(())
}

/// Pins the calling thread to `cpu`.
fn pin_to_cpu(cpu: usize) -> io::Result<()> {
    if cpu >= libc::CPU_SETSIZE as usize {
        return Err(io::Error::other(format!(
            "CPU {cpu} is beyond CPU_SETSIZE ({})",
            libc::CPU_SETSIZE
        )));
    }
    // SAFETY: a zeroed cpu_set_t is a valid empty set; CPU_SET writes bit
    // `cpu` of it, in bounds (checked above); sched_setaffinity reads only
    // that set.
    #[allow(unsafe_code)] // no safe std affinity API
    let rc = unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set)
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The possible CPUs. The kernel makes one queue per possible CPU
/// (num_possible_cpus), so this many queues must register.
fn possible_cpus() -> io::Result<Vec<usize>> {
    parse_cpu_list(&std::fs::read_to_string(
        "/sys/devices/system/cpu/possible",
    )?)
}

/// A kernel CPU list ("0-3,8-11", "0", "0-17"), in order, without repeats.
fn parse_cpu_list(s: &str) -> io::Result<Vec<usize>> {
    let bad = || io::Error::other(format!("bad CPU list {s:?}"));
    let mut cpus = Vec::new();
    for part in s.trim().split(',') {
        let (lo, hi) = match part.split_once('-') {
            Some((a, b)) => (a, b),
            None => (part, part),
        };
        let (lo, hi): (usize, usize) = (
            lo.trim().parse().map_err(|_| bad())?,
            hi.trim().parse().map_err(|_| bad())?,
        );
        if hi < lo || cpus.last().is_some_and(|&l| lo <= l) {
            return Err(bad());
        }
        cpus.extend(lo..=hi);
    }
    Ok(cpus)
}

struct QueueCfg {
    qid: u16,
    /// The CPU to pin to, if any.
    cpu: Option<usize>,
    depth: usize,
    nice: i32,
    /// Whether to register at all (the `Unready` test hook skips one).
    register: bool,
    /// The queue id put in REGISTER (the `Register` test hook breaks it).
    register_qid: u16,
}

/// One queue's thread: registers, reports, then serves. A failure before
/// the report goes to `mount` (fallback); after it, to `Shared::fail`.
fn queue_thread(sh: &Shared, cfg: QueueCfg, tx: mpsc::Sender<(u16, io::Result<()>)>) {
    let mut report = Some(tx);
    let what = match catch_unwind(AssertUnwindSafe(|| queue(sh, &cfg, &mut report))) {
        Ok(Ok(())) => return,
        Ok(Err(e)) => format!("ring queue {} failed: {e}", cfg.qid),
        Err(p) => format!("ring queue {} panicked: {}", cfg.qid, panic_text(&*p)),
    };
    match report.take() {
        Some(tx) => {
            let _ = tx.send((cfg.qid, Err(io::Error::other(what))));
        }
        None => sh.fail(what),
    }
}

type Report = Option<mpsc::Sender<(u16, io::Result<()>)>>;

fn queue(sh: &Shared, cfg: &QueueCfg, report: &mut Report) -> io::Result<()> {
    if let Some(cpu) = cfg.cpu
        && let Err(e) = pin_to_cpu(cpu)
    {
        tracing::warn!(qid = cfg.qid, cpu, error = %e, "ring queue thread not pinned");
    }
    if cfg.nice != 0 {
        // SAFETY: gettid and setpriority have no memory preconditions; on
        // Linux PRIO_PROCESS with a thread id sets that thread's nice value.
        #[allow(unsafe_code)] // no safe std priority API
        let rc = unsafe {
            libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, cfg.nice)
        };
        if rc != 0 {
            tracing::warn!(qid = cfg.qid, error = %io::Error::last_os_error(), "ring queue thread nice unchanged");
        }
    }
    let fd = sh.dev.as_raw_fd();
    // The entries are declared before the ring, so they are dropped after
    // it, on every exit including a panic. FUSE copies a request into an
    // entry only from the ring's task work (fuse_uring_send_in_task) and
    // copies a reply out only inside this thread's submit; dropping the ring
    // kills its context (io_uring_release, before close returns), after
    // which pending task work runs cancelled (io_should_terminate_tw) and
    // copies nothing. So nothing writes to the entries once the ring is gone.
    let mut ents: Vec<Ent> = (0..cfg.depth).map(|_| Ent::new()).collect();
    let mut ring: IoUring<squeue::Entry128, cqueue::Entry32> =
        IoUring::builder().build(cfg.depth as u32)?;
    let mut pending: Vec<(u64, i32)> = Vec::with_capacity(cfg.depth);
    if cfg.register {
        for (i, ent) in ents.iter().enumerate() {
            let sqe = fuse_sqe(
                fd,
                FUSE_IO_URING_CMD_REGISTER,
                cfg.register_qid,
                0,
                ent,
                i as u64,
            );
            // SAFETY: the SQE points at the entry's iovecs and, through
            // them, its buffers; all live in `ents`, which outlives the ring.
            #[allow(unsafe_code)] // io_uring's push is unsafe by contract
            unsafe { ring.submission().push(&sqe) }
                .map_err(|_| io::Error::other("submission queue full"))?;
        }
        let n = ring.submit()?;
        if n != cfg.depth {
            return Err(io::Error::other(format!(
                "the kernel took {n} of {} registrations",
                cfg.depth
            )));
        }
        // A registration the kernel refuses completes during the submit
        // with a negative result; an accepted one stays pending until a
        // request arrives (a non-negative completion, served below).
        pending.extend(ring.completion().map(|c| (c.user_data(), c.result())));
        if let Some(&(ud, res)) = pending.iter().find(|(_, r)| *r < 0) {
            return Err(io::Error::other(format!(
                "REGISTER of entry {ud}: {}",
                io::Error::from_raw_os_error(-res)
            )));
        }
    }
    if let Some(tx) = report.take() {
        let _ = tx.send((cfg.qid, Ok(())));
    }
    if !cfg.register {
        // The `Unready` test hook: a queue with nothing in flight would wait
        // on its ring forever, so it waits for the teardown instead.
        while !sh.stopping() {
            std::thread::sleep(Duration::from_millis(10));
        }
        return Ok(());
    }
    serve_queue(sh, cfg.qid, &mut ring, &mut ents, pending)
}

/// Commit-and-fetch until the connection ends. Each completion hands one
/// entry, holding one request, to userspace; the reply is written into the
/// same entry and committed, which also fetches its next request.
fn serve_queue(
    sh: &Shared,
    qid: u16,
    ring: &mut IoUring<squeue::Entry128, cqueue::Entry32>,
    ents: &mut [Ent],
    mut pending: Vec<(u64, i32)>,
) -> io::Result<()> {
    let fd = sh.dev.as_raw_fd();
    let stats = sh.fs.stats();
    let depth = ents.len();
    loop {
        while pending.is_empty() {
            match ring.submit_and_wait(1) {
                Ok(_) => {}
                // A signal interrupts only the wait; the commits were
                // submitted or are still queued. Wait again.
                Err(e) if e.raw_os_error() == Some(libc::EINTR) => continue,
                Err(_) if sh.stopping() => return Ok(()),
                Err(e) => return Err(e),
            }
            pending.extend(ring.completion().map(|c| (c.user_data(), c.result())));
        }
        stats.ring_batches.fetch_add(1, Ordering::Relaxed);
        if pending.len() == depth {
            stats.ring_full_batches.fetch_add(1, Ordering::Relaxed);
        }
        for (ud, mut res) in pending.drain(..) {
            if res >= 0 {
                match sh.take_armed() {
                    1 => panic!("test hook: ring queue {qid} dies holding a request"),
                    2 => res = -libc::EIO,
                    _ => {}
                }
            }
            if res < 0 {
                if sh.stopping() {
                    return Ok(());
                }
                return Err(io::Error::other(format!(
                    "completion of entry {ud}: {}",
                    io::Error::from_raw_os_error(-res)
                )));
            }
            let t = Instant::now();
            let ent = ents
                .get_mut(ud as usize)
                .ok_or_else(|| io::Error::other(format!("completion for unknown entry {ud}")))?;
            let commit_id = {
                // SAFETY: this completion handed the entry to userspace: the
                // kernel neither reads nor writes its buffers until the
                // commit below is submitted, and these slices end first.
                #[allow(unsafe_code)] // see Ent::buffers
                let (header, payload) = unsafe { ent.buffers() };
                let unique = u64_at(header, 8);
                let commit_id = u64_at(header, ENT_IN_OUT + 8);
                let payload_sz = (u32_at(header, ENT_IN_OUT + 16) as usize).min(payload.len());
                let reply = handle(
                    &sh.fs,
                    &sh.dev,
                    &header[..IN_HEADER],
                    &header[OP_IN..ENT_IN_OUT],
                    &payload[..payload_sz],
                );
                let Some((error, data)) = reply else {
                    return Err(io::Error::other(format!(
                        "the kernel put opcode {} (no reply) on the ring",
                        u32_at(header, 4)
                    )));
                };
                if data.len() > payload.len() {
                    return Err(io::Error::other(format!(
                        "a {}-byte reply does not fit the ring payload",
                        data.len()
                    )));
                }
                // Out header into in_out, out-args into the payload.
                header[0..4].copy_from_slice(&((OUT_HEADER + data.len()) as u32).to_ne_bytes());
                header[4..8].copy_from_slice(&error.to_ne_bytes());
                header[8..16].copy_from_slice(&unique.to_ne_bytes());
                payload[..data.len()].copy_from_slice(&data);
                header[ENT_IN_OUT + 16..ENT_IN_OUT + 20]
                    .copy_from_slice(&(data.len() as u32).to_ne_bytes());
                commit_id
            };
            let sqe = fuse_sqe(
                fd,
                FUSE_IO_URING_CMD_COMMIT_AND_FETCH,
                qid,
                commit_id,
                ent,
                ud,
            );
            // SAFETY: as for REGISTER; the entry outlives the ring.
            #[allow(unsafe_code)] // io_uring's push is unsafe by contract
            unsafe { ring.submission().push(&sqe) }
                .map_err(|_| io::Error::other("submission queue full"))?;
            stats.ring_requests.fetch_add(1, Ordering::Relaxed);
            stats
                .ring_serve_ns
                .fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
            sh.per_queue[qid as usize].fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    //! The encoders against the kernel's structs at FUSE 7.45, mirrored
    //! here from include/uapi/linux/fuse.h (Linux 7.0) as #[repr(C)] so the
    //! compiler computes sizes and offsets the way the kernel's C does.

    use std::mem::{offset_of, size_of};
    use std::time::UNIX_EPOCH;

    use fuser::INodeNo;

    use super::*;

    #[repr(C)]
    struct FuseAttr {
        ino: u64,
        size: u64,
        blocks: u64,
        atime: u64,
        mtime: u64,
        ctime: u64,
        atimensec: u32,
        mtimensec: u32,
        ctimensec: u32,
        mode: u32,
        nlink: u32,
        uid: u32,
        gid: u32,
        rdev: u32,
        blksize: u32,
        flags: u32,
    }
    #[repr(C)]
    struct FuseEntryOut {
        nodeid: u64,
        generation: u64,
        entry_valid: u64,
        attr_valid: u64,
        entry_valid_nsec: u32,
        attr_valid_nsec: u32,
        attr: FuseAttr,
    }
    #[repr(C)]
    struct FuseAttrOut {
        attr_valid: u64,
        attr_valid_nsec: u32,
        dummy: u32,
        attr: FuseAttr,
    }
    #[repr(C)]
    struct FuseOpenOut {
        fh: u64,
        open_flags: u32,
        backing_id: i32,
    }
    #[repr(C)]
    struct FuseKstatfs {
        blocks: u64,
        bfree: u64,
        bavail: u64,
        files: u64,
        ffree: u64,
        bsize: u32,
        namelen: u32,
        frsize: u32,
        padding: u32,
        spare: [u32; 6],
    }
    #[repr(C)]
    struct FuseDirent {
        ino: u64,
        off: u64,
        namelen: u32,
        typ: u32,
        name: [u8; 0],
    }
    #[repr(C)]
    struct FuseInitOut {
        major: u32,
        minor: u32,
        max_readahead: u32,
        flags: u32,
        max_background: u16,
        congestion_threshold: u16,
        max_write: u32,
        time_gran: u32,
        max_pages: u16,
        map_alignment: u16,
        flags2: u32,
        max_stack_depth: u32,
        request_timeout: u16,
        unused: [u16; 11],
    }
    #[repr(C)]
    struct FuseInHeader {
        len: u32,
        opcode: u32,
        unique: u64,
        nodeid: u64,
        uid: u32,
        gid: u32,
        pid: u32,
        total_extlen: u16,
        padding: u16,
    }
    #[repr(C)]
    struct FuseOutHeader {
        len: u32,
        error: i32,
        unique: u64,
    }
    #[repr(C)]
    struct FuseReadIn {
        fh: u64,
        offset: u64,
        size: u32,
        read_flags: u32,
        lock_owner: u64,
        flags: u32,
        padding: u32,
    }
    #[repr(C)]
    struct FuseUringEntInOut {
        flags: u64,
        commit_id: u64,
        payload_sz: u32,
        padding: u32,
        reserved: u64,
    }
    #[repr(C)]
    struct FuseUringReqHeader {
        in_out: [u8; 128],
        op_in: [u8; 128],
        ring_ent_in_out: FuseUringEntInOut,
    }
    #[repr(C)]
    struct FuseUringCmdReq {
        flags: u64,
        commit_id: u64,
        qid: u16,
        padding: [u8; 6],
    }

    fn attr(kind: FileType, perm: u16) -> FileAttr {
        FileAttr {
            ino: INodeNo(0x0102_0304_0506_0708),
            size: 0x1111_2222_3333,
            blocks: 0x44_5555,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind,
            perm,
            nlink: 7,
            uid: 1001,
            gid: 1002,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    fn u16_at(b: &[u8], off: usize) -> u16 {
        u16::from_ne_bytes(b[off..off + 2].try_into().unwrap())
    }

    /// The attr fields at `base` decode to `attr(kind, perm)`.
    fn check_attr(b: &[u8], base: usize, mode: u32) {
        let f = |o: usize| base + o;
        assert_eq!(
            u64_at(b, f(offset_of!(FuseAttr, ino))),
            0x0102_0304_0506_0708
        );
        assert_eq!(u64_at(b, f(offset_of!(FuseAttr, size))), 0x1111_2222_3333);
        assert_eq!(u64_at(b, f(offset_of!(FuseAttr, blocks))), 0x44_5555);
        for o in [
            offset_of!(FuseAttr, atime),
            offset_of!(FuseAttr, mtime),
            offset_of!(FuseAttr, ctime),
        ] {
            assert_eq!(u64_at(b, f(o)), 0);
        }
        assert_eq!(u32_at(b, f(offset_of!(FuseAttr, mode))), mode);
        assert_eq!(u32_at(b, f(offset_of!(FuseAttr, nlink))), 7);
        assert_eq!(u32_at(b, f(offset_of!(FuseAttr, uid))), 1001);
        assert_eq!(u32_at(b, f(offset_of!(FuseAttr, gid))), 1002);
        assert_eq!(u32_at(b, f(offset_of!(FuseAttr, rdev))), 0);
        assert_eq!(u32_at(b, f(offset_of!(FuseAttr, blksize))), 4096);
        assert_eq!(u32_at(b, f(offset_of!(FuseAttr, flags))), 0);
    }

    #[test]
    fn mirrored_structs_have_the_kernel_sizes() {
        assert_eq!(size_of::<FuseAttr>(), ATTR);
        assert_eq!(size_of::<FuseEntryOut>(), ENTRY_OUT);
        assert_eq!(size_of::<FuseAttrOut>(), ATTR_OUT);
        assert_eq!(size_of::<FuseOpenOut>(), OPEN_OUT);
        assert_eq!(size_of::<FuseKstatfs>(), STATFS_OUT);
        assert_eq!(offset_of!(FuseDirent, name), DIRENT_NAME);
        assert_eq!(size_of::<FuseInitOut>(), INIT_OUT);
        assert_eq!(size_of::<FuseInHeader>(), IN_HEADER);
        assert_eq!(size_of::<FuseOutHeader>(), OUT_HEADER);
        assert_eq!(size_of::<FuseUringReqHeader>(), RING_HEADER);
        assert_eq!(offset_of!(FuseUringReqHeader, op_in), OP_IN);
        assert_eq!(offset_of!(FuseUringReqHeader, ring_ent_in_out), ENT_IN_OUT);
        assert_eq!(offset_of!(FuseUringEntInOut, commit_id), 8);
        assert_eq!(offset_of!(FuseUringEntInOut, payload_sz), 16);
        assert!(size_of::<FuseUringCmdReq>() <= 80);
        // The fields the decoders read.
        assert_eq!(offset_of!(FuseInHeader, opcode), 4);
        assert_eq!(offset_of!(FuseInHeader, unique), 8);
        assert_eq!(offset_of!(FuseInHeader, nodeid), 16);
        assert_eq!(offset_of!(FuseInHeader, pid), 32);
        assert_eq!(offset_of!(FuseReadIn, offset), 8);
        assert_eq!(offset_of!(FuseReadIn, size), 16);
        assert_eq!(offset_of!(FuseUringCmdReq, commit_id), 8);
        assert_eq!(offset_of!(FuseUringCmdReq, qid), 16);
    }

    #[test]
    fn put_attr_matches_fuse_attr() {
        let mut b = Vec::new();
        put_attr(&mut b, &attr(FileType::RegularFile, 0o644));
        assert_eq!(b.len(), size_of::<FuseAttr>());
        check_attr(&b, 0, libc::S_IFREG | 0o644);
        let mut b = Vec::new();
        put_attr(&mut b, &attr(FileType::Directory, 0o755));
        check_attr(&b, 0, libc::S_IFDIR | 0o755);
        let mut b = Vec::new();
        put_attr(&mut b, &attr(FileType::Symlink, 0o777));
        check_attr(&b, 0, libc::S_IFLNK | 0o777);
    }

    #[test]
    fn entry_out_matches_fuse_entry_out() {
        let b = entry_out(42, &attr(FileType::RegularFile, 0o600));
        assert_eq!(b.len(), size_of::<FuseEntryOut>());
        assert_eq!(u64_at(&b, offset_of!(FuseEntryOut, nodeid)), 42);
        assert_eq!(u64_at(&b, offset_of!(FuseEntryOut, generation)), 0);
        assert_eq!(
            u64_at(&b, offset_of!(FuseEntryOut, entry_valid)),
            TTL.as_secs()
        );
        assert_eq!(
            u64_at(&b, offset_of!(FuseEntryOut, attr_valid)),
            TTL.as_secs()
        );
        assert_eq!(u32_at(&b, offset_of!(FuseEntryOut, entry_valid_nsec)), 0);
        assert_eq!(u32_at(&b, offset_of!(FuseEntryOut, attr_valid_nsec)), 0);
        check_attr(&b, offset_of!(FuseEntryOut, attr), libc::S_IFREG | 0o600);
    }

    #[test]
    fn attr_out_matches_fuse_attr_out() {
        let b = attr_out(&attr(FileType::Directory, 0o755));
        assert_eq!(b.len(), size_of::<FuseAttrOut>());
        assert_eq!(
            u64_at(&b, offset_of!(FuseAttrOut, attr_valid)),
            TTL.as_secs()
        );
        assert_eq!(u32_at(&b, offset_of!(FuseAttrOut, attr_valid_nsec)), 0);
        assert_eq!(u32_at(&b, offset_of!(FuseAttrOut, dummy)), 0);
        check_attr(&b, offset_of!(FuseAttrOut, attr), libc::S_IFDIR | 0o755);
    }

    #[test]
    fn open_out_matches_fuse_open_out() {
        let b = open_out(0xdead_beef_0000_0001, FOPEN_PASSTHROUGH, 9);
        assert_eq!(b.len(), size_of::<FuseOpenOut>());
        assert_eq!(
            u64_at(&b, offset_of!(FuseOpenOut, fh)),
            0xdead_beef_0000_0001
        );
        assert_eq!(
            u32_at(&b, offset_of!(FuseOpenOut, open_flags)),
            FOPEN_PASSTHROUGH
        );
        assert_eq!(u32_at(&b, offset_of!(FuseOpenOut, backing_id)), 9);
        assert_eq!(open_out(0, 0, 0), vec![0u8; 16], "opendir's reply");
    }

    #[test]
    fn statfs_out_matches_fuse_kstatfs() {
        let b = statfs_out(12345);
        assert_eq!(b.len(), size_of::<FuseKstatfs>());
        for o in [
            offset_of!(FuseKstatfs, blocks),
            offset_of!(FuseKstatfs, bfree),
            offset_of!(FuseKstatfs, bavail),
            offset_of!(FuseKstatfs, ffree),
        ] {
            assert_eq!(u64_at(&b, o), 0);
        }
        assert_eq!(u64_at(&b, offset_of!(FuseKstatfs, files)), 12345);
        assert_eq!(u32_at(&b, offset_of!(FuseKstatfs, bsize)), 4096);
        assert_eq!(u32_at(&b, offset_of!(FuseKstatfs, namelen)), 255);
        assert_eq!(u32_at(&b, offset_of!(FuseKstatfs, frsize)), 4096);
        assert!(
            b[offset_of!(FuseKstatfs, padding)..]
                .iter()
                .all(|&x| x == 0)
        );
    }

    #[test]
    fn dirents_are_aligned_records_that_stop_at_the_size() {
        let names = ["a", "bcdefgh", "ijklmnop", "x".repeat(200).leak()];
        let entries: Vec<DirEnt> = names
            .iter()
            .enumerate()
            .map(|(i, n)| DirEnt {
                ino: 100 + i as u64,
                off: i as u64 + 1,
                kind: [
                    FileType::RegularFile,
                    FileType::Directory,
                    FileType::Symlink,
                ][i % 3],
                name: n,
            })
            .collect();
        let b = dirents(&entries, 4096);
        // 24 + namelen rounded up to 8: 32, 32, 32, 224.
        assert_eq!(b.len(), 32 + 32 + 32 + 224);
        let mut at = 0;
        for (i, n) in names.iter().enumerate() {
            assert_eq!(u64_at(&b, at + offset_of!(FuseDirent, ino)), 100 + i as u64);
            assert_eq!(u64_at(&b, at + offset_of!(FuseDirent, off)), i as u64 + 1);
            assert_eq!(
                u32_at(&b, at + offset_of!(FuseDirent, namelen)) as usize,
                n.len()
            );
            let want = [libc::DT_REG, libc::DT_DIR, libc::DT_LNK][i % 3] as u32;
            assert_eq!(u32_at(&b, at + offset_of!(FuseDirent, typ)), want);
            assert_eq!(
                &b[at + DIRENT_NAME..at + DIRENT_NAME + n.len()],
                n.as_bytes()
            );
            let rec = (DIRENT_NAME + n.len()).div_ceil(8) * 8;
            assert!(
                b[at + DIRENT_NAME + n.len()..at + rec]
                    .iter()
                    .all(|&x| x == 0)
            );
            at += rec;
        }
        // A size that ends inside the third record keeps two whole ones,
        // like fuser's ReplyDirectory; the kernel asks again from offset 2.
        assert_eq!(dirents(&entries, 95).len(), 64);
        assert_eq!(dirents(&entries, 96).len(), 96);
        assert!(dirents(&entries, 31).is_empty());
    }

    #[test]
    fn init_out_matches_fuse_init_out() {
        let b = init_out(
            45,
            0x2_0000,
            WANT,
            FUSE_OVER_IO_URING_2 | FUSE_PASSTHROUGH_2,
            true,
        );
        assert_eq!(b.len(), size_of::<FuseInitOut>());
        assert_eq!(u32_at(&b, offset_of!(FuseInitOut, major)), 7);
        assert_eq!(u32_at(&b, offset_of!(FuseInitOut, minor)), 45);
        assert_eq!(u32_at(&b, offset_of!(FuseInitOut, max_readahead)), 0x2_0000);
        assert_eq!(u32_at(&b, offset_of!(FuseInitOut, flags)), WANT);
        assert_eq!(u16_at(&b, offset_of!(FuseInitOut, max_background)), 16);
        assert_eq!(
            u16_at(&b, offset_of!(FuseInitOut, congestion_threshold)),
            12
        );
        assert_eq!(u32_at(&b, offset_of!(FuseInitOut, max_write)), MAX_WRITE);
        assert_eq!(u32_at(&b, offset_of!(FuseInitOut, time_gran)), 1);
        assert_eq!(u16_at(&b, offset_of!(FuseInitOut, max_pages)), MAX_PAGES);
        assert_eq!(u16_at(&b, offset_of!(FuseInitOut, map_alignment)), 0);
        assert_eq!(
            u32_at(&b, offset_of!(FuseInitOut, flags2)),
            FUSE_OVER_IO_URING_2 | FUSE_PASSTHROUGH_2
        );
        assert_eq!(u32_at(&b, offset_of!(FuseInitOut, max_stack_depth)), 1);
        assert_eq!(u16_at(&b, offset_of!(FuseInitOut, request_timeout)), 0);
        assert!(b[offset_of!(FuseInitOut, unused)..].iter().all(|&x| x == 0));
        let b = init_out(45, 0, WANT, FUSE_OVER_IO_URING_2, false);
        assert_eq!(u32_at(&b, offset_of!(FuseInitOut, max_stack_depth)), 0);
        // The payload covers the largest read and readdir reply.
        assert!(PAYLOAD >= MAX_WRITE as usize && PAYLOAD >= MAX_PAGES as usize * 4096);
    }

    #[test]
    fn sqe_layout_matches_the_kernel_abi() {
        check_sqe_layout().unwrap();
        let ent = Ent::new();
        let raw = sqe_bytes(fuse_sqe(
            7,
            FUSE_IO_URING_CMD_COMMIT_AND_FETCH,
            3,
            0xabcd,
            &ent,
            2,
        ));
        assert_eq!(raw[0], IORING_OP_URING_CMD);
        assert_eq!(u32_at(&raw, 4), 7);
        assert_eq!(u32_at(&raw, 8), FUSE_IO_URING_CMD_COMMIT_AND_FETCH);
        assert_eq!(u64_at(&raw, 16), ent.iov as u64);
        assert_eq!(u32_at(&raw, 24), 2);
        assert_eq!(u64_at(&raw, 32), 2);
        // The command, struct fuse_uring_cmd_req, starts at byte 48.
        assert_eq!(u64_at(&raw, 48 + offset_of!(FuseUringCmdReq, flags)), 0);
        assert_eq!(
            u64_at(&raw, 48 + offset_of!(FuseUringCmdReq, commit_id)),
            0xabcd
        );
        assert_eq!(u16_at(&raw, 48 + offset_of!(FuseUringCmdReq, qid)), 3);
        // SAFETY: ent.iov is live (ent is), and nothing writes it.
        #[allow(unsafe_code)] // reading back what Ent::new wrote
        let iov = unsafe { &*ent.iov };
        assert_eq!((iov[0].iov_len, iov[1].iov_len), (RING_HEADER, PAYLOAD));
    }

    #[test]
    fn cpu_lists() {
        assert_eq!(
            parse_cpu_list("0-17\n").unwrap(),
            (0..18).collect::<Vec<_>>()
        );
        assert_eq!(parse_cpu_list("0").unwrap(), [0]);
        assert_eq!(
            parse_cpu_list("0-3,8-11").unwrap(),
            [0, 1, 2, 3, 8, 9, 10, 11]
        );
        assert_eq!(parse_cpu_list("0,2,4-5").unwrap(), [0, 2, 4, 5]);
        assert_eq!(parse_cpu_list("1023-1025").unwrap(), [1023, 1024, 1025]);
        for bad in ["", "a", "3-1", "0-3,2-5", "1,0", "0-", "-1"] {
            assert!(parse_cpu_list(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn pinning_beyond_cpu_setsize_is_an_error_not_a_panic() {
        assert!(pin_to_cpu(libc::CPU_SETSIZE as usize).is_err());
        assert!(pin_to_cpu(100_000).is_err());
    }
}
