//! The served git index: a `.git/index` whose stat data is what the observer
//! serves for each path, so `git status` (go build's VCS stamping runs it)
//! trusts the stat data of unchanged files instead of reading and hashing
//! every one of them, exactly as it does in a fresh checkout on GitHub's
//! runner.
//!
//! What git does with an entry (read-cache.c, `ie_match_stat`): it lstat()s
//! the path and compares mtime, ctime, inode, uid, gid and size (and the
//! mode's type and exec bit) with the entry's; if they agree, and the entry
//! is not racily clean (its mtime at or after the index file's own), the
//! file is taken to hold the entry's blob, unread. Otherwise git reads and
//! hashes the file and compares the object id.
//!
//! Why recording stays correct. The observer records every lookup and
//! getattr of a file as the file's content hash (`f`), not as its
//! attributes: the attributes it serves are a function of the content
//! (`content_mtime`, the blob's size), so a stat already distinguishes
//! contents, and a program that trusts stat data (git, a build cache) is
//! keyed on exactly what decides it. Serving matching stat data therefore
//! changes which files git reads, never what the manifest says it depends
//! on: every tracked path git lstat()s is still an `f` entry with its
//! content hash, and the index itself, which git reads, is one too.
//!
//! The index is refreshed against the tree that is served, per step, by the
//! caller (`refresh_index`): an entry gets the served stat data only when
//! the served content is the entry's blob (its git object id, computed from
//! the store and memoized), and its stat data is cleared otherwise, so git
//! looks at that file whatever the entry said before. Entries git does not
//! stat (unmerged stages, skip-worktree, assume-valid, intent-to-add,
//! gitlinks, sparse directories) are left as they are. An index this code
//! does not understand (a version other than 2-4, a checksum other than
//! SHA-1, a truncated file) is served unchanged: git then reads files, as
//! before, which is slower and just as correct.
//!
//! The served stat data, as a step sees it through the workspace overlay
//! (checked on Linux 7.0, overlayfs without xino): `st_ino` the observer's
//! inode number (`inode_numbers`; git keeps its low 32 bits), uid and gid
//! the step's own (the sandbox maps the serving process's ids onto them),
//! mtime and ctime `served_time`, size the blob's. `st_dev` differs per
//! mount; git compares it only when built with USE_STDEV (not the default),
//! and the entry keeps 0. The index file itself is served with an mtime
//! after every content mtime (`index_mtime`), so no entry is racily clean.

use std::io::Read;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use quod_cas::{Cas, Entry, Kind, ROOT, Tree};

use crate::FsError;

/// The index of a repository whose work tree is the served tree's root.
pub const INDEX: &str = ".git/index";

/// How far past every content mtime the index's own mtime is served: one
/// more than the span `content_mtime` uses.
const INDEX_OFFSET_SECS: u64 = 1 << 28;

/// The modification time served for the git index at `INDEX`: still a
/// function of its content, but always later than every `content_mtime`
/// (2008-07-05 .. 2017-01-06), as an index written after its files is. Git
/// treats an entry whose mtime is not older than the index's as racily
/// clean and reads the file; served files have random content-derived
/// times, so with an ordinary content mtime about half the entries would
/// be.
pub fn index_mtime(content: &quod_cas::Hash) -> SystemTime {
    crate::content_mtime(content) + std::time::Duration::from_secs(INDEX_OFFSET_SECS)
}

/// The mtime (and ctime, and atime) the observer serves for an entry:
/// `content_mtime` for a file (`index_mtime` for the index at `INDEX`),
/// the epoch for directories and symlinks.
pub fn served_time(e: &Entry) -> SystemTime {
    match (e.k, e.h) {
        (Kind::File, Some(h)) if e.p == INDEX => index_mtime(&h),
        (Kind::File, Some(h)) => crate::content_mtime(&h),
        _ => UNIX_EPOCH,
    }
}

