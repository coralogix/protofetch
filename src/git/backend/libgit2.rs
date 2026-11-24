use std::path::{Path, PathBuf};

use git2::{
    cert::Cert, AutotagOption, CertificateCheckStatus, Config, Cred, CredentialType, FetchOptions,
    Oid, RemoteCallbacks, Repository,
};
use log::{debug, trace};
use ssh_key::{known_hosts::HostPatterns, KnownHosts};

use super::{
    error::GitBackendError,
    types::{FetchResult, GitOid, TreeEntry, TreeEntryKind},
    GitBackend, GitRepository, UNSHALLOW_DEPTH,
};

const GLOBAL_KNOWN_HOSTS: &str = "/etc/ssh/ssh_known_hosts";

pub struct Libgit2Backend;

impl Libgit2Backend {
    pub fn new() -> Self {
        Self
    }
}

pub struct Libgit2Repository {
    repo_path: PathBuf,
    git_config: Config,
}

impl Libgit2Repository {
    fn fetch_options(&self) -> FetchOptions<'_> {
        let mut callbacks = RemoteCallbacks::new();

        let mut tried_username = false;
        let mut tried_agent = false;
        let mut tried_helper = false;

        // Consider using https://crates.io/crates/git2_credentials that supports
        // more authentication options
        callbacks.credentials(move |url, username, allowed_types| {
            trace!(
                "Requested credentials for {}, username {:?}, allowed types {:?}",
                url,
                username,
                allowed_types
            );
            // Asking for ssh username
            if allowed_types.contains(CredentialType::USERNAME) && !tried_username {
                tried_username = true;
                return Cred::username("git");
            }
            // SSH auth
            if allowed_types.contains(CredentialType::SSH_KEY) && !tried_agent {
                tried_agent = true;
                return Cred::ssh_key_from_agent(username.unwrap_or("git"));
            }
            // HTTP auth
            if allowed_types.contains(CredentialType::USER_PASS_PLAINTEXT) && !tried_helper {
                tried_helper = true;
                return Cred::credential_helper(&self.git_config, url, username);
            }
            Err(git2::Error::from_str("no valid authentication available"))
        });

        callbacks.certificate_check(check_certificate);

        let mut fetch_options = FetchOptions::new();
        fetch_options
            .remote_callbacks(callbacks)
            .download_tags(AutotagOption::None)
            .prune(git2::FetchPrune::On);

        fetch_options
    }
}

