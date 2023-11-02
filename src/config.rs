use std::{collections::HashMap, path::PathBuf};

use config::{Config, ConfigError, Environment};
use serde::Deserialize;

use crate::model::protofetch::Protocol;

pub struct ProtofetchConfig {
    pub cache_dir: Option<PathBuf>,
    pub default_protocol: Option<Protocol>,
}

impl ProtofetchConfig {
    pub fn load() -> anyhow::Result<Self> {
        let raw_config = RawConfig::load(None)?;

        Ok(Self {
            cache_dir: raw_config.cache.dir,
            default_protocol: raw_config.git.protocol,
        })
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
    fn load(env: Option<HashMap<String, String>>) -> Result<Self, ConfigError> {
        Config::builder()
            .add_source(
                Environment::with_prefix("PROTOFETCH")
                    .separator("_")
                    .source(env),
            )
            .build()?
            .try_deserialize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pretty_assertions::assert_eq;

    #[test]
    fn load_empty() {
        let env = HashMap::from([]);
        let config = RawConfig::load(Some(env)).unwrap();
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
        let config = RawConfig::load(Some(env)).unwrap();
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
