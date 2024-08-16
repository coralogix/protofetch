use std::{
    fs::File,
    path::Path,
    time::{Duration, Instant},
};

use fs4::fs_std::FileExt;
use log::debug;
use thiserror::Error;

pub struct FileLock {
    _file: File,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub struct Error(#[from] std::io::Error);

impl FileLock {
    pub fn new(path: &Path) -> Result<Self, Error> {
        let file = File::create(path)?;
        let start = Instant::now();
        loop {
            match file.try_lock_exclusive() {
                Ok(_) => {
                    return Ok(Self { _file: file });
                }
                Err(error)
                    if error.raw_os_error() == fs4::lock_contended_error().raw_os_error()
                        && start.elapsed().as_secs() < 300 =>
                {
                    debug!("Failed to acquire a lock on {}, retrying", path.display());
                    std::thread::sleep(Duration::from_secs(1));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}
