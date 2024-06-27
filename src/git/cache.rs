use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use git2::{
    build::RepoBuilder, cert::Cert, CertificateCheckStatus, Config, Cred, CredentialType,
    FetchOptions, RemoteCallbacks, Repository,
};
use gix_lock::Marker;
use log::{debug, info, trace};
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
    git_config: Config,
    default_protocol: Protocol,
    _lock: Marker,
}

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("Cache location {location} does not exist")]
    BadLocation { location: String },
    #[error("Cache lock cannot be acquired")]
    Lock(#[from] gix_lock::acquire::Error),
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

        gix_lock::tempfile::signal::setup(Default::default());
        let lock = Self::acquire_lock(&location)?;

        let worktrees = location.join(WORKTREES_DIR);
        Ok(ProtofetchGitCache {
            location,
            worktrees,
            git_config,
            default_protocol,
            _lock: lock,
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

    pub fn repository(&self, entry: &Coordinate) -> Result<ProtoGitRepository, CacheError> {
        let repo = match self.get_entry(entry) {
            None => self.clone_repo(entry)?,
            Some(path) => self.open_entry(&path, entry)?,
        };

        Ok(ProtoGitRepository::new(self, repo))
    }

    pub fn worktrees_path(&self) -> &Path {
        &self.worktrees
    }

    fn acquire_lock(location: &Path) -> Result<Marker, CacheError> {
        use gix_lock::acquire::Fail;
        debug!(
            "Acquiring a lock on the cache location: {}",
            location.display()
        );
        let start = Instant::now();
        loop {
            match Marker::acquire_to_hold_resource(location, Fail::Immediately, None) {
                Ok(lock) => {
                    info!("Acquired a lock on the cache location");
                    return Ok(lock);
                }
                Err(error) => {
                    if start.elapsed() < Duration::from_secs(300) {
                        debug!("Failed to acquire a lock on the cache location, retrying");
                        std::thread::sleep(Duration::from_secs(1));
                    } else {
                        return Err(error.into());
                    }
                }
            }
        }
    }

    fn get_entry(&self, entry: &Coordinate) -> Option<PathBuf> {
        let mut full_path = self.location.clone();
        full_path.push(entry.to_path());

        if full_path.exists() {
            Some(full_path)
        } else {
            None
        }
    }

    fn open_entry(&self, path: &Path, entry: &Coordinate) -> Result<Repository, CacheError> {
        let repo = Repository::open(path).map_err(CacheError::from)?;

        {
            let remote = repo.find_remote("origin").map_err(CacheError::from)?;

            if let (Some(url), Some(protocol)) = (remote.url(), entry.protocol) {
                let new_url = entry.to_git_url(protocol);

                if url != new_url {
                    // If true then the protocol was updated before updating the cache.
                    trace!(
                        "Updating remote existing url {} to new url {}",
                        url,
                        new_url
                    );
                    repo.remote_set_url("origin", &new_url)?;
                }
            }
        } // `remote` reference is dropped here so that we can return `repo`

        Ok(repo)
    }

    fn clone_repo(&self, entry: &Coordinate) -> Result<Repository, CacheError> {
        let mut repo_builder = RepoBuilder::new();
        let options = self.fetch_options()?;
        repo_builder.bare(true).fetch_options(options);

        let url = entry.to_git_url(self.default_protocol);
        trace!("Cloning repo {}", url);
        repo_builder
            .clone(&url, self.location.join(entry.to_path()).as_path())
            .map_err(|e| e.into())
    }

    pub(super) fn fetch_options(&self) -> Result<FetchOptions<'_>, CacheError> {
        let mut callbacks = RemoteCallbacks::new();
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
            if allowed_types.contains(CredentialType::USERNAME) {
                return Cred::username("git");
            }
            // SSH auth
            if allowed_types.contains(CredentialType::SSH_KEY) {
                return Cred::ssh_key_from_agent(username.unwrap_or("git"));
            }
            // HTTP auth
            if allowed_types.contains(CredentialType::USER_PASS_PLAINTEXT) {
                return Cred::credential_helper(&self.git_config, url, username);
            }
            Err(git2::Error::from_str("no valid authentication available"))
        });

        callbacks.certificate_check(|certificate, host| self.check_certificate(certificate, host));

        let mut fetch_options = FetchOptions::new();
        fetch_options
            .remote_callbacks(callbacks)
            .download_tags(git2::AutotagOption::All);

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
