use std::{env, error::Error, path::PathBuf};

use home::home_dir;

use crate::{cache::ProtofetchGitCache, cli::HttpGitAuth, Protofetch};

#[derive(Default)]
pub struct ProtofetchBuilder {
    // All other paths are relative to `root`
    root: Option<PathBuf>,
    module_file_name: Option<PathBuf>,
    lock_file_name: Option<PathBuf>,
    cache_directory_path: Option<PathBuf>,

    // These fields are deprecated
    http_username: Option<String>,
    http_password: Option<String>,
    default_output_directory_name: Option<PathBuf>,
    cache_dependencies_directory_name: Option<PathBuf>,
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

    /// Name of the default output directory for proto source files,
    /// that will be used if `proto_out_dir` is not set in the module toml config.
    ///
    /// Defaults to `proto_src`.
    #[deprecated(
        since = "0.0.23",
        note = "overriding the default is not very useful, consider specifying `proto_out_dir` instead"
    )]
    #[doc(hidden)]
    pub fn default_output_directory_name(mut self, path: impl Into<PathBuf>) -> Self {
        self.default_output_directory_name = Some(path.into());
        self
    }

    /// Location of the protofetch cache directory.
    ///
    /// Defaults to `$HOME/.protofetch/cache`.
    pub fn cache_directory(mut self, path: impl Into<PathBuf>) -> Self {
        self.cache_directory_path = Some(path.into());
        self
    }

    #[deprecated(
        since = "0.0.23",
        note = "configure credentials using standard git configuration instead"
    )]
    #[doc(hidden)]
    pub fn http_credentials(mut self, username: Option<String>, password: Option<String>) -> Self {
        self.http_username = username;
        self.http_password = password;
        self
    }

    #[deprecated(
        since = "0.0.23",
        note = "this is an implementation detail and should not be overridden"
    )]
    #[doc(hidden)]
    pub fn cache_dependencies_directory_name(mut self, path: impl Into<PathBuf>) -> Self {
        self.cache_dependencies_directory_name = Some(path.into());
        self
    }

    pub fn try_build(self) -> Result<Protofetch, Box<dyn Error>> {
        let Self {
            root,
            module_file_name,
            lock_file_name,
            default_output_directory_name,
            cache_directory_path,
            http_username,
            http_password,
            cache_dependencies_directory_name,
        } = self;
        let root = match root {
            Some(root) => root,
            None => env::current_dir()?,
        };

        let module_file_name = module_file_name.unwrap_or_else(|| PathBuf::from("protofetch.toml"));

        let lock_file_name = lock_file_name.unwrap_or_else(|| PathBuf::from("protofetch.lock"));

        let default_output_directory_name =
            default_output_directory_name.unwrap_or_else(|| PathBuf::from("proto_src"));

        let cache_directory =
            root.join(cache_directory_path.unwrap_or_else(default_cache_directory));

        let git_config = git2::Config::open_default()?;

        let http_credentials =
            HttpGitAuth::resolve_git_auth(&git_config, http_username, http_password);

        let cache = ProtofetchGitCache::new(cache_directory, git_config, http_credentials)?;

        let cache_dependencies_directory_name =
            cache_dependencies_directory_name.unwrap_or_else(|| PathBuf::from("dependencies"));

        Ok(Protofetch {
            cache,
            root,
            module_file_name,
            lock_file_name,
            default_output_directory_name,
            cache_dependencies_directory_name,
        })
    }
}

fn default_cache_directory() -> PathBuf {
    let mut cache_directory =
        home_dir().expect("Could not find home dir. Please define $HOME env variable.");
    cache_directory.push(".protofetch/cache");
    cache_directory
}
