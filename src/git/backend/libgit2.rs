use std::path::{Path, PathBuf};

use git2::{
    cert::Cert, AutotagOption, CertificateCheckStatus, Config, Cred, CredentialType, FetchOptions,
    Oid, RemoteCallbacks, Repository, ResetType, WorktreeAddOptions,
};
use log::{debug, info, trace};
use ssh_key::{known_hosts::HostPatterns, KnownHosts};

use super::{error::GitBackendError, types::GitOid, GitBackend, GitRepository, WorktreeResult};

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

        callbacks.credentials(move |url, username, allowed_types| {
            trace!(
                "Requested credentials for {}, username {:?}, allowed types {:?}",
                url,
                username,
                allowed_types
            );
            if allowed_types.contains(CredentialType::USERNAME) && !tried_username {
                tried_username = true;
                return Cred::username("git");
            }
            if allowed_types.contains(CredentialType::SSH_KEY) && !tried_agent {
                tried_agent = true;
                return Cred::ssh_key_from_agent(username.unwrap_or("git"));
            }
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
            .download_tags(AutotagOption::None);

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
                Ok(url)
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

    fn fetch(&self, remote_name: &str, refspecs: &[String]) -> Result<(), GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let mut remote = repo.find_remote(remote_name)?;
        debug!("Fetching {:?} from {}", refspecs, self.repo_path.display());
        remote.fetch(refspecs, Some(&mut self.fetch_options()), None)?;
        Ok(())
    }

    fn commit_exists(&self, oid: &str) -> Result<bool, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let oid = Oid::from_str(oid).map_err(|e| GitBackendError::InvalidRef(e.to_string()))?;
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

    fn is_ancestor(&self, ancestor: &GitOid, descendant: &GitOid) -> Result<bool, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let a = Oid::from_str(ancestor.as_str())
            .map_err(|e| GitBackendError::InvalidRef(e.to_string()))?;
        let b = Oid::from_str(descendant.as_str())
            .map_err(|e| GitBackendError::InvalidRef(e.to_string()))?;
        let merge_base = repo.merge_base(a, b)?;
        Ok(merge_base == a)
    }

    fn create_worktree(
        &self,
        name: &str,
        worktree_path: &Path,
        commit: &str,
    ) -> Result<WorktreeResult, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;

        match repo.find_worktree(name) {
            Ok(worktree) => {
                let canonical_existing = worktree.path().canonicalize().map_err(|e| {
                    GitBackendError::IO(std::io::Error::new(
                        e.kind(),
                        format!(
                            "Error while canonicalizing path {}: {}",
                            worktree.path().display(),
                            e
                        ),
                    ))
                })?;
                Ok(WorktreeResult::Existing(canonical_existing))
            }
            Err(_) => {
                info!(
                    "Creating new worktree {} at {}.",
                    name,
                    worktree_path.display()
                );

                let reference = repo.reference(
                    &format!("refs/heads/{}", commit),
                    repo.revparse_single(commit)?.id(),
                    true,
                    "",
                )?;

                let mut options = WorktreeAddOptions::new();
                options.reference(Some(&reference));
                repo.worktree(name, worktree_path, Some(&options))?;

                Ok(WorktreeResult::Created(Box::new(Libgit2Repository {
                    repo_path: worktree_path.to_path_buf(),
                    git_config: Config::open_default()?,
                })))
            }
        }
    }

    fn find_worktree(&self, name: &str) -> Result<Option<PathBuf>, GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        match repo.find_worktree(name) {
            Ok(worktree) => Ok(Some(worktree.path().to_path_buf())),
            Err(_) => Ok(None),
        }
    }

    fn reset(&self, commit: &str) -> Result<(), GitBackendError> {
        let repo = Repository::open(&self.repo_path)?;
        let obj = repo.revparse_single(commit)?;
        repo.reset(&obj, ResetType::Hard, None)?;
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
                trace!("No know host entry matched the host key");
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
        HostPatterns::HashedName { .. } => false,
    }
}
