use std::{collections::HashMap, path::PathBuf};

use anyhow::bail;
use config::{Config, ConfigError, Environment, File, FileFormat};
use log::{debug, trace};
use serde::Deserialize;

use crate::model::protofetch::Protocol;

#[derive(Debug)]
pub struct ProtofetchConfig {
    pub cache_dir: PathBuf,
    pub default_protocol: Protocol,
}

impl ProtofetchConfig {
    pub fn load() -> anyhow::Result<Self> {
        let config_dir = config_dir();
        let raw_config = RawConfig::load(config_dir, None, None)?;

        let config = Self {
            cache_dir: match raw_config.cache.dir {
                Some(cache_dir) => cache_dir,
                None => default_cache_dir()?,
            },
            default_protocol: raw_config.git.protocol.unwrap_or(Protocol::Ssh),
        };
        trace!("Loaded configuration: {:?}", config);

        Ok(config)
    }
}

#[derive(Default, Debug, Deserialize, PartialEq, Eq)]
struct RawConfig {
    #[serde(default)]
    cache: CacheConfig,
    #[serde(default)]
    git: GitConfig,
}

#[derive(Default, Debug, Deserialize, PartialEq, Eq)]
struct CacheConfig {
    dir: Option<PathBuf>,
}

#[derive(Default, Debug, Deserialize, PartialEq, Eq)]
struct GitConfig {
    protocol: Option<Protocol>,
}

impl RawConfig {
    fn load(
        config_dir: Option<PathBuf>,
        config_override: Option<toml::Table>,
        env_override: Option<HashMap<String, String>>,
    ) -> Result<Self, ConfigError> {
        let mut builder = Config::builder();

        if let Some(mut path) = config_dir {
            path.push("config.toml");
            debug!("Loading configuration from {}", path.display());
            builder = builder.add_source(File::from(path).required(false));
        }

        if let Some(config_override) = config_override {
            builder = builder.add_source(File::from_str(
                &config_override.to_string(),
                FileFormat::Toml,
            ));
        }

        builder
            .add_source(
                Environment::with_prefix("PROTOFETCH")
                    .separator("_")
                    .source(env_override),
            )
            .build()?
            .try_deserialize()
    }
}

fn config_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PROTOFETCH_CONFIG_DIR") {
        return Some(PathBuf::from(path));
    }
    if let Ok(path) = std::env::var("XDG_CONFIG_HOME") {
        let mut path = PathBuf::from(path);
        path.push("protofetch");
        return Some(path);
    }
    if let Some(mut path) = home::home_dir() {
        path.push(".config");
        path.push("protofetch");
        return Some(path);
    }
    None
}

fn default_cache_dir() -> anyhow::Result<PathBuf> {
    if let Some(mut path) = home::home_dir() {
        path.push(".protofetch");
        path.push("cache");
        return Ok(path);
    }
    bail!("Could not find home dir. Please define $HOME env variable.")
}

#[cfg(test)]
mod tests {
    use toml::toml;

    use super::*;

    use pretty_assertions::assert_eq;

    #[test]
    fn load_empty() {
        let env = HashMap::new();
        let config = RawConfig::load(None, Some(Default::default()), Some(env)).unwrap();
        assert_eq!(
            config,
            RawConfig {
                cache: CacheConfig { dir: None },
                git: GitConfig { protocol: None }
            }
        )
    }

    #[test]
    fn load_environment() {
        let env = HashMap::from([
            ("PROTOFETCH_CACHE_DIR".to_owned(), "/cache".to_owned()),
            ("PROTOFETCH_GIT_PROTOCOL".to_owned(), "ssh".to_owned()),
        ]);
        let config = RawConfig::load(None, Some(Default::default()), Some(env)).unwrap();
        assert_eq!(
            config,
            RawConfig {
                cache: CacheConfig {
                    dir: Some("/cache".into())
                },
                git: GitConfig {
                    protocol: Some(Protocol::Ssh)
                }
            }
        )
    }

    #[test]
    fn load_config_file() {
        let env = HashMap::new();
        let config = RawConfig::load(
            None,
            Some(toml! {
                [cache]
                dir = "/cache"

                [git]
                protocol = "ssh"
            }),
            Some(env),
        )
        .unwrap();
        assert_eq!(
            config,
            RawConfig {
                cache: CacheConfig {
                    dir: Some("/cache".into())
                },
                git: GitConfig {
                    protocol: Some(Protocol::Ssh)
                }
            }
        )
    }
}