/// The inode number the observer serves for every entry of `tree`, root
/// first, the rest in the tree's bytewise path order: the root is 1 (FUSE's
/// root id); any other path gets a number from its own name (BLAKE3 of the
/// path, in 2..2^32: git keeps 32 bits, and overlayfs's xino needs high bits
/// free), the next free one on a collision. So a path's number does not
/// move when other paths come and go (Quod's own state under `.quod/`
/// between jobs, a step's outputs), and neither does the served index's
/// record of it; only a collision, which the probe resolves in path order,
/// can move one, and that only costs git a read.
pub fn inode_numbers(tree: &Tree) -> Vec<(&Entry, u64)> {
    const SPAN: u64 = (1 << 32) - 2;
    let root = tree.get(ROOT).expect("tree without a root");
    let mut used = std::collections::HashSet::with_capacity(tree.len());
    let mut out = Vec::with_capacity(tree.len());
    used.insert(1);
    out.push((root, 1));
    for e in tree.iter().filter(|e| e.p != ROOT) {
        let h = quod_cas::Hash::of(e.p.as_bytes());
        let b = h.as_bytes();
        let mut ino = u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])) % SPAN + 2;
        while !used.insert(ino) {
            ino = (ino - 2 + 1) % SPAN + 2;
        }
        out.push((e, ino));
    }
    out
}

/// What `refresh_index` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refreshed {
    /// Index entries.
    pub entries: usize,
    /// Entries now carrying the served stat data: the served content is the
    /// entry's blob, so git will not read the file.
    pub matched: usize,
    /// Entries whose stat data was cleared: the path is missing, of another
    /// kind, or holds other content; git will look at it.
    pub cleared: usize,
    /// Entries git does not stat, left as they were.
    pub skipped: usize,
}

/// The outcome of `refresh_index`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// The tree has no index at `INDEX`.
    NoIndex,
    /// The index is served as it is (the reason).
    Unchanged(String),
    Refreshed(Refreshed),
}

/// `tree` with its git index (`INDEX`) refreshed against the tree itself,
/// for a step that runs as `uid`:`gid` (see the module docs). The tree is
/// returned unchanged when it has no index or one this code does not
/// understand; only the index entry ever changes.
pub fn refresh_index(
    tree: &Tree,
    cas: &Cas,
    uid: u32,
    gid: u32,
) -> Result<(Tree, Refresh), FsError> {
    let Some(entry) = tree.get(INDEX).filter(|e| e.k == Kind::File) else {
        return Ok((tree.clone(), Refresh::NoIndex));
    };
    let h = entry.h.expect("file entry without hash");
    let mut bytes = cas
        .get(&h)?
        .ok_or(FsError::Cas(quod_cas::CasError::Missing(h)))?;
    let entries = match parse(&bytes) {
        Ok(e) => e,
        Err(why) => return Ok((tree.clone(), Refresh::Unchanged(why))),
    };
    let inos: std::collections::HashMap<&str, u64> = inode_numbers(tree)
        .into_iter()
        .map(|(e, i)| (e.p.as_str(), i))
        .collect();
    let mut stats = Refreshed {
        entries: entries.len(),
        ..Refreshed::default()
    };
    for ie in &entries {
        if ie.untouched() {
            stats.skipped += 1;
            continue;
        }
        let served = std::str::from_utf8(&ie.path)
            .ok()
            .and_then(|p| tree.get(p).map(|e| (e, inos[p])));
        let stat = match served {
            Some((e, ino)) => served_stat(cas, e, ino, ie, uid, gid)?,
            None => None,
        };
        match stat {
            Some(s) => {
                stats.matched += 1;
                s.write(&mut bytes[ie.offset..]);
            }
            None => {
                stats.cleared += 1;
                Stat::default().write(&mut bytes[ie.offset..]);
            }
        }
    }
    let body = bytes.len() - 20;
    if bytes[body..] != [0u8; 20] {
        // An all-zero trailer (index.skipHash) stays zero.
        let sum = sha1(&bytes[..body]);
        bytes[body..].copy_from_slice(&sum);
    }
    let refreshed = cas.put(&bytes)?;
    let mode = entry.m.unwrap_or(0o644);
    Ok((
        tree.with_leaf_changed(Entry::file(INDEX, refreshed, mode)),
        Refresh::Refreshed(stats),
    ))
}

