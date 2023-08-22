use std::path::{Path, PathBuf};

use git2::Config;
use git2::{build::RepoBuilder, Cred, CredentialType, FetchOptions, RemoteCallbacks, Repository};
use thiserror::Error;
use tracing::{trace};

use crate::{
    cli::HttpGitAuth, model::protofetch::Coordinate, proto_repository::ProtoGitRepository,
};

use crate::proto_repository::ProtoRepository;
#[cfg(test)]
use mockall::{predicate::*, *};

#[cfg_attr(test, automock)]
pub trait RepositoryCache {
    fn clone_or_update(&self, entry: &Coordinate) -> Result<Box<dyn ProtoRepository>, CacheError>;
}

pub struct ProtofetchGitCache {
    pub location: PathBuf,
    git_config: Config,
    git_auth: Option<HttpGitAuth>,
}

impl std::fmt::Debug for ProtofetchGitCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtofetchGitCache")
            .field("location", &self.location)
            .field("git_auth", &self.git_auth)
            .finish()
    }
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

impl RepositoryCache for ProtofetchGitCache {
    fn clone_or_update(&self, entry: &Coordinate) -> Result<Box<dyn ProtoRepository>, CacheError> {
        let repo = match self.get_entry(entry) {
            None => self.clone_repo(entry)?,
            Some(path) => {
                let mut repo = self.open_entry(&path)?;

                self.fetch(&mut repo)?;

                repo
            }
        };

        Ok(Box::new(ProtoGitRepository::new(repo)))
    }
}

impl ProtofetchGitCache {
    pub fn new(
        location: PathBuf,
        git_config: Config,
        git_auth: Option<HttpGitAuth>,
    ) -> Result<ProtofetchGitCache, CacheError> {
        if location.exists() && location.is_dir() {
            Ok(ProtofetchGitCache {
                location,
                git_config,
                git_auth,
            })
        } else if !location.exists() {
            std::fs::create_dir_all(&location)?;
            Ok(ProtofetchGitCache {
                location,
                git_config,
                git_auth,
            })
        } else {
            Err(CacheError::BadLocation {
                location: location.to_str().unwrap_or("").to_string(),
            })
        }
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
        let options = ProtofetchGitCache::fetch_options(&self.git_config, &self.git_auth)?;
        repo_builder.bare(true).fetch_options(options);

        let url = entry.url();
        trace!("Cloning repo {}", url);
        repo_builder
            .clone(&url, self.location.join(entry.as_path()).as_path())
            .map_err(|e| e.into())
    }

    fn fetch(&self, repo: &mut Repository) -> Result<(), CacheError> {
        let mut remote = repo.find_remote("origin")?;
        let refspecs: Vec<String> = remote
            .refspecs()
            .filter_map(|refspec| refspec.str().map(|s| s.to_string()))
            .collect();
        let options = &mut ProtofetchGitCache::fetch_options(&self.git_config, &self.git_auth)?;
        remote.fetch(&refspecs, Some(options), None)?;

        Ok(())
    }

    fn fetch_options<'a>(
        config: &'a Config,
        auth: &'a Option<HttpGitAuth>,
    ) -> Result<FetchOptions<'a>, CacheError> {
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
                if let Some(auth) = auth {
                    return Cred::userpass_plaintext(&auth.username, &auth.password);
                }
                return Cred::credential_helper(config, url, username);
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
