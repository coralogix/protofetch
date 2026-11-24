use std::path::PathBuf;

pub use crate::model::git_oid::GitOid;

/// Branch tips covered by a completed fetch.
#[derive(Debug, Default)]
pub struct FetchResult {
    pub branch_tips: Vec<GitOid>,
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

#[cfg(test)]
mod tests {
    use super::GitOid;

    #[test]
    fn try_from_rejects_a_non_hex_oid() {
        assert!(GitOid::try_from("not-a-commit").is_err());
        assert!(GitOid::try_from("short").is_err());
    }

    #[test]
    fn try_from_accepts_a_40_hex_digit_oid() {
        let oid = GitOid::try_from("1".repeat(40).as_str()).unwrap();

        assert_eq!(oid.as_str(), "1".repeat(40));
    }
}