/// The stat data a step sees for `e`, when `e` holds the index entry's
/// object; `None` when it does not (or is of another kind).
fn served_stat(
    cas: &Cas,
    e: &Entry,
    ino: u64,
    ie: &IndexEntry,
    uid: u32,
    gid: u32,
) -> Result<Option<Stat>, FsError> {
    let (id, size) = match (e.k, ie.mode & 0o170000) {
        (Kind::File, 0o100000) => {
            let h = e.h.expect("file entry without hash");
            blob_id(cas, &h)?
        }
        (Kind::Symlink, 0o120000) => {
            let t = e.t.as_deref().unwrap_or("").as_bytes();
            (object_id(t), t.len() as u64)
        }
        _ => return Ok(None),
    };
    if id != ie.oid {
        return Ok(None);
    }
    let t = served_time(e)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(Some(Stat {
        secs: t.as_secs() as u32,
        nanos: t.subsec_nanos(),
        ino: ino as u32,
        uid,
        gid,
        size: size as u32,
    }))
}

/// The on-disk stat fields this code writes: ctime and mtime (both the
/// served time), dev (always 0), ino, uid, gid, size. Mode is never touched.
#[derive(Clone, Copy, Default)]
struct Stat {
    secs: u32,
    nanos: u32,
    ino: u32,
    uid: u32,
    gid: u32,
    size: u32,
}

impl Stat {
    /// Into an on-disk entry starting at `b[0]`.
    fn write(&self, b: &mut [u8]) {
        let put = |b: &mut [u8], at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_be_bytes());
        put(b, 0, self.secs); // ctime
        put(b, 4, self.nanos);
        put(b, 8, self.secs); // mtime
        put(b, 12, self.nanos);
        put(b, 16, 0); // dev
        put(b, 20, self.ino);
        put(b, 28, self.uid);
        put(b, 32, self.gid);
        put(b, 36, self.size);
    }
}

/// One index entry: where it starts, and what decides whether it is
/// refreshed.
#[derive(Debug)]
struct IndexEntry {
    offset: usize,
    mode: u32,
    oid: [u8; 20],
    flags: u16,
    extended: u16,
    path: Vec<u8>,
}

impl IndexEntry {
    /// An entry git never takes stat data from, or one whose stat data
    /// means something else: left exactly as it is.
    fn untouched(&self) -> bool {
        const ASSUME_VALID: u16 = 0x8000;
        const STAGE: u16 = 0x3000;
        const SKIP_WORKTREE: u16 = 0x4000;
        const INTENT_TO_ADD: u16 = 0x2000;
        let kind = self.mode & 0o170000;
        self.flags & (ASSUME_VALID | STAGE) != 0
            || self.extended & (SKIP_WORKTREE | INTENT_TO_ADD) != 0
            || !(kind == 0o100000 || kind == 0o120000)
    }
}

/// The entries of an index file (versions 2, 3 and 4), after checking its
/// header and SHA-1 trailer. Extensions are not interpreted: they hold no
/// per-entry stat data this code serves (the untracked cache's directory
/// stat data is validated by git itself and recomputed on a mismatch).
fn parse(b: &[u8]) -> Result<Vec<IndexEntry>, String> {
    if b.len() < 12 + 20 || &b[..4] != b"DIRC" {
        return Err("not a git index".into());
    }
    let be32 = |at: usize| u32::from_be_bytes(b[at..at + 4].try_into().unwrap());
    let be16 = |at: usize| u16::from_be_bytes(b[at..at + 2].try_into().unwrap());
    let version = be32(4);
    if !(2..=4).contains(&version) {
        return Err(format!("index version {version}"));
    }
    let body = b.len() - 20;
    let trailer = &b[body..];
    if trailer != [0u8; 20] && sha1(&b[..body]) != trailer {
        return Err("the index checksum is not SHA-1 (a SHA-256 repository?)".into());
    }
    let n = be32(8) as usize;
    let mut out = Vec::with_capacity(n.min(1 << 20));
    let mut pos = 12;
    let mut prev: Vec<u8> = Vec::new();
    for _ in 0..n {
        let start = pos;
        if start + 62 > body {
            return Err("truncated index entry".into());
        }
        let mode = be32(start + 24);
        let oid: [u8; 20] = b[start + 40..start + 60].try_into().unwrap();
        let flags = be16(start + 60);
        pos = start + 62;
        let mut extended = 0;
        if flags & 0x4000 != 0 {
            if version < 3 || pos + 2 > body {
                return Err("extended flags in a version 2 index".into());
            }
            extended = be16(pos);
            pos += 2;
        }
        let path = if version == 4 {
            let (strip, used) = varint(&b[pos..body]).ok_or("bad path prefix")?;
            pos += used;
            let nul = b[pos..body]
                .iter()
                .position(|&c| c == 0)
                .ok_or("unterminated path")?;
            let keep = prev
                .len()
                .checked_sub(strip as usize)
                .ok_or("path prefix longer than the previous path")?;
            let mut p = prev[..keep].to_vec();
            p.extend_from_slice(&b[pos..pos + nul]);
            pos += nul + 1;
            p
        } else {
            let len = (flags & 0xfff) as usize;
            let len = if len < 0xfff {
                len
            } else {
                b[pos..body]
                    .iter()
                    .position(|&c| c == 0)
                    .ok_or("unterminated path")?
            };
            if pos + len >= body {
                return Err("truncated index path".into());
            }
            let p = b[pos..pos + len].to_vec();
            // NUL-padded to a multiple of 8, with at least one NUL.
            pos = start + ((pos - start + len + 8) & !7);
            if pos > body {
                return Err("truncated index entry".into());
            }
            p
        };
        prev.clone_from(&path);
        out.push(IndexEntry {
            offset: start,
            mode,
            oid,
            flags,
            extended,
            path,
        });
    }
    Ok(out)
}

