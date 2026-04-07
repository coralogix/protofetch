use std::path::{Path, PathBuf};

use gix::{
    bstr::{BStr, BString},
    progress::Discard,
    remote::Direction,
    Repository,
};
use log::{debug, info, trace};
use thiserror::Error;

use crate::{
    flock::FileLock,
    git::repository::ProtoGitRepository,
    model::protofetch::{Coordinate, Protocol},
};

const WORKTREES_DIR: &str = "dependencies";

pub struct ProtofetchGitCache {
    location: PathBuf,
    worktrees: PathBuf,
    default_protocol: Protocol,
    _lock: FileLock,
}

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Git error: {0}")]
    Git(#[from] Box<dyn std::error::Error + Send + Sync>),
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

        let repo = gix::open(path).map_err(|e| CacheError::Git(Box::new(e)))?;

        // Check and update remote URL if needed
        let origin_name: &BStr = "origin".into();
        if let Ok(remote) = repo.find_remote(origin_name) {
            if let Some(existing_url) = remote.url(Direction::Fetch) {
                let existing_url_str = existing_url.to_bstring().to_string();
                if existing_url_str != url {
                    trace!(
                        "Updating remote existing url {} to new url {}",
                        existing_url_str,
                        url
                    );
                    // Write the new URL to the config file directly
                    let config_path = path.join("config");
                    if config_path.exists() {
                        let config_content = std::fs::read_to_string(&config_path)?;
                        // Simple replacement - in practice this works for most cases
                        let new_content = config_content.replace(&existing_url_str, url);
                        std::fs::write(&config_path, new_content)?;
                    }
                }
            }
        }

        Ok(repo)
    }

    fn create_repo(&self, path: &Path, url: &str) -> Result<Repository, CacheError> {
        trace!("Creating a new repository at {}", path.display());

        std::fs::create_dir_all(path)?;
        let _repo = gix::init_bare(path).map_err(|e| CacheError::Git(Box::new(e)))?;

        // Write remote config directly to the config file
        let config_path = path.join("config");
        let mut config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
        config_content.push_str(&format!(
            "\n[remote \"origin\"]\n\turl = {}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n",
            url
        ));
        std::fs::write(&config_path, config_content)?;

        // Re-open to pick up the new config
        let repo = gix::open(path).map_err(|e| CacheError::Git(Box::new(e)))?;

        Ok(repo)
    }

    pub(super) fn fetch_repo(
        &self,
        repo: &Repository,
        refspecs: &[String],
    ) -> Result<(), CacheError> {
        let origin_name: &BStr = "origin".into();
        let remote = repo
            .find_remote(origin_name)
            .map_err(|e| CacheError::Git(Box::new(e)))?;

        debug!("Fetching {:?} from remote", refspecs);

        // Convert refspecs to gix format
        let extra_refspecs: Vec<gix::refspec::RefSpec> = refspecs
            .iter()
            .filter_map(|s| {
                gix::refspec::parse(s.as_str().into(), gix::refspec::parse::Operation::Fetch)
                    .ok()
                    .map(|r| r.to_owned())
            })
            .collect();

        // Connect and fetch
        let mut connection = remote
            .connect(Direction::Fetch)
            .map_err(|e| CacheError::Git(Box::new(e)))?;
        // if let Some(url) = remote.url(Direction::Fetch) {
        //     connection.set_credentials(gix_credentials::builtin);
        //     let get_creds = connection
        //         .configured_credentials(url.clone())
        //         .map_err(|e| CacheError::Git(Box::new(e)))?;
        //     info!("remote {:?}", connection.remote());
        // }

        let fetch = connection
            .with_credentials(gix_credentials::builtin)
            .prepare_fetch(
                Discard,
                gix::remote::ref_map::Options {
                    extra_refspecs,
                    ..Default::default()
                },
            )
            .map_err(|e| CacheError::Git(Box::new(e)))?;

        fetch
            .receive(Discard, &gix::interrupt::IS_INTERRUPTED)
            .map_err(|e| CacheError::Git(Box::new(e)))?;

        Ok(())
    }
}
