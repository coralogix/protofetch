use std::{
    error::Error,
    path::{Path, PathBuf},
};

use crate::{
    cache::ProtofetchGitCache,
    cli::command_handlers::{do_clean, do_clear_cache, do_fetch, do_init, do_lock, do_migrate},
};

mod builder;

pub use builder::ProtofetchBuilder;

pub struct Protofetch {
    cache: ProtofetchGitCache,
    root: PathBuf,
    module_file_name: PathBuf,
    lock_file_name: PathBuf,
    output_directory_name: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LockMode {
    /// Verify that the lock file is up to date. This mode should be normally used on CI.
    Locked,
    /// Update the lock file if necessary.
    Update,
    /// Recreate the lock file from scratch.
    Recreate,
}

impl Protofetch {
    pub fn builder() -> ProtofetchBuilder {
        ProtofetchBuilder::default()
    }

    /// Creates an initial protofetch setup
    pub fn init(&self, name: Option<String>) -> Result<(), Box<dyn Error>> {
        do_init(&self.root, name, &self.module_file_name)
    }

    /// Fetches dependencies defined in the toml configuration file
    pub fn fetch(&self, lock_mode: LockMode) -> Result<(), Box<dyn Error>> {
        do_fetch(
            lock_mode,
            &self.cache,
            &self.root,
            &self.module_file_name,
            &self.lock_file_name,
            self.output_directory_name.as_deref(),
        )
    }

    /// Creates, updates or verifies a lock file based on the toml configuration file
    pub fn lock(&self, lock_mode: LockMode) -> Result<(), Box<dyn Error>> {
        do_lock(
            lock_mode,
            &self.cache,
            &self.root,
            &self.module_file_name,
            &self.lock_file_name,
        )?;
        Ok(())
    }

    /// Migrates a protodep.toml file to the protofetch format
    pub fn migrate(
        &self,
        name: Option<String>,
        source_directory_path: impl AsRef<Path>,
    ) -> Result<(), Box<dyn Error>> {
        do_migrate(
            &self.root,
            name,
            &self.module_file_name,
            source_directory_path.as_ref(),
        )
    }

    /// Delete generated proto sources and the lock file
    pub fn clean(&self) -> Result<(), Box<dyn Error>> {
        do_clean(
            &self.root,
            &self.module_file_name,
            &self.lock_file_name,
            self.output_directory_name.as_deref(),
        )
    }

    pub fn clear_cache(&self) -> Result<(), Box<dyn Error>> {
        do_clear_cache(&self.cache)
    }
}
