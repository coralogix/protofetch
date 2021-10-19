use std::path::{Path, PathBuf};

use git2::{build::RepoBuilder, Cred, FetchOptions, RemoteCallbacks, Repository};
use thiserror::Error;

use crate::{
    model::{Coordinate, Protocol},
    proto_repository::ProtoRepository,
};

pub trait RepositoryCache {
    fn clone_or_update(&self, entry: &Coordinate) -> Result<ProtoRepository, CacheError>;
}

pub struct ProtofetchCache {
    location: PathBuf,
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

impl RepositoryCache for ProtofetchCache {
    fn clone_or_update(&self, entry: &Coordinate) -> Result<ProtoRepository, CacheError> {
        let repo = match self.get_entry(entry) {
            None => self.clone_repo(entry)?,
            Some(path) => {
                let mut repo = self.open_entry(&path)?;

                self.fetch(&entry.protocol, &mut repo)?;

                repo
            }
        };

        Ok(ProtoRepository::new(repo))
    }
}

impl ProtofetchCache {
    pub fn new(location: PathBuf) -> Result<ProtofetchCache, CacheError> {
        if location.exists() && location.is_dir() {
            Ok(ProtofetchCache { location })
        } else if !location.exists() {
            std::fs::create_dir(&location)?;
            Ok(ProtofetchCache { location })
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
        repo_builder
            .bare(true)
            .fetch_options(ProtofetchCache::fetch_options(&entry.protocol));

        repo_builder
            .clone(&entry.url(), &self.location.join(entry.as_path()))
            .map_err(|e| e.into())
    }

    fn fetch(&self, protocol: &Protocol, repo: &mut Repository) -> Result<(), CacheError> {
        let mut remote = repo.find_remote("origin")?;
        let refspecs: Vec<String> = remote
            .refspecs()
            .filter_map(|refspec| refspec.str().map(|s| s.to_string()))
            .collect();

        remote.fetch(
            &refspecs,
            Some(&mut ProtofetchCache::fetch_options(protocol)),
            None,
        )?;

        Ok(())
    }

    fn fetch_options(protocol: &Protocol) -> FetchOptions {
        let mut callbacks = RemoteCallbacks::new();
        if let Protocol::Ssh = protocol {
            callbacks.credentials(|_url, username, _allowed_types| {
                Cred::ssh_key_from_agent(username.unwrap_or("git"))
            });
        };

        let mut fetch_options = FetchOptions::new();
        fetch_options
            .remote_callbacks(callbacks)
            .download_tags(git2::AutotagOption::All);

        fetch_options
    }
}
