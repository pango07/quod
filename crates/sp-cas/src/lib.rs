//! Content-addressed blob store on local disk, keyed by BLAKE3, plus the tree
//! data model shared by every layer above it.
//!
//! The store knows blobs and trees. It does not know steps, manifests, FUSE or
//! VMs.

mod canon;
mod hash;
mod store;
mod tree;

pub use canon::{CanonError, canonical_json};
pub use hash::{Hash, HashParseError};
pub use store::remove_any;
pub use store::{Cas, CasError, ImportStats};
pub use tree::{Entry, Kind, ROOT, Tree, child_path, listing_hash, parent_and_name};
