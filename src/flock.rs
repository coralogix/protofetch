use std::{
    fs::{File, OpenOptions},
    path::Path,
};

use fs4::fs_std::FileExt;
use log::info;
use thiserror::Error;

/// An exclusive advisory file lock, released on drop.
pub struct FileLock {
    _file: File,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub struct Error(#[from] std::io::Error);

impl FileLock {
    /// Waits as long as another run holds the lock, like Cargo's package cache lock.
    pub fn new(path: &Path) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        if !FileExt::try_lock_exclusive(&file)? {
            info!("Waiting for a lock on {}", path.display());
            FileExt::lock_exclusive(&file)?;
        }
        Ok(Self { _file: file })
    }
}
