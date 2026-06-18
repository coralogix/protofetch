pub mod model;

mod compat;
mod fetch;
mod resolve;

use std::str::Utf8Error;

use thiserror::Error;

pub use fetch::fetch;
pub use resolve::resolve;

/// Tunables for the parallel resolver / fetcher.
#[derive(Debug, Clone, Copy)]
pub struct ParallelConfig {
    /// Maximum number of in-flight network operations (resolve / fetch).
    pub network_jobs: usize,
    /// Maximum number of in-flight disk-bound operations (worktree + copy).
    pub copy_jobs: usize,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self {
            network_jobs: 16,
            copy_jobs: (cpus / 2).max(4),
        }
    }
}

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Error while fetching repo from cache: {0}")]
    Cache(anyhow::Error),
    #[error("Git error: {0}")]
    GitError(#[from] git2::Error),
    #[error("Error while decoding utf8 bytes from blob: {0}")]
    BlobRead(#[from] Utf8Error),
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
    #[error("Error while processing protobuf repository: {0}")]
    ProtoRepoError(#[from] crate::git::repository::ProtoRepoError),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error(transparent)]
    Resolver(anyhow::Error),
}
