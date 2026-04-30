use std::path::{Path, PathBuf};

use git2::{
    cert::Cert, AutotagOption, CertificateCheckStatus, Config, Cred, CredentialType, FetchOptions,
    RemoteCallbacks, Repository,
};
use log::{debug, info, trace};
use ssh_key::{known_hosts::HostPatterns, KnownHosts};
use thiserror::Error;

use crate::{
    flock::FileLock,
    git::{coord_locks::CoordinateLocks, repository::ProtoGitRepository},
    model::protofetch::{Coordinate, Protocol},
};

const WORKTREES_DIR: &str = "dependencies";
const GLOBAL_KNOWN_HOSTS: &str = "/etc/ssh/ssh_known_hosts";

pub struct ProtofetchGitCache {
    location: PathBuf,
    worktrees: PathBuf,
    // `git2::Config` is neither `Send` nor `Sync`, so we cannot keep one on the
    // cache when the cache is shared across threads. Instead each call to
    // `fetch_options` opens a fresh default config, which only inspects the
    // user/system git config files and is cheap.
    default_protocol: Protocol,
    coord_locks: CoordinateLocks,
    _lock: FileLock,
}

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("Cache location {location} does not exist")]
    BadLocation { location: String },
    #[error("Cache lock cannot be acquired")]
    Lock(#[from] crate::flock::Error),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
}

impl ProtofetchGitCache {
    pub fn new(
        location: PathBuf,
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

        let lock = Self::acquire_lock(&location)?;

        let worktrees = location.join(WORKTREES_DIR);
        Ok(ProtofetchGitCache {
            location,
            worktrees,
            default_protocol,
            coord_locks: CoordinateLocks::default(),
            _lock: lock,
        })
    }

    pub fn coord_locks(&self) -> &CoordinateLocks {
        &self.coord_locks
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

    fn acquire_lock(location: &Path) -> Result<FileLock, CacheError> {
        let location = location.join(".lock");
        debug!(
            "Acquiring a lock on the cache location: {}",
            location.display()
        );
        let lock = FileLock::new(&location)?;
        info!("Acquired a lock on the cache location");
        Ok(lock)
    }

    fn open_entry(&self, path: &Path, url: &str) -> Result<Repository, CacheError> {
        trace!("Opening existing repository at {}", path.display());

        let repo = Repository::open(path)?;

        {
            let remote = repo.find_remote("origin")?;
            if remote.url() != Some(url) {
                // If true then the protocol was updated before updating the cache.
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
        // `git2::Config` is `!Send` and `!Sync`, so we open a fresh per-call
        // copy and let the credentials closure own it for the duration of the
        // fetch.
        let git_config = Config::open_default()?;

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
                return Cred::credential_helper(&git_config, url, username);
            }
            Err(git2::Error::from_str("no valid authentication available"))
        });

        callbacks.certificate_check(|certificate, host| self.check_certificate(certificate, host));

        let mut fetch_options = FetchOptions::new();
        fetch_options
            .remote_callbacks(callbacks)
            .download_tags(AutotagOption::None);

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

// `CoordinateLocks` wraps a `DashMap` whose internal `RwLock`s do not
// implement `UnwindSafe` / `RefUnwindSafe` automatically. The map's value
// type is `Arc<Mutex<()>>` — a dataless mutex — so a panic while a lock is
// held cannot leave invariants broken. We therefore assert these auto-traits
// manually to preserve the auto-trait surface that `ProtofetchGitCache` and
// `Protofetch` exposed before the parallelism rewrite.
impl std::panic::UnwindSafe for ProtofetchGitCache {}
impl std::panic::RefUnwindSafe for ProtofetchGitCache {}

#[allow(dead_code)]
fn _assert_traits() {
    fn assert<T: Send + Sync + std::panic::UnwindSafe + std::panic::RefUnwindSafe>() {}
    assert::<ProtofetchGitCache>();
}
