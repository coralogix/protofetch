use std::path::{Path, PathBuf};

use git2::{
    build::RepoBuilder, Config, Cred, CredentialType, FetchOptions, RemoteCallbacks, Repository,
};
use log::{info, trace};
use thiserror::Error;

use crate::{git::repository::ProtoGitRepository, model::protofetch::Coordinate};

const WORKTREES_DIR: &str = "dependencies";

pub struct ProtofetchGitCache {
    location: PathBuf,
    worktrees: PathBuf,
    git_config: Config,
}

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
    pub fn new(location: PathBuf, git_config: Config) -> Result<ProtofetchGitCache, CacheError> {
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
            Some(path) => self.open_entry(&path)?,
        };

        Ok(ProtoGitRepository::new(self, repo))
    }

    pub fn worktrees_path(&self) -> &Path {
        &self.worktrees
    }

    fn get_entry(&self, entry: &Coordinate) -> Option<PathBuf> {
        let mut full_path = self.location.clone();
        full_path.push(entry.as_path());

        if full_path.exists() {
            Some(full_path)
        } else {
            None
        }
    }

    fn open_entry(&self, path: &Path) -> Result<Repository, CacheError> {
        Repository::open(path).map_err(|e| e.into())
    }

    fn clone_repo(&self, entry: &Coordinate) -> Result<Repository, CacheError> {
        let mut repo_builder = RepoBuilder::new();
        let options = self.fetch_options()?;
        repo_builder.bare(true).fetch_options(options);

        let url = entry.url();
        trace!("Cloning repo {}", url);
        repo_builder
            .clone(&url, self.location.join(entry.as_path()).as_path())
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

        let mut fetch_options = FetchOptions::new();
        fetch_options
            .remote_callbacks(callbacks)
            .download_tags(git2::AutotagOption::All);

        Ok(fetch_options)
    }
}
