use std::path::{Path, PathBuf};

use git2::{build::RepoBuilder, FetchOptions, Repository};
use thiserror::Error;

use crate::model::Coordinate;

pub struct ProtofetchCache {
    location: PathBuf,
}

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("Cache location {location} does not exist")]
    BadLocation { location: String },
}

impl ProtofetchCache {
    pub fn new(location: PathBuf) -> Result<ProtofetchCache, CacheError> {
        if location.exists() && location.is_dir() {
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
        RepoBuilder::new()
            .bare(true)
            .clone(&entry.url(), &self.location.join(entry.as_path()))
            .map_err(|e| e.into())
    }

    fn fetch(&self, repo: &mut Repository) -> Result<(), CacheError> {
        let mut fetch_options = FetchOptions::new();

        fetch_options.download_tags(git2::AutotagOption::All);

        let mut remote = repo.find_remote("origin")?;
        let refspecs: Vec<String> = remote
            .refspecs()
            .filter_map(|refspec| refspec.str().map(|s| s.to_string()))
            .collect();

        remote.fetch(&refspecs, Some(&mut fetch_options), None)?;

        Ok(())
    }

    pub fn clone_or_fetch(&self, entry: &Coordinate) -> Result<Repository, CacheError> {
        match self.get_entry(entry) {
            None => self.clone_repo(entry),
            Some(path) => {
                let mut repo = self.open_entry(&path)?;

                self.fetch(&mut repo)?;

                Ok(repo)
            }
        }
    }
}
