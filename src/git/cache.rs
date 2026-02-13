use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use git2::{
    cert::Cert, AutotagOption, CertificateCheckStatus, Config, Cred, CredentialType, FetchOptions,
    RemoteCallbacks, Repository,
};
use log::{info, trace};
use parking_lot::Mutex;
use ssh_key::{known_hosts::HostPatterns, KnownHosts};
use thiserror::Error;

use crate::{
    git::repository::ProtoGitRepository,
    model::protofetch::{Coordinate, Protocol},
};

const WORKTREES_DIR: &str = "dependencies";
const GLOBAL_KNOWN_HOSTS: &str = "/etc/ssh/ssh_known_hosts";

pub struct ProtofetchGitCache {
    location: PathBuf,
    worktrees: PathBuf,
    /// Cloned per-thread to avoid shared mutable access across threads.
    git_config: Config,
    default_protocol: Protocol,
    /// Per-repository locks keyed by repo path. Allows parallel operations on
    /// different repositories while serializing access to the same one.
    repo_locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

// Safety: ProtofetchGitCache is shared across rayon threads via &self references.
// - repo_locks: parking_lot::Mutex is Send + Sync.
// - git_config: only accessed via clone() before use (each thread gets its own copy).
// - location, worktrees, default_protocol: immutable after construction.
unsafe impl Send for ProtofetchGitCache {}
unsafe impl Sync for ProtofetchGitCache {}

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("Cache location {location} does not exist")]
    BadLocation { location: String },
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
}

impl ProtofetchGitCache {
    pub fn new(
        location: PathBuf,
        git_config: Config,
        default_protocol: Protocol,
    ) -> Result<ProtofetchGitCache, CacheError> {
        if location.exists() {
            if !location.is_dir() {
                return Err(CacheError::BadLocation {
                    location: location.to_str().unwrap_or("").to_string(),
                });
            }
        } else {
            std::fs::create_dir_all(&location)?;
        }

        let worktrees = location.join(WORKTREES_DIR);
        Ok(ProtofetchGitCache {
            location,
            worktrees,
            git_config,
            default_protocol,
            repo_locks: Mutex::new(HashMap::new()),
        })
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        if self.location.exists() {
            info!(
                "Clearing protofetch repository cache {}.",
                &self.location.display()
            );
            std::fs::remove_dir_all(&self.location)?;
        }
        Ok(())
    }

    /// Get the per-repository lock for the given coordinate. Callers must lock
    /// the returned mutex and hold the guard for the entire git operation.
    pub(crate) fn lock_repo(&self, coordinate: &Coordinate) -> Arc<Mutex<()>> {
        let repo_path = {
            let mut p = self.location.clone();
            p.push(coordinate.to_path());
            p
        };
        let mut locks = self.repo_locks.lock();
        locks
            .entry(repo_path)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Open or create a bare git repository for the given coordinate.
    /// Caller must hold the per-repo lock via lock_repo().
    pub(crate) fn open_or_create_repo(
        &self,
        entry: &Coordinate,
    ) -> Result<ProtoGitRepository, CacheError> {
        let mut path = self.location.clone();
        path.push(entry.to_path());

        let url = entry.to_git_url(self.default_protocol);

        let repo = if path.exists() {
            self.open_entry(&path, &url)?
        } else {
            self.create_repo(&path, &url)?
        };

        Ok(ProtoGitRepository::new(self, repo, url))
    }

    pub fn worktrees_path(&self) -> &Path {
        &self.worktrees
    }

    fn open_entry(&self, path: &Path, url: &str) -> Result<Repository, CacheError> {
        trace!("Opening existing repository at {}", path.display());

        let repo = Repository::open(path)?;

        {
            let remote = repo.find_remote("origin")?;
            if remote.url() != Some(url) {
                trace!(
                    "Updating remote existing url {:?} to new url {}",
                    remote.url(),
                    url
                );
                repo.remote_set_url("origin", url)?;
            }
        }

        Ok(repo)
    }

    fn create_repo(&self, path: &Path, url: &str) -> Result<Repository, CacheError> {
        trace!("Creating a new repository at {}", path.display());

        let repo = Repository::init_bare(path)?;
        repo.remote_with_fetch("origin", url, "")?;

        Ok(repo)
    }

    pub(super) fn fetch_options(&self) -> Result<FetchOptions<'_>, CacheError> {
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

        callbacks.certificate_check(|certificate, host| self.check_certificate(certificate, host));

        let mut fetch_options = FetchOptions::new();
        fetch_options
            .remote_callbacks(callbacks)
            .download_tags(AutotagOption::None)
            .depth(1);

        Ok(fetch_options)
    }

    fn check_certificate(
        &self,
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
