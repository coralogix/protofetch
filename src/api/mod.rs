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
    default_output_directory_name: PathBuf,
    cache_dependencies_directory_name: PathBuf,
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
    pub fn fetch(&self, ignore_lock_file: bool) -> Result<(), Box<dyn Error>> {
        do_fetch(
            ignore_lock_file,
            &self.cache,
            &self.root,
            &self.module_file_name,
            &self.lock_file_name,
            &self.cache_dependencies_directory_name,
            &self.default_output_directory_name,
        )
    }

    /// Creates a lock file based on the toml configuration file
    pub fn lock(&self) -> Result<(), Box<dyn Error>> {
        do_lock(
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
            &self.lock_file_name,
            &self.default_output_directory_name,
        )
    }

    pub fn clear_cache(&self) -> Result<(), Box<dyn Error>> {
        do_clear_cache(&self.cache)
    }
}
