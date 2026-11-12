pub mod error;
pub mod types;

pub mod libgit2;

#[cfg(feature = "git-backend-cli")]
pub mod cli;

use std::{
    panic::{RefUnwindSafe, UnwindSafe},
    path::Path,
    str::FromStr,
};

use anyhow::bail;
use log::info;
use serde::Deserialize;

use error::GitBackendError;
use types::{FetchResult, GitOid, TreeEntry};

/// Fetch depth that deepens a shallow repository to its full history, the value git and
/// libgit2 use for "unshallow". The CLI backend passes it as `--depth` because
/// `git fetch --unshallow` fails with "shallow file has changed since we read it" when a
/// repository fetched from GitHub has more than one shallow boundary.
pub const UNSHALLOW_DEPTH: i32 = i32::MAX;

/// Per-repository operations. Obtained from a [`GitBackend`].
/// The implementing type stores the repository path internally.
pub trait GitRepository {
    fn remote_add(&self, name: &str, url: &str) -> Result<(), GitBackendError>;
    fn remote_get_url(&self, name: &str) -> Result<Option<String>, GitBackendError>;
    fn remote_set_url(&self, name: &str, url: &str) -> Result<(), GitBackendError>;
    /// Fetches refspecs with full history, deepening the repository if it is shallow.
    fn fetch(&self, remote_name: &str, refspecs: &[String])
        -> Result<FetchResult, GitBackendError>;
    /// Fetches a single commit without its history.
    fn fetch_shallow(&self, remote_name: &str, commit: &GitOid) -> Result<(), GitBackendError>;
    fn commit_exists(&self, oid: &GitOid) -> Result<bool, GitBackendError>;
    fn revparse_commit(&self, spec: &str) -> Result<GitOid, GitBackendError>;
    fn read_blob(&self, commit: &str, blob_path: &str) -> Result<Option<Vec<u8>>, GitBackendError>;
    /// Check if `ancestor` is an ancestor of `descendant`.
    fn is_ancestor(&self, ancestor: &GitOid, descendant: &GitOid) -> Result<bool, GitBackendError>;
    /// Lists the leaf entries of the tree at `commit`, following subtrees but
    /// not symlinks. Symlink entries carry their recorded target; submodule
    /// entries are skipped. Entries whose paths or symlink targets are not
    /// valid UTF-8 are skipped.
    ///
    /// # Errors
    /// Returns an error if the commit cannot be resolved or the tree cannot
    /// be read.
    fn list_tree(&self, commit: &str) -> Result<Vec<TreeEntry>, GitBackendError>;
    /// Reads the blobs `oids`, returning their contents in the same order.
    ///
    /// # Errors
    /// Returns an error if any blob is missing or cannot be read.
    fn read_blobs(&self, oids: &[GitOid]) -> Result<Vec<Vec<u8>>, GitBackendError>;
    /// Point the reference `name` at `commit`, creating or moving it.
    fn update_ref(&self, name: &str, commit: &str) -> Result<(), GitBackendError>;
}

/// Factory for opening or creating git repositories.
pub trait GitBackend: Send + Sync + UnwindSafe + RefUnwindSafe {
    /// Initialize a new bare repository at the given path and return a handle to it.
    fn init_bare(&self, path: &Path) -> Result<Box<dyn GitRepository>, GitBackendError>;

    /// Open an existing repository at the given path and return a handle to it.
    /// Returns an error if no repository exists there.
    fn open(&self, path: &Path) -> Result<Box<dyn GitRepository>, GitBackendError>;
}

/// The type of git backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
pub enum GitBackendType {
    #[default]
    #[serde(rename = "libgit2")]
    Libgit2,
    #[cfg(feature = "git-backend-cli")]
    #[serde(rename = "cli")]
    Cli,
}

impl FromStr for GitBackendType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "libgit2" => Ok(GitBackendType::Libgit2),
            #[cfg(feature = "git-backend-cli")]
            "cli" => Ok(GitBackendType::Cli),
            _ => bail!("invalid git backend type: {s}"),
        }
    }
}

