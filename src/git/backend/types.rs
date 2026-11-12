use std::{fmt, path::PathBuf};

/// Branch tips covered by a completed fetch.
#[derive(Debug, Default)]
pub struct FetchResult {
    pub branch_tips: Vec<GitOid>,
}

/// Backend-agnostic representation of a git object ID (commit hash).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GitOid {
    hex: String,
}

/// The kind of a leaf entry in a tree listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeEntryKind {
    /// A regular or executable file.
    Blob,
    /// A symbolic link; `target` is the link target as stored in the
    /// repository.
    Symlink { target: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// Path of the entry relative to the repository root.
    pub path: PathBuf,
    pub oid: GitOid,
    pub kind: TreeEntryKind,
}

impl GitOid {
    pub fn from_hex(hex: impl Into<String>) -> Self {
        Self { hex: hex.into() }
    }

    pub fn as_str(&self) -> &str {
        &self.hex
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex)
    }
}
