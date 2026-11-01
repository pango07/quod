use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::{Entry, Hash, Kind, ROOT, Tree, child_path};

/// Writes everything it is given to the inner file while hashing it.
struct HashingWriter<'a> {
    file: &'a mut fs::File,
    hasher: blake3::Hasher,
}

impl Write for HashingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.file.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Files at or below this size are read whole; larger ones are streamed.
const STREAM_THRESHOLD: u64 = 1 << 20;

/// The mode every blob gets, however it arrived (see `take_files`).
/// Every blob has this mode, however its bytes arrived. A convention, not
/// the protection: a step reaches blobs only through a read-only bind
/// (MS_RDONLY, EROFS for everyone including the root-mapped step user), and
/// nothing in the store writes into an existing blob. 0444 because blobs are
/// immutable, and because it is the mode the Go module cache gives its
/// files, so a snapshot of one can hard-link blobs instead of copying
/// (`link_entries` copies when the modes differ; a store written before
/// this convention, with 0644 blobs, is copied in full once per snapshot).
const BLOB_MODE: u32 = 0o444;

/// A file's identity as `take_files` saw it. The inode says it is the same
/// file; size and mtime say nothing wrote to it (rename and chmod change
/// neither, so the check survives the move).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Stamp {
    ino: u64,
    size: u64,
    mtime: (i64, i64),
}

impl Stamp {
    fn of(m: &fs::Metadata) -> Stamp {
        Stamp {
            ino: m.ino(),
            size: m.len(),
            mtime: (m.mtime(), m.mtime_nsec()),
        }
    }
}

/// What phase one of `take_files` learned about a file.
#[derive(Clone, Copy)]
struct Taken {
    h: Hash,
    new: bool,
    /// Has other links: copy it rather than move it.
    copy: bool,
    stamp: Stamp,
}

/// BLAKE3 of everything `r` yields, read in large chunks so hashing a file
/// already in the page cache is bound by the hash, not by syscalls.
fn hash_reader(r: &mut impl io::Read) -> io::Result<Hash> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 256 << 10];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Hash::from_blake3(hasher.finalize()))
}

#[derive(Debug, thiserror::Error)]
pub enum CasError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("path is not UTF-8: {0:?}")]
    NonUtf8(PathBuf),
    #[error("file name contains a tab or newline, which listings cannot represent: {0:?}")]
    BadName(PathBuf),
    #[error("unsupported file type (not a file, directory or symlink): {0:?}")]
    Unsupported(PathBuf),
    #[error("blob {0} is not in the store")]
    Missing(Hash),
    #[error("{0:?} changed while it was being stored; nothing is recorded")]
    Changed(PathBuf),
}

trait IoContext<T> {
    fn at(self, path: &Path) -> Result<T, CasError>;
}