impl GitRepository for Libgit2Repository {
    fn remote_add(&self, name: &str, url: &str) -> Result<(), GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        repo.remote_with_fetch(name, url, "")?;
        Ok(())
    }

    fn remote_get_url(&self, name: &str) -> Result<Option<String>, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let result = repo.find_remote(name);
        match result {
            Ok(remote) => {
                let url = remote.url().map(|s| s.to_string());
                Ok(url.into_option())
            }
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn remote_set_url(&self, name: &str, url: &str) -> Result<(), GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        repo.remote_set_url(name, url)?;
        Ok(())
    }

    fn fetch(
        &self,
        remote_name: &str,
        refspecs: &[String],
    ) -> Result<FetchResult, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let mut remote = repo.find_remote(remote_name)?;
        debug!("Fetching {:?} from {}", refspecs, self.repo_path.display());
        let mut options = self.fetch_options();
        if repo.is_shallow() {
            options.depth(UNSHALLOW_DEPTH);
        }
        remote.fetch(refspecs, Some(&mut options), None)?;
        // A successful libgit2 fetch does not imply that explicit refs exist.
        // Use the advertisement from this fetch, not refs left in the cache.
        let advertised = remote.list()?;
        for refspec in refspecs {
            let refspec = refspec.strip_prefix('+').unwrap_or(refspec);
            let Some((source, destination)) = refspec.split_once(':') else {
                continue;
            };
            if source.contains('*') || advertised.iter().any(|head| head.name() == source) {
                continue;
            }
            if source.starts_with("refs/tags/") {
                // Tags are optional because revision can also be a hash.
                match repo.find_reference(destination) {
                    Ok(mut reference) => reference.delete()?,
                    Err(error) if error.code() == git2::ErrorCode::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            } else {
                return Err(GitBackendError::NotFound(format!(
                    "Remote ref {source} was not found"
                )));
            }
        }
        let branch_tips = advertised
            .iter()
            .filter(|head| {
                head.name().starts_with("refs/heads/")
                    && refspecs.iter().any(|refspec| {
                        let source = refspec
                            .trim_start_matches('+')
                            .split(':')
                            .next()
                            .unwrap_or("");
                        source == head.name() || source == "refs/heads/*"
                    })
            })
            .map(|head| GitOid::from(head.oid()))
            .collect();
        Ok(FetchResult { branch_tips })
    }

    fn fetch_shallow(&self, remote_name: &str, commit: &GitOid) -> Result<(), GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let mut remote = repo.find_remote(remote_name)?;
        debug!(
            "Fetching {commit} without history into {}",
            self.repo_path.display()
        );
        let mut options = self.fetch_options();
        options.depth(1);
        remote.fetch(&[commit.as_str()], Some(&mut options), None)?;
        Ok(())
    }

    fn commit_exists(&self, oid: &GitOid) -> Result<bool, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let oid =
            Oid::from_str(oid.as_str()).map_err(|e| GitBackendError::InvalidRef(e.to_string()))?;
        let exists = repo.find_commit(oid).is_ok();
        Ok(exists)
    }

    fn revparse_commit(&self, spec: &str) -> Result<GitOid, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let obj = repo.revparse_single(spec)?;
        let commit = obj.peel_to_commit()?;
        Ok(GitOid::from(commit.id()))
    }

    fn read_blob(&self, commit: &str, blob_path: &str) -> Result<Option<Vec<u8>>, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let spec = format!("{commit}:{blob_path}");
        let result = repo.revparse_single(&spec);
        match result {
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e.into()),
            Ok(obj) => match obj.kind() {
                Some(git2::ObjectType::Blob) => {
                    let blob = obj.peel_to_blob()?;
                    let content = blob.content().to_vec();
                    Ok(Some(content))
                }
                Some(kind) => Err(GitBackendError::GitError(format!(
                    "Bad git object kind {} found for {} (expected blob)",
                    kind, spec
                ))),
                None => Err(GitBackendError::NotFound(format!(
                    "Missing object for {}",
                    spec
                ))),
            },
        }
    }

    fn list_tree(&self, commit: &str) -> Result<Vec<TreeEntry>, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let oid = Oid::from_str(commit).map_err(|e| GitBackendError::InvalidRef(e.to_string()))?;
        let tree = repo.find_commit(oid)?.tree()?;
        let mut entries = Vec::new();
        walk_tree(&repo, &tree, Path::new(""), &mut entries)?;
        Ok(entries)
    }

    fn read_blobs(&self, oids: &[GitOid]) -> Result<Vec<Vec<u8>>, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        oids.iter()
            .map(|oid| {
                let oid = Oid::from_str(oid.as_str())
                    .map_err(|e| GitBackendError::InvalidRef(e.to_string()))?;
                Ok(repo.find_blob(oid)?.content().to_vec())
            })
            .collect()
    }

    fn is_ancestor(&self, ancestor: &GitOid, descendant: &GitOid) -> Result<bool, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let a = Oid::from_str(ancestor.as_str())
            .map_err(|e| GitBackendError::InvalidRef(e.to_string()))?;
        let b = Oid::from_str(descendant.as_str())
            .map_err(|e| GitBackendError::InvalidRef(e.to_string()))?;
        match repo.merge_base(a, b) {
            Ok(merge_base) => Ok(merge_base == a),
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn update_ref(&self, name: &str, commit: &str) -> Result<(), GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let oid = Oid::from_str(commit).map_err(|e| GitBackendError::InvalidRef(e.to_string()))?;
        repo.reference(name, oid, true, "protofetch: pin locked commit")?;
        Ok(())
    }
}

impl GitBackend for Libgit2Backend {
    fn init_bare(&self, path: &Path) -> Result<Box<dyn GitRepository>, GitBackendError> {
        trace!("Creating a new bare repository at {}", path.display());
        Repository::init_bare(path)?;
        Ok(Box::new(Libgit2Repository {
            repo_path: path.to_path_buf(),
            git_config: Config::open_default()?,
        }))
    }

    fn open(&self, path: &Path) -> Result<Box<dyn GitRepository>, GitBackendError> {
        trace!("Opening existing repository at {}", path.display());
        Repository::open_bare(path)?;
        Ok(Box::new(Libgit2Repository {
            repo_path: path.to_path_buf(),
            git_config: Config::open_default()?,
        }))
    }
}

fn check_certificate(
    certificate: &Cert<'_>,
    host: &str,
) -> Result<CertificateCheckStatus, git2::Error> {
    if let Some(hostkey) = certificate.as_hostkey().and_then(|h| h.hostkey()) {
        trace!("Loading {}", GLOBAL_KNOWN_HOSTS);
        match KnownHosts::read_file(GLOBAL_KNOWN_HOSTS) {
            Ok(entries) => {
                for entry in entries {
                    if host_matches_patterns(host, entry.host_patterns()) {
                        trace!(
                            "Found known host entry for {} ({})",
                            host,
                            entry.public_key().algorithm()
                        );
                        if entry.public_key().to_bytes().as_deref() == Ok(hostkey) {
                            trace!("Known host entry matches the host key");
                            return Ok(CertificateCheckStatus::CertificateOk);
                        }
                    }
                }
                trace!("No known host entry matched the host key");
            }
            Err(error) => trace!("Could not load {}: {}", GLOBAL_KNOWN_HOSTS, error),
        }
    }
    Ok(CertificateCheckStatus::CertificatePassthrough)
}

