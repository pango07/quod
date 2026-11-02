//! The blob store as an interface: what the trust layer and the server need
//! from a content-addressed store, whatever holds the bytes. `Cas` (blobs on
//! local disk) is the implementation every local path uses; quod-server
//! keeps one per team behind the same trait, and an S3-compatible store is
//! a later implementation of it (docs/cloud-ledger.md).
//!
//! Content addressing is the contract: `put_blob` returns the BLAKE3 hash of
//! exactly the bytes stored, and `read_blob` returns exactly the bytes
//! stored under a hash. An implementation never stores bytes under a name
//! it did not compute from them.

use std::io::Read;
use std::path::Path;

use crate::{Cas, CasError, Hash};

pub trait BlobStore {
    /// Whether the store holds `h`.
    fn has_blob(&self, h: &Hash) -> Result<bool, CasError>;
    /// The whole blob; `CasError::Missing` when absent.
    fn read_blob(&self, h: &Hash) -> Result<Vec<u8>, CasError>;
    /// A reader over the blob, for large ones; `CasError::Missing` when absent.
    fn open_blob_reader(&self, h: &Hash) -> Result<Box<dyn Read + Send>, CasError>;
    /// Stores `bytes`, returning their hash.
    fn put_blob(&self, bytes: &[u8]) -> Result<Hash, CasError>;
    /// Stores a local file's bytes (streamed when large, never trusted to
    /// stay put: the hash is of the bytes stored), returning their hash and
    /// the bytes newly written.
    fn put_blob_file(&self, path: &Path) -> Result<(Hash, u64), CasError>;
}

impl BlobStore for Cas {
    fn has_blob(&self, h: &Hash) -> Result<bool, CasError> {
        Ok(self.has(h))
    }

    fn read_blob(&self, h: &Hash) -> Result<Vec<u8>, CasError> {
        let mut buf = Vec::new();
        self.open_blob(h)?
            .read_to_end(&mut buf)
            .map_err(|source| CasError::Io {
                path: self.blob_path(h),
                source,
            })?;
        Ok(buf)
    }

    fn open_blob_reader(&self, h: &Hash) -> Result<Box<dyn Read + Send>, CasError> {
        Ok(Box::new(self.open_blob(h)?))
    }

    fn put_blob(&self, bytes: &[u8]) -> Result<Hash, CasError> {
        self.put(bytes)
    }

    fn put_blob_file(&self, path: &Path) -> Result<(Hash, u64), CasError> {
        self.put_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cas_is_a_blob_store() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let store: &dyn BlobStore = &cas;
        let h = store.put_blob(b"hello").unwrap();
        assert_eq!(h, Hash::of(b"hello"));
        assert!(store.has_blob(&h).unwrap());
        assert_eq!(store.read_blob(&h).unwrap(), b"hello");
        let missing = Hash::of(b"nope");
        assert!(!store.has_blob(&missing).unwrap());
        assert!(matches!(store.read_blob(&missing), Err(CasError::Missing(m)) if m == missing));
        let f = tmp.path().join("file");
        std::fs::write(&f, b"from a file").unwrap();
        let (fh, n) = store.put_blob_file(&f).unwrap();
        assert_eq!((fh, n), (Hash::of(b"from a file"), 11));
        let mut s = String::new();
        store
            .open_blob_reader(&fh)
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, "from a file");
    }
}