impl<T> IoContext<T> for io::Result<T> {
    fn at(self, path: &Path) -> Result<T, CasError> {
        self.map_err(|source| CasError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportStats {
    pub files: u64,
    pub bytes: u64,
    pub dirs: u64,
    pub symlinks: u64,
    /// Bytes newly written to the store by this import.
    pub new_bytes: u64,
}

/// Blobs on disk at `<dir>/<first 2 hex>/<hex>`. Writes go to a temp file and
/// are renamed into place, so a crash never leaves a partial blob under its
/// hash.
#[derive(Clone, Debug)]
pub struct Cas {
    dir: PathBuf,
}

impl Cas {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Cas, CasError> {
        let dir = dir.into();
        fs::create_dir_all(dir.join("tmp")).at(&dir)?;
        Ok(Cas { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn blob_path(&self, h: &Hash) -> PathBuf {
        let hex = h.hex();
        self.dir.join(&hex[..2]).join(hex)
    }

    pub fn has(&self, h: &Hash) -> bool {
        self.blob_path(h).is_file()
    }

    pub fn get(&self, h: &Hash) -> Result<Option<Vec<u8>>, CasError> {
        let p = self.blob_path(h);
        match fs::read(&p) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).at(&p),
        }
    }

    pub fn put(&self, bytes: &[u8]) -> Result<Hash, CasError> {
        let h = Hash::of(bytes);
        if !self.has(&h) {
            self.write_blob(&h, |f| f.write_all(bytes))?;
        }
        Ok(h)
    }

    /// Hashes a file and stores it if new. Returns the hash and the number of
    /// bytes newly written.
    pub fn put_file(&self, path: &Path) -> Result<(Hash, u64), CasError> {
        let meta = fs::metadata(path).at(path)?;
        if meta.len() <= STREAM_THRESHOLD {
            let bytes = fs::read(path).at(path)?;
            let h = Hash::of(&bytes);
            if self.has(&h) {
                return Ok((h, 0));
            }
            self.write_blob(&h, |f| f.write_all(&bytes))?;
            return Ok((h, bytes.len() as u64));
        }
        // Hash in place first: a blob the store already has (nearly every
        // file of an unchanged dependency tree) costs one read, no copy.
        // The bytes hashed are what the caller's tree records, as for a
        // small file read once.
        let h = hash_reader(&mut fs::File::open(path).at(path)?).at(path)?;
        if self.has(&h) {
            return Ok((h, 0));
        }
        // Stream once into a temp file while hashing, then rename to the hash
        // of the bytes actually stored: a file changing underneath can never
        // put wrong bytes under a hash.
        let tmp = self.tmp_path("stream");
        let mut f = fs::File::create(&tmp).at(&tmp)?;
        let mut w = HashingWriter {
            file: &mut f,
            hasher: blake3::Hasher::new(),
        };
        let size = io::copy(&mut fs::File::open(path).at(path)?, &mut w).at(path)?;
        let h = Hash::from_blake3(w.hasher.finalize());
        if self.has(&h) {
            drop(f);
            fs::remove_file(&tmp).at(&tmp)?;
            return Ok((h, 0));
        }
        f.sync_data().at(&tmp)?;
        f.set_permissions(fs::Permissions::from_mode(BLOB_MODE))
            .at(&tmp)?;
        drop(f);
        let dest = self.blob_path(&h);
        let parent = dest.parent().unwrap();
        fs::create_dir_all(parent).at(parent)?;
        fs::rename(&tmp, &dest).at(&dest)?;
        Ok((h, size))
    }

    /// Moves files into the store instead of copying them. For files the
    /// caller owns and is finished with (an overlay upper dir): afterwards
    /// each is gone unless its blob already existed, in which case it is left
    /// alone. Returns (hash, new bytes) in input order.
    ///
    /// Three phases, so a blob name never points at bytes that are not on
    /// disk (the same guarantee `put_file` gets from fsync and rename):
    /// every file is hashed in place; `sync` is called once with the store's
    /// directory and must flush that filesystem (the caller has syncfs(2));
    /// then each new file is moved to its blob path, verified under a
    /// staging name first (`move_in`), metadata operations only.
    /// Storing a step's outputs thus costs one read of the bytes and one
    /// filesystem flush, not a second write and an fsync per file. Across
    /// filesystems a file falls back to `put_file`, durable on its own.
    ///
    /// The bytes under a blob name must be the bytes that were hashed. The
    /// caller guarantees nothing writes to these files any more; this
    /// method checks it: each file's identity (inode, size, mtime) is taken
    /// before hashing and compared after hashing and again on the staged
    /// name, before it gets the blob name. Any difference removes it and fails with
    /// `CasError::Changed`, so a run whose outputs moved under it records
    /// nothing. A file with more than one link is copied, not moved, since
    /// its other names could still write to it.
    pub fn take_files(
        &self,
        files: &[PathBuf],
        sync: impl FnOnce(&Path) -> io::Result<()>,
    ) -> Result<Vec<(Hash, u64)>, CasError> {
        self.take_files_with(files, sync, &|_| {})
    }

    /// `take_files` with `staged` called for each new blob once it has its
    /// staging name and before it is verified (tests look at the store
    /// there).
    fn take_files_with(
        &self,
        files: &[PathBuf],
        sync: impl FnOnce(&Path) -> io::Result<()>,
        staged: &(dyn Fn(&Hash) + Sync),
    ) -> Result<Vec<(Hash, u64)>, CasError> {
        let hashed = self.par_map(files, |path| {
            let mut f = fs::File::open(path).at(path)?;
            let meta = f.metadata().at(path)?;
            let stamp = Stamp::of(&meta);
            let h = hash_reader(&mut f).at(path)?;
            if Stamp::of(&f.metadata().at(path)?) != stamp {
                return Err(CasError::Changed(path.to_path_buf()));
            }
            let new = !self.has(&h);
            let copy = meta.nlink() > 1;
            // Every blob looks alike regardless of how it arrived: the mode a
            // step gave its file must not decide whether the store can read it.
            if new && !copy && meta.mode() & 0o777 != BLOB_MODE {
                f.set_permissions(fs::Permissions::from_mode(BLOB_MODE))
                    .at(path)?;
            }
            Ok(Taken {
                h,
                new,
                copy,
                stamp,
            })
        })?;
        // One move per hash: a second file with the same new content stays
        // where it is, like a file whose blob already existed.
        let mut seen = std::collections::HashSet::new();
        let moves: Vec<(&Path, Taken)> = files
            .iter()
            .zip(&hashed)
            .filter(|(_, t)| t.new && seen.insert(t.h))
            .map(|(p, t)| (p.as_path(), *t))
            .collect();
        if !moves.is_empty() {
            sync(&self.dir).at(&self.dir)?;
        }
        self.par_map(&moves, |(path, t)| self.move_in(path, t, &staged))?;
        Ok(hashed
            .into_iter()
            .map(|t| (t.h, if t.new { t.stamp.size } else { 0 }))
            .collect())
    }

    /// Moves one hashed file to its blob name. Nothing is ever visible under
    /// the blob name unverified, and nothing another writer stored there is
    /// ever replaced or removed:
    ///
    /// 1. the file gets a second name in the blob's own directory, a staging
    ///    name no reader looks up (`.<hash>.<pid>.<seq>`): a link, or where
    ///    links are refused (fs.protected_hardlinks, a filesystem without
    ///    hard links, a full link count) a rename, which cannot replace
    ///    anything under a fresh name;
    /// 2. the staged inode is checked to be the one hashed, unchanged
    ///    (ino, size, mtime), else it goes and the run fails;
    /// 3. the staging name is linked to the blob name; EEXIST means another
    ///    writer stored the same content meanwhile, which stays as it is.
    ///    Where links are refused, a rename, only while the blob name is
    ///    absent (without renameat2's RENAME_NOREPLACE a writer in between
    ///    could still be replaced, by these same verified bytes, never
    ///    removed);
    /// 4. the staging name goes, then the file's own name.
    ///
    /// Links rather than one rename: a rename across directories takes the
    /// filesystem-wide rename lock, and ten thousand of them (a Go build
    /// cache's action entries, new on every run) serialized to ~0.4s; links
    /// take only their directories' locks. A crash leaves at most a staging
    /// name in the blob directory, never under a blob name.
    fn move_in(
        &self,
        path: &Path,
        t: &Taken,
        staged: &(dyn Fn(&Hash) + Sync),
    ) -> Result<(), CasError> {
        let changed = || CasError::Changed(path.to_path_buf());
        if t.copy {
            return (self.put_file(path)?.0 == t.h)
                .then_some(())
                .ok_or_else(changed);
        }
        let dest = self.blob_path(&t.h);
        let parent = dest.parent().unwrap();
        fs::create_dir_all(parent).at(parent)?;
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let stage = parent.join(format!(
            ".{}.{}.{}",
            t.h.hex(),
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        // 1. A staging name.
        let linked = match fs::hard_link(path, &stage) {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                return (self.put_file(path)?.0 == t.h)
                    .then_some(())
                    .ok_or_else(changed);
            }
            Err(_) => match fs::rename(path, &stage) {
                Ok(()) => false,
                Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                    return (self.put_file(path)?.0 == t.h)
                        .then_some(())
                        .ok_or_else(changed);
                }
                Err(e) => return Err(e).at(&stage),
            },
        };
        staged(&t.h);
        // 2. The inode that was hashed, unchanged since.
        if Stamp::of(&fs::symlink_metadata(&stage).at(&stage)?) != t.stamp {
            let _ = fs::remove_file(&stage);
            return Err(changed());
        }
        // 3. Under the blob name, unless someone stored it first.
        match fs::hard_link(&stage, &dest) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) if !dest.exists() => {
                fs::rename(&stage, &dest).at(&dest)?;
            }
            Err(_) => {}
        }
        // 4. The staging name, then the file's own (a rename moved it).
        match fs::remove_file(&stage) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).at(&stage),
        }
        if linked {
            fs::remove_file(path).at(path)?;
        }
        Ok(())
    }