/// git's varint (varint.c): seven bits per byte, most significant first,
/// each continuation adding one.
fn varint(b: &[u8]) -> Option<(u64, usize)> {
    let mut i = 0;
    let mut c = *b.get(i)?;
    i += 1;
    let mut v = u64::from(c & 127);
    while c & 128 != 0 {
        v = v.checked_add(1)?;
        if v >> 57 != 0 {
            return None;
        }
        c = *b.get(i)?;
        i += 1;
        v = (v << 7) + u64::from(c & 127);
    }
    Some((v, i))
}

/// The git object id of a blob holding `content`.
pub fn object_id(content: &[u8]) -> [u8; 20] {
    let mut s = Sha1::new();
    s.update(format!("blob {}\0", content.len()).as_bytes());
    s.update(content);
    s.finish()
}

/// The git blob id and size of a stored blob, memoized in the store
/// (`<cas>/gitoid-v1/<hh>/<hash>`: the id in hex) so each blob is hashed
/// once.
pub fn blob_id(cas: &Cas, h: &quod_cas::Hash) -> Result<([u8; 20], u64), FsError> {
    let size = std::fs::metadata(cas.blob_path(h))
        .map_err(FsError::io(format!("blob {h}")))?
        .len();
    let memo = memo_path(cas, h);
    if let Ok(t) = std::fs::read_to_string(&memo)
        && let Some(id) = unhex(t.trim())
    {
        return Ok((id, size));
    }
    let mut f = cas.open_blob(h)?;
    let mut s = Sha1::new();
    s.update(format!("blob {size}\0").as_bytes());
    let mut buf = vec![0u8; 1 << 16];
    let mut seen = 0u64;
    loop {
        let n = f
            .read(&mut buf)
            .map_err(FsError::io(format!("read blob {h}")))?;
        if n == 0 {
            break;
        }
        seen += n as u64;
        s.update(&buf[..n]);
    }
    if seen != size {
        return Err(FsError::Io {
            what: format!("blob {h} changed size while hashed"),
            source: std::io::ErrorKind::InvalidData.into(),
        });
    }
    let id = s.finish();
    if let Some(dir) = memo.parent() {
        let tmp = memo.with_extension(format!("tmp.{}", std::process::id()));
        let _ = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&tmp, hex(&id)))
            .and_then(|()| std::fs::rename(&tmp, &memo));
    }
    Ok((id, size))
}

fn memo_path(cas: &Cas, h: &quod_cas::Hash) -> PathBuf {
    let x = h.hex();
    cas.dir().join("gitoid-v1").join(&x[..2]).join(x)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 20]> {
    if s.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// SHA-1 of `data`.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut s = Sha1::new();
    s.update(data);
    s.finish()
}

/// SHA-1 (FIPS 180-4), for git object ids and the index checksum. Not used
/// for anything that needs collision resistance: the store's hashes are
/// BLAKE3.
struct Sha1 {
    h: [u32; 5],
    block: [u8; 64],
    fill: usize,
    len: u64,
}