fn host_matches_patterns(host: &str, patterns: &HostPatterns) -> bool {
    match patterns {
        HostPatterns::Patterns(patterns) => {
            let mut match_found = false;
            for pattern in patterns {
                let pattern = pattern.to_lowercase();
                // * and ? wildcards are not yet supported
                if let Some(pattern) = pattern.strip_prefix('!') {
                    if pattern == host {
                        return false;
                    }
                } else {
                    match_found |= pattern == host;
                }
            }
            match_found
        }
        // Not yet supported
        HostPatterns::HashedName { .. } => false,
    }
}

// A recursive walk instead of `Tree::walk`: the callback API reports the
// root prefix as a `&str` and aborts when a directory name is not valid
// UTF-8, while this walk skips those entries like the CLI backend.
fn walk_tree(
    repo: &Repository,
    tree: &git2::Tree,
    prefix: &Path,
    entries: &mut Vec<TreeEntry>,
) -> Result<(), GitBackendError> {
    for entry in tree {
        // Non-UTF-8 names are skipped, matching the CLI backend; a
        // non-UTF-8 directory is therefore descended into by neither.
        let Ok(name) = std::str::from_utf8(entry.name_bytes()) else {
            debug!("Skipping non-UTF-8 tree entry {}", entry.id());
            continue;
        };
        let path = prefix.join(name);
        match entry.kind() {
            Some(git2::ObjectType::Tree) => {
                walk_tree(repo, &repo.find_tree(entry.id())?, &path, entries)?;
            }
            Some(git2::ObjectType::Blob) => {
                let oid = GitOid::from(entry.id());
                let kind = if entry.filemode() == 0o120000 {
                    // The link target is the blob's content; links whose
                    // target is not valid UTF-8 are skipped.
                    let blob = repo.find_blob(entry.id())?;
                    let Ok(target) = std::str::from_utf8(blob.content()) else {
                        debug!("Skipping symlink with non-UTF-8 target {}", path.display());
                        continue;
                    };
                    TreeEntryKind::Symlink {
                        target: PathBuf::from(target),
                    }
                } else {
                    TreeEntryKind::Blob
                };
                entries.push(TreeEntry { path, oid, kind });
            }
            // Submodules and entries without a kind are skipped.
            _ => {}
        }
    }
    Ok(())
}

impl From<git2::Oid> for GitOid {
    fn from(oid: git2::Oid) -> Self {
        Self::from_git(oid.to_string())
    }
}

impl From<git2::Error> for GitBackendError {
    fn from(e: git2::Error) -> Self {
        match e.code() {
            git2::ErrorCode::NotFound => GitBackendError::NotFound(e.message().to_string()),
            git2::ErrorCode::Auth => GitBackendError::AuthError(e.message().to_string()),
            _ => GitBackendError::GitError(e.message().to_string()),
        }
    }
}

// Starting from git2 0.21, some methods return Result<T, git2::Error> that previously used to return Option<T>.
// This is a compatibility layer to be able to keep a broad git2 dependency range.
trait Git2ResultCompat<T> {
    fn into_option(self) -> Option<T>;
}

impl<T> Git2ResultCompat<T> for Result<T, git2::Error> {
    fn into_option(self) -> Option<T> {
        self.ok()
    }
}

impl<T> Git2ResultCompat<T> for Option<T> {
    fn into_option(self) -> Option<T> {
        self
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    #[cfg(unix)]
    use crate::git::backend::test_support::non_utf8_repo;
    use crate::git::backend::test_support::{
        assert_fixture_listing, assert_fixture_reads, fixture_repo,
    };

    #[test]
    fn list_tree_lists_blobs_and_symlinks() {
        let (tempdir, commit) = fixture_repo();
        let repo = Libgit2Backend::new().open(tempdir.path()).unwrap();

        assert_fixture_listing(repo.as_ref(), &commit);
    }

    #[test]
    fn read_blobs_reads_exact_bytes_in_order() {
        let (tempdir, commit) = fixture_repo();
        let repo = Libgit2Backend::new().open(tempdir.path()).unwrap();

        assert_fixture_reads(repo.as_ref(), &commit);
    }

    #[cfg(unix)]
    #[test]
    fn list_tree_skips_non_utf8_names_and_targets() {
        let (bare, commit) = non_utf8_repo();
        let repo = Libgit2Backend::new().open(bare.path()).unwrap();

        let entries = repo.list_tree(&commit).unwrap();

        let paths: Vec<_> = entries.iter().map(|entry| &entry.path).collect();
        assert_eq!(paths, [Path::new("good.proto")]);
    }
}