    /// Copies a blob into `dest` (which must not exist).
    pub fn copy_out(&self, h: &Hash, dest: &Path) -> Result<(), CasError> {
        let src = self.blob_path(h);
        if !src.is_file() {
            return Err(CasError::Missing(*h));
        }
        fs::copy(&src, dest).at(dest)?;
        Ok(())
    }

    /// Opens a blob for streaming reads.
    pub fn open_blob(&self, h: &Hash) -> Result<fs::File, CasError> {
        let p = self.blob_path(h);
        fs::File::open(&p).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => CasError::Missing(*h),
            _ => CasError::Io { path: p, source: e },
        })
    }

    fn write_blob(
        &self,
        h: &Hash,
        write: impl FnOnce(&mut fs::File) -> io::Result<()>,
    ) -> Result<(), CasError> {
        let dest = self.blob_path(h);
        let parent = dest.parent().unwrap();
        fs::create_dir_all(parent).at(parent)?;
        let tmp = self.tmp_path(&h.hex());
        let mut f = fs::File::create(&tmp).at(&tmp)?;
        write(&mut f)
            .and_then(|()| f.sync_data())
            .and_then(|()| f.set_permissions(fs::Permissions::from_mode(BLOB_MODE)))
            .at(&tmp)?;
        drop(f);
        fs::rename(&tmp, &dest).at(&dest)
    }

    fn tmp_path(&self, tag: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        self.dir
            .join("tmp")
            .join(format!("{tag}.{}.{seq}", std::process::id()))
    }

    pub fn import_dir(&self, root: &Path) -> Result<Tree, CasError> {
        self.import_dir_stats(root).map(|(t, _)| t)
    }

    /// `import_dir` without the named top-level entries, which are neither
    /// read nor stored (a checkout's `.git`, for a caller that serves its
    /// own): the tree `import_dir(root)` gives with them removed.
    pub fn import_dir_except(&self, root: &Path, skip: &[&str]) -> Result<Tree, CasError> {
        self.import_dir_walk(root, skip).map(|(t, _)| t)
    }

    /// Walks `root`, stores every regular file and returns the tree. Skips
    /// nothing; `.git` included.
    pub fn import_dir_stats(&self, root: &Path) -> Result<(Tree, ImportStats), CasError> {
        self.import_dir_walk(root, &[])
    }

    fn import_dir_walk(&self, root: &Path, skip: &[&str]) -> Result<(Tree, ImportStats), CasError> {
        let mut stats = ImportStats::default();
        let mut files: Vec<(String, PathBuf, u32)> = Vec::new();
        let mut leaves: Vec<Entry> = Vec::new();
        let mut stack = vec![(ROOT.to_string(), root.to_path_buf())];
        while let Some((rel, abs)) = stack.pop() {
            stats.dirs += 1;
            leaves.push(Entry::dir(rel.clone(), Hash::of(b"")));
            for item in fs::read_dir(&abs).at(&abs)? {
                let item = item.at(&abs)?;
                let path = item.path();
                let name = item
                    .file_name()
                    .into_string()
                    .map_err(|_| CasError::NonUtf8(path.clone()))?;
                if rel == ROOT && skip.contains(&name.as_str()) {
                    continue;
                }
                if !crate::valid_name(&name) {
                    return Err(CasError::BadName(path));
                }
                let child = child_path(&rel, &name);
                let meta = fs::symlink_metadata(&path).at(&path)?;
                let ft = meta.file_type();
                if ft.is_dir() {
                    stack.push((child, path));
                } else if ft.is_file() {
                    stats.files += 1;
                    stats.bytes += meta.len();
                    files.push((child, path, meta.mode() & 0o777));
                } else if ft.is_symlink() {
                    stats.symlinks += 1;
                    let target = fs::read_link(&path).at(&path)?;
                    let target = target
                        .into_os_string()
                        .into_string()
                        .map_err(|_| CasError::NonUtf8(path.clone()))?;
                    leaves.push(Entry::symlink(child, target));
                } else {
                    return Err(CasError::Unsupported(path));
                }
            }
        }

        let paths: Vec<PathBuf> = files.iter().map(|(_, p, _)| p.clone()).collect();
        let hashed = self.put_files(&paths)?;
        for ((rel, _, mode), (h, new)) in files.into_iter().zip(hashed) {
            stats.new_bytes += new;
            leaves.push(Entry::file(rel, h, mode));
        }
        Ok((Tree::from_leaves(leaves), stats))
    }

    /// `put_file` for many files in parallel. Returns (hash, new bytes) in
    /// input order.
    pub fn put_files(&self, files: &[PathBuf]) -> Result<Vec<(Hash, u64)>, CasError> {
        self.par_map(files, |p| self.put_file(p))
    }

    /// Applies `f` to every item on a pool of threads; results in input
    /// order, the first error wins.
    fn par_map<I: Sync, T: Send + Clone>(
        &self,
        items: &[I],
        f: impl Fn(&I) -> Result<T, CasError> + Sync,
    ) -> Result<Vec<T>, CasError> {
        let n = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(16);
        let next = AtomicUsize::new(0);
        let out: Mutex<Vec<Option<T>>> = Mutex::new(vec![None; items.len()]);
        let first_err: Mutex<Option<CasError>> = Mutex::new(None);
        std::thread::scope(|s| {
            for _ in 0..n {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= items.len() || first_err.lock().unwrap().is_some() {
                            break;
                        }
                        match f(&items[i]) {
                            Ok(r) => out.lock().unwrap()[i] = Some(r),
                            Err(e) => {
                                first_err.lock().unwrap().get_or_insert(e);
                            }
                        }
                    }
                });
            }
        });
        if let Some(e) = first_err.into_inner().unwrap() {
            return Err(e);
        }
        Ok(out
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|r| r.unwrap())
            .collect())
    }

    /// Writes `entries` (f, d and l) under `dest`, with modes and symlinks.
    /// Existing paths are replaced.
    pub fn materialize_entries<'a>(
        &self,
        entries: impl IntoIterator<Item = &'a Entry>,
        dest: &Path,
    ) -> Result<(), CasError> {
        fs::create_dir_all(dest).at(dest)?;
        // Directories first, so parents exist and their modes do not block writes.
        let (dirs, rest): (Vec<&Entry>, Vec<&Entry>) =
            entries.into_iter().partition(|e| e.k == Kind::Dir);
        for e in dirs {
            let p = join(dest, &e.p);
            if fs::symlink_metadata(&p).is_ok_and(|m| !m.is_dir()) {
                fs::remove_file(&p).at(&p)?;
            }
            fs::create_dir_all(&p).at(&p)?;
        }
        for e in rest {
            let p = join(dest, &e.p);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).at(parent)?;
            }
            remove_any(&p)?;
            match e.k {
                Kind::File => {
                    let h = e.h.expect("file entry without hash");
                    self.copy_out(&h, &p)?;
                    let mode = e.m.unwrap_or(0o644);
                    fs::set_permissions(&p, fs::Permissions::from_mode(mode)).at(&p)?;
                }
                Kind::Symlink => {
                    symlink(e.t.as_deref().expect("symlink entry without target"), &p).at(&p)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Writes a whole tree to `dest`.
    pub fn materialize(&self, tree: &Tree, dest: &Path) -> Result<(), CasError> {
        self.materialize_entries(tree.iter(), dest)
    }

    /// Like `materialize_entries`, but a file whose mode is the store's
    /// (`BLOB_MODE`) is a hard link to its blob instead of a copy: a
    /// snapshot of exactly the hashed content at one syscall per file. The
    /// store never writes into an existing blob (new blobs arrive by
    /// rename), so the view cannot change under a reader. A file with any
    /// other mode is copied, because a hard link shares its inode's mode and
    /// a blob may be linked from several snapshots: no snapshot ever changes
    /// a blob's mode. Across filesystems (EXDEV) files are copied too.
    pub fn link_entries<'a>(
        &self,
        entries: impl IntoIterator<Item = &'a Entry>,
        dest: &Path,
    ) -> Result<(), CasError> {
        fs::create_dir_all(dest).at(dest)?;
        let (dirs, rest): (Vec<&Entry>, Vec<&Entry>) =
            entries.into_iter().partition(|e| e.k == Kind::Dir);
        for e in dirs {
            let p = join(dest, &e.p);
            fs::create_dir_all(&p).at(&p)?;
        }
        for e in rest {
            let p = join(dest, &e.p);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).at(parent)?;
            }
            match e.k {
                Kind::File => {
                    let h = e.h.expect("file entry without hash");
                    let mode = e.m.unwrap_or(BLOB_MODE);
                    let blob = self.blob_path(&h);
                    let blob_mode = fs::metadata(&blob)
                        .map_err(|e| match e.kind() {
                            io::ErrorKind::NotFound => CasError::Missing(h),
                            _ => CasError::Io {
                                path: blob.clone(),
                                source: e,
                            },
                        })?
                        .mode()
                        & 0o777;
                    let linked = blob_mode == mode
                        && match fs::hard_link(&blob, &p) {
                            Ok(()) => true,
                            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => false,
                            Err(e) => return Err(e).at(&p),
                        };
                    if !linked {
                        self.copy_out(&h, &p)?;
                        fs::set_permissions(&p, fs::Permissions::from_mode(mode)).at(&p)?;
                    }
                }
                Kind::Symlink => {
                    symlink(e.t.as_deref().expect("symlink entry without target"), &p).at(&p)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn join(base: &Path, rel: &str) -> PathBuf {
    if rel == ROOT {
        base.to_path_buf()
    } else {
        base.join(rel)
    }
}

/// Removes a file, symlink or directory tree at `p` if present, making
/// read-only directories writable first.
pub fn remove_any(p: &Path) -> Result<(), CasError> {
    let meta = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).at(p),
    };
    if meta.is_dir() {
        make_writable(p)?;
        fs::remove_dir_all(p).at(p)
    } else {
        fs::remove_file(p).at(p)
    }
}

fn make_writable(dir: &Path) -> Result<(), CasError> {
    fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).at(dir)?;
    for item in fs::read_dir(dir).at(dir)? {
        let item = item.at(dir)?;
        if item.file_type().at(dir)?.is_dir() {
            make_writable(&item.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod move_tests {
    use super::*;

    #[test]
    fn nothing_is_under_the_blob_name_before_it_is_verified() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let out = tmp.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("a"), b"a new output").unwrap();
        let h = Hash::of(b"a new output");
        let looked = AtomicU64::new(0);
        let got = cas
            .take_files_with(&[out.join("a")], |_| Ok(()), &|staged| {
                assert_eq!(*staged, h);
                // Between staging and the check: not visible as h yet.
                assert!(!cas.has(staged), "the blob name exists before the check");
                looked.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap();
        assert_eq!(looked.load(Ordering::Relaxed), 1);
        assert_eq!(got, vec![(h, 12)]);
        assert_eq!(cas.get(&h).unwrap().unwrap(), b"a new output");
        assert!(!out.join("a").exists());
        // Only the blob in its directory: no staging name left.
        let dir = cas.blob_path(&h).parent().unwrap().to_path_buf();
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(h.hex())]);
        assert_eq!(fs::metadata(cas.blob_path(&h)).unwrap().nlink(), 1);
    }

    #[test]
    fn a_file_changed_before_the_check_never_gets_the_blob_name() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let out = tmp.path().join("out");
        fs::create_dir(&out).unwrap();
        let file = out.join("a");
        fs::write(&file, b"hashed bytes").unwrap();
        let h = Hash::of(b"hashed bytes");
        // A writer the caller did not stop holds the file open already (the
        // move makes it read-only, so reopening would fail for non-root).
        let writer = Mutex::new(fs::OpenOptions::new().append(true).open(&file).unwrap());
        let err = cas
            .take_files_with(std::slice::from_ref(&file), |_| Ok(()), &|_| {
                // Other bytes, other size.
                writer.lock().unwrap().write_all(b" and more").unwrap();
            })
            .unwrap_err();
        assert!(matches!(err, CasError::Changed(_)), "{err}");
        assert!(!cas.has(&h), "unverified bytes under the blob name");
        let dir = cas.blob_path(&h).parent().unwrap().to_path_buf();
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            0,
            "no staging name left"
        );
    }

    #[test]
    fn a_blob_stored_meanwhile_is_left_as_it_is() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let out = tmp.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("a"), b"same content").unwrap();
        let h = Hash::of(b"same content");
        let theirs = AtomicU64::new(0);
        cas.take_files_with(&[out.join("a")], |_| Ok(()), &|staged| {
            // Another writer stores the same content first.
            cas.put(b"same content").unwrap();
            let ino = fs::metadata(cas.blob_path(staged)).unwrap().ino();
            theirs.store(ino, Ordering::Relaxed);
        })
        .unwrap();
        let ino = theirs.load(Ordering::Relaxed);
        assert_eq!(
            fs::metadata(cas.blob_path(&h)).unwrap().ino(),
            ino,
            "theirs was replaced"
        );
        assert_eq!(cas.get(&h).unwrap().unwrap(), b"same content");
    }
}