impl Sha1 {
    fn new() -> Sha1 {
        Sha1 {
            h: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0],
            block: [0; 64],
            fill: 0,
            len: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        while !data.is_empty() {
            let n = (64 - self.fill).min(data.len());
            self.block[self.fill..self.fill + n].copy_from_slice(&data[..n]);
            self.fill += n;
            data = &data[n..];
            if self.fill == 64 {
                let block = self.block;
                self.compress(&block);
                self.fill = 0;
            }
        }
    }

    fn finish(mut self) -> [u8; 20] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.fill != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 20];
        for (o, h) in out.chunks_mut(4).zip(self.h) {
            o.copy_from_slice(&h.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes(c.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = self.h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (h, v) in self.h.iter_mut().zip([a, b, c, d, e]) {
            *h = h.wrapping_add(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_vectors() {
        // FIPS 180-2 appendix A, and the 1,000,000 'a' message.
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        let mut s = Sha1::new();
        for _ in 0..1000 {
            s.update(&[b'a'; 1000]);
        }
        assert_eq!(hex(&s.finish()), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
        // git's empty blob and `git hash-object` of "hello\n".
        assert_eq!(
            hex(&object_id(b"")),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
        assert_eq!(
            hex(&object_id(b"hello\n")),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
    }

    #[test]
    fn inode_numbers_are_unique_and_follow_the_path_alone() {
        let h = quod_cas::Hash::of(b"x");
        let base: Vec<Entry> = (0..5000)
            .map(|i| Entry::file(format!("d{}/f{i}", i % 37), h, 0o644))
            .collect();
        let a = Tree::from_leaves(base.clone());
        let b = Tree::from_leaves(base.into_iter().chain([Entry::file(
            ".quod/gocache/new",
            h,
            0o644,
        )]));
        let map = |t: &Tree| -> std::collections::HashMap<String, u64> {
            inode_numbers(t)
                .into_iter()
                .map(|(e, i)| (e.p.clone(), i))
                .collect()
        };
        let (ma, mb) = (map(&a), map(&b));
        assert_eq!(ma["."], 1);
        let distinct: std::collections::HashSet<u64> = mb.values().copied().collect();
        assert_eq!(distinct.len(), mb.len(), "unique");
        assert!(mb.values().all(|&i| (1..1 << 32).contains(&i)));
        // Another path appearing moves no one else's number.
        for (p, i) in &ma {
            assert_eq!(mb[p], *i, "{p}");
        }
    }

    #[test]
    fn varints() {
        assert_eq!(varint(&[0]), Some((0, 1)));
        assert_eq!(varint(&[127]), Some((127, 1)));
        // 128 is 0x80 0x00 in git's encoding (each continuation adds one).
        assert_eq!(varint(&[0x80, 0x00]), Some((128, 2)));
        assert_eq!(varint(&[0x80]), None);
    }

    /// A v2 index built the way git writes one, for `paths` with the given
    /// contents: zero stat data, as `git read-tree` leaves it.
    fn index_v2(entries: &[(&str, u32, &[u8])]) -> Vec<u8> {
        let mut b = b"DIRC".to_vec();
        b.extend_from_slice(&2u32.to_be_bytes());
        b.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for (path, mode, content) in entries {
            let start = b.len();
            b.extend_from_slice(&[0u8; 24]);
            b.extend_from_slice(&mode.to_be_bytes());
            b.extend_from_slice(&[0u8; 12]);
            b.extend_from_slice(&object_id(content));
            b.extend_from_slice(&(path.len() as u16).to_be_bytes());
            b.extend_from_slice(path.as_bytes());
            let len = (b.len() - start + 8) & !7;
            b.resize(start + len, 0);
        }
        let sum = sha1(&b);
        b.extend_from_slice(&sum);
        b
    }

    fn be32(b: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn refresh_serves_matching_stat_data_and_clears_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path()).unwrap();
        let a = cas.put(b"hello\n").unwrap();
        let b = cas.put(b"changed\n").unwrap();
        let index = index_v2(&[
            ("a.txt", 0o100644, b"hello\n"),
            ("b.txt", 0o100644, b"original\n"),
            ("gone.txt", 0o100644, b"x"),
            ("link", 0o120000, b"a.txt"),
        ]);
        let ih = cas.put(&index).unwrap();
        let tree = Tree::from_leaves([
            Entry::file(".git/index", ih, 0o644),
            Entry::file("a.txt", a, 0o644),
            Entry::file("b.txt", b, 0o644),
            Entry::symlink("link", "a.txt"),
        ]);
        let (t, r) = refresh_index(&tree, &cas, 1001, 123).unwrap();
        assert_eq!(
            r,
            Refresh::Refreshed(Refreshed {
                entries: 4,
                matched: 2,
                cleared: 2,
                skipped: 0
            })
        );
        let out = cas.get(&t.get(INDEX).unwrap().h.unwrap()).unwrap().unwrap();
        assert_eq!(&out[out.len() - 20..], &sha1(&out[..out.len() - 20]));
        let parsed = parse(&out).unwrap();
        let ino = |p: &str| {
            inode_numbers(&t)
                .into_iter()
                .find(|(e, _)| e.p == p)
                .map(|(_, i)| i as u32)
                .unwrap()
        };
        // a.txt: the served stat data.
        let at = parsed[0].offset;
        let want = served_time(t.get("a.txt").unwrap())
            .duration_since(UNIX_EPOCH)
            .unwrap();
        assert_eq!(be32(&out, at), want.as_secs() as u32);
        assert_eq!(be32(&out, at + 4), want.subsec_nanos());
        assert_eq!(be32(&out, at + 8), want.as_secs() as u32);
        assert_eq!(be32(&out, at + 20), ino("a.txt"));
        assert_eq!((be32(&out, at + 28), be32(&out, at + 32)), (1001, 123));
        assert_eq!(be32(&out, at + 36), 6);
        // b.txt (other content) and gone.txt: cleared.
        for e in &parsed[1..3] {
            assert!(out[e.offset..e.offset + 24].iter().all(|&c| c == 0));
            assert_eq!(be32(&out, e.offset + 36), 0);
        }
        // The symlink: epoch times, its target's length.
        let l = parsed[3].offset;
        assert_eq!(be32(&out, l + 8), 0);
        assert_eq!(be32(&out, l + 20), ino("link"));
        assert_eq!(be32(&out, l + 36), 5);
        // Everything else is unchanged: modes, ids, paths.
        for (x, y) in parse(&index).unwrap().iter().zip(&parsed) {
            assert_eq!((x.mode, x.oid, &x.path), (y.mode, y.oid, &y.path));
        }
        // Idempotent.
        let (again, _) = refresh_index(&t, &cas, 1001, 123).unwrap();
        assert_eq!(again.get(INDEX), t.get(INDEX));
        // The index is served after every file.
        assert!(served_time(t.get(INDEX).unwrap()) > served_time(t.get("a.txt").unwrap()));
    }

    #[test]
    fn an_index_it_does_not_understand_is_served_as_is() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path()).unwrap();
        let mut bad = index_v2(&[("a", 0o100644, b"x")]);
        let n = bad.len();
        bad[n - 1] ^= 1; // not a SHA-1 checksum
        for bytes in [bad, b"DIRC\0\0\0\x05".to_vec(), b"junk".to_vec()] {
            let h = cas.put(&bytes).unwrap();
            let tree = Tree::from_leaves([Entry::file(INDEX, h, 0o644)]);
            let (t, r) = refresh_index(&tree, &cas, 0, 0).unwrap();
            assert!(matches!(r, Refresh::Unchanged(_)), "{r:?}");
            assert_eq!(t.get(INDEX), tree.get(INDEX));
        }
        let tree = Tree::from_leaves([Entry::file("a", cas.put(b"x").unwrap(), 0o644)]);
        assert_eq!(
            refresh_index(&tree, &cas, 0, 0).unwrap().1,
            Refresh::NoIndex
        );
    }

    #[test]
    fn blob_ids_are_memoized() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path()).unwrap();
        let h = cas.put(b"hello\n").unwrap();
        let (id, size) = blob_id(&cas, &h).unwrap();
        assert_eq!((id, size), (object_id(b"hello\n"), 6));
        assert!(memo_path(&cas, &h).is_file());
        assert_eq!(blob_id(&cas, &h).unwrap(), (id, size));
    }
}
