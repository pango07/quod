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
        drop(f);
        let dest = self.blob_path(&h);
        let parent = dest.parent().unwrap();
        fs::create_dir_all(parent).at(parent)?;
        fs::rename(&tmp, &dest).at(&dest)?;
        Ok((h, size))
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
        write(&mut f).and_then(|()| f.sync_data()).at(&tmp)?;
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

    /// Walks `root`, stores every regular file and returns the tree. Skips
    /// nothing; `.git` included.
    pub fn import_dir_stats(&self, root: &Path) -> Result<(Tree, ImportStats), CasError> {
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
        let n = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(16);
        let next = AtomicUsize::new(0);
        let out: Mutex<Vec<Option<(Hash, u64)>>> = Mutex::new(vec![None; files.len()]);
        let first_err: Mutex<Option<CasError>> = Mutex::new(None);
        std::thread::scope(|s| {
            for _ in 0..n {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= files.len() || first_err.lock().unwrap().is_some() {
                            break;
                        }
                        match self.put_file(&files[i]) {
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
