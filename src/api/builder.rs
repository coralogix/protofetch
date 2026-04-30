use std::{env, error::Error, path::PathBuf, sync::Arc};

use crate::{
    config::ProtofetchConfig, fetch::ParallelConfig, git::cache::ProtofetchGitCache, Protofetch,
};

#[derive(Default)]
pub struct ProtofetchBuilder {
    // All other paths are relative to `root`
    root: Option<PathBuf>,
    module_file_name: Option<PathBuf>,
    lock_file_name: Option<PathBuf>,
    cache_directory_path: Option<PathBuf>,
    output_directory_name: Option<PathBuf>,
    parallel: ParallelConfig,
}

impl ProtofetchBuilder {
    /// Project root directory.
    ///
    /// Defaults to the current directory.
    pub fn root(mut self, path: impl Into<PathBuf>) -> Self {
        self.root = Some(path.into());
        self
    }

    /// Name of the protofetch configuration toml file.
    ///
    /// Defaults to `protofetch.toml`.
    pub fn module_file_name(mut self, path: impl Into<PathBuf>) -> Self {
        self.module_file_name = Some(path.into());
        self
    }

    /// Name of the protofetch lock file.
    ///
    /// Defaults to `protofetch.lock`.
    pub fn lock_file_name(mut self, path: impl Into<PathBuf>) -> Self {
        self.lock_file_name = Some(path.into());
        self
    }

    /// Name of the default output directory for proto source files.
    /// It will override the `proto_out_dir` set in the module toml config.
    pub fn output_directory_name(mut self, path: impl Into<PathBuf>) -> Self {
        self.output_directory_name = Some(path.into());
        self
    }

    /// Location of the protofetch cache directory.
    ///
    /// Defaults to `$HOME/.protofetch/cache`.
    pub fn cache_directory(mut self, path: impl Into<PathBuf>) -> Self {
        self.cache_directory_path = Some(path.into());
        self
    }

    /// Maximum number of in-flight network jobs (resolve + fetch). Defaults
    /// to 16. Setting `0` falls back to 1 (effectively sequential).
    pub fn jobs(mut self, jobs: usize) -> Self {
        self.parallel.network_jobs = jobs.max(1);
        self
    }

    /// Maximum number of in-flight disk jobs (worktree + copy). Defaults to
    /// `max(4, num_cpus / 2)`.
    pub fn copy_jobs(mut self, jobs: usize) -> Self {
        self.parallel.copy_jobs = jobs.max(1);
        self
    }

    pub fn try_build(self) -> Result<Protofetch, Box<dyn Error>> {
        let config = ProtofetchConfig::load()?;

        let Self {
            root,
            module_file_name,
            lock_file_name,
            output_directory_name,
            cache_directory_path,
            parallel,
        } = self;
        let root = match root {
            Some(root) => root,
            None => env::current_dir()?,
        };

        let module_file_name = module_file_name.unwrap_or_else(|| PathBuf::from("protofetch.toml"));

        let lock_file_name = lock_file_name.unwrap_or_else(|| PathBuf::from("protofetch.lock"));

        let cache_directory = root.join(cache_directory_path.unwrap_or(config.cache_dir));

        let cache = ProtofetchGitCache::new(cache_directory, config.default_protocol)?;

        Ok(Protofetch {
            cache: Arc::new(cache),
            root,
            module_file_name,
            lock_file_name,
            output_directory_name,
            parallel,
        })
    }
}