/// Create a git backend of the specified type.
pub fn create_backend(
    backend_type: GitBackendType,
    git_executable: Option<String>,
) -> Box<dyn GitBackend> {
    match backend_type {
        GitBackendType::Libgit2 => {
            info!("Using libgit2 git backend");
            Box::new(libgit2::Libgit2Backend::new())
        }
        #[cfg(feature = "git-backend-cli")]
        GitBackendType::Cli => {
            info!("Using git CLI backend");
            Box::new(cli::CliBackend::new(
                git_executable.unwrap_or_else(|| "git".to_string()),
            ))
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    use git2::{Repository, Signature};

    use super::types::{GitOid, TreeEntryKind};
    use super::GitRepository;

    /// A bare repository whose root tree holds nested blobs (`proto/one.proto`,
    /// `proto/sub/two.proto`, `README`) and a symlink to a .proto file
    /// (`alias.proto` -> `proto/one.proto`).
    /// Returns the directory and the commit hash of that tree.
    pub(crate) fn fixture_repo() -> (tempfile::TempDir, String) {
        let tempdir = tempfile::tempdir().unwrap();
        let repo = Repository::init_bare(tempdir.path()).unwrap();
        let one = repo
            .blob(b"syntax = \"proto3\";\r\n// exact bytes\n")
            .unwrap();
        let two = repo.blob(b"// two\n").unwrap();
        let readme = repo.blob(b"not a proto").unwrap();
        let alias = repo.blob(b"proto/one.proto").unwrap();

        let mut sub = repo.treebuilder(None).unwrap();
        sub.insert("two.proto", two, 0o100644).unwrap();
        let sub = repo.find_tree(sub.write().unwrap()).unwrap();

        let mut proto = repo.treebuilder(None).unwrap();
        proto.insert("one.proto", one, 0o100644).unwrap();
        proto.insert("sub", sub.id(), 0o040000).unwrap();
        let proto = repo.find_tree(proto.write().unwrap()).unwrap();

        let signature = Signature::now("test", "test@example.com").unwrap();
        let mut root = repo.treebuilder(None).unwrap();
        root.insert("proto", proto.id(), 0o040000).unwrap();
        root.insert("README", readme, 0o100644).unwrap();
        root.insert("alias.proto", alias, 0o120000).unwrap();
        let tree = repo.find_tree(root.write().unwrap()).unwrap();
        let commit = repo
            .commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
            .unwrap();
        (tempdir, commit.to_string())
    }

    /// Asserts the listing of [`fixture_repo`] so every backend is held to the
    /// same result.
    pub(crate) fn assert_fixture_listing(repo: &dyn GitRepository, commit: &str) {
        let mut entries = repo.list_tree(commit).unwrap();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let listed: Vec<(PathBuf, TreeEntryKind)> = entries
            .into_iter()
            .map(|entry| (entry.path, entry.kind))
            .collect();
        assert_eq!(
            listed,
            vec![
                (PathBuf::from("README"), TreeEntryKind::Blob),
                (
                    PathBuf::from("alias.proto"),
                    TreeEntryKind::Symlink {
                        target: PathBuf::from("proto/one.proto")
                    }
                ),
                (PathBuf::from("proto/one.proto"), TreeEntryKind::Blob),
                (PathBuf::from("proto/sub/two.proto"), TreeEntryKind::Blob),
            ]
        );
    }

    /// Asserts that `read_blobs` returns the exact bytes of [`fixture_repo`]
    /// blobs in request order, and fails for a missing object.
    pub(crate) fn assert_fixture_reads(repo: &dyn GitRepository, commit: &str) {
        let entries = repo.list_tree(commit).unwrap();
        let oid = |path: &str| {
            entries
                .iter()
                .find(|entry| entry.path == Path::new(path))
                .unwrap()
                .oid
                .clone()
        };
        let (one, two) = (oid("proto/one.proto"), oid("proto/sub/two.proto"));

        let contents = repo.read_blobs(&[two, one.clone(), one]).unwrap();

        let one: &[u8] = b"syntax = \"proto3\";\r\n// exact bytes\n";
        assert_eq!(contents, [b"// two\n".as_slice(), one, one]);
        assert!(repo
            .read_blobs(&[GitOid::from_git("1".repeat(40))])
            .is_err());
    }

    /// A bare repository whose tree holds a non-UTF-8 directory, file, and
    /// symlink target, built with git plumbing commands so the raw-byte
    /// names exist only in the tree: macOS rejects them as file names.
    /// Returns the directory and the commit hash.
    #[cfg(unix)]
    pub(crate) fn non_utf8_repo() -> (tempfile::TempDir, String) {
        fn git_stdin(git_dir: &Path, args: &[&str], input: &[u8]) -> String {
            use std::io::Write as _;
            let mut child = std::process::Command::new("git")
                .arg("--git-dir")
                .arg(git_dir)
                .args(args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(input).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }

        let bare = tempfile::tempdir().unwrap();
        git_stdin(bare.path(), &["init", "--bare"], b"");
        let good = git_stdin(bare.path(), &["hash-object", "-w", "--stdin"], b"// good");
        let bad = git_stdin(bare.path(), &["hash-object", "-w", "--stdin"], b"// bad");
        let link_target = git_stdin(
            bare.path(),
            &["hash-object", "-w", "--stdin"],
            b"bad\xffdir/b.proto",
        );
        let sub = git_stdin(
            bare.path(),
            &["mktree", "-z"],
            format!("100644 blob {bad}\tb.proto\0").as_bytes(),
        );
        let mut root_input = Vec::new();
        root_input.extend_from_slice(format!("100644 blob {good}\tgood.proto\0").as_bytes());
        root_input.extend_from_slice(b"040000 tree ");
        root_input.extend_from_slice(sub.as_bytes());
        root_input.extend_from_slice(b"\tbad");
        root_input.push(0xff);
        root_input.extend_from_slice(b"dir\0");
        root_input.extend_from_slice(format!("100644 blob {bad}\tbad").as_bytes());
        root_input.push(0xff);
        root_input.extend_from_slice(b".proto\0");
        root_input.extend_from_slice(b"120000 blob ");
        root_input.extend_from_slice(link_target.as_bytes());
        root_input.extend_from_slice(b"\tlink.proto\0");
        let root = git_stdin(bare.path(), &["mktree", "-z"], &root_input);
        let commit = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Protofetch Test",
                "-c",
                "user.email=protofetch@example.com",
            ])
            .arg("--git-dir")
            .arg(bare.path())
            .args(["commit-tree", &root, "-m", "non-utf8"])
            .output()
            .unwrap();
        assert!(commit.status.success());
        (
            bare,
            String::from_utf8(commit.stdout).unwrap().trim().to_owned(),
        )
    }
}
