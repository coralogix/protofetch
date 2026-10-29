use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

use fs4::fs_std::FileExt;
use log::info;
use thiserror::Error;

/// An advisory file lock, released on drop.
///
/// Locks belong to an open file handle, so they also exclude other threads of
/// the same process that lock the same path.
pub struct FileLock {
    _file: File,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub struct Error(#[from] io::Error);

impl FileLock {
    pub fn shared(path: &Path) -> Result<Self, Error> {
        Self::acquire(path, FileExt::try_lock_shared, FileExt::lock_shared)
    }

    pub fn exclusive(path: &Path) -> Result<Self, Error> {
        Self::acquire(path, FileExt::try_lock_exclusive, FileExt::lock_exclusive)
    }

    fn acquire(
        path: &Path,
        try_lock: fn(&File) -> io::Result<bool>,
        lock: fn(&File) -> io::Result<()>,
    ) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        if !try_lock(&file)? {
            info!("Waiting for a lock on {}", path.display());
            lock(&file)?;
        }
        Ok(Self { _file: file })
    }
}
