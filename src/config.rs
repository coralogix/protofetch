use std::{collections::HashMap, path::PathBuf, str::FromStr};

use anyhow::{bail, Context};
use log::{debug, trace};
use serde::Deserialize;

use crate::model::protofetch::Protocol;

#[derive(Debug)]
pub struct ProtofetchConfig {
    pub cache_dir: PathBuf,
    pub default_protocol: Protocol,
    pub jobs: Option<usize>,
    pub copy_jobs: Option<usize>,
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
            jobs: raw_config.jobs,
            copy_jobs: raw_config.copy_jobs,
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
    #[serde(default)]
    jobs: Option<usize>,
    #[serde(default)]
    copy_jobs: Option<usize>,
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
    ) -> anyhow::Result<Self> {
        // Base config: override table (tests) takes priority; otherwise read
        // the optional config.toml file; fall back to defaults.
        let mut config: RawConfig = if let Some(table) = config_override {
            table.try_into().context("invalid config override")?
        } else if let Some(mut path) = config_dir {
            path.push("config.toml");
            debug!("Loading configuration from {}", path.display());
            match std::fs::read_to_string(&path) {
                Ok(contents) => toml::from_str(&contents)
                    .with_context(|| format!("invalid config file at {}", path.display()))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => RawConfig::default(),
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("could not read config file at {}", path.display())
                    })
                }
            }
        } else {
            RawConfig::default()
        };

        // Overlay env vars; they win over the file.  The function abstracts
        // the source so tests inject a HashMap and prod reads std::env.
        fn get<T>(
            key: &str,
            env_override: &Option<HashMap<String, String>>,
        ) -> anyhow::Result<Option<T>>
        where
            T: FromStr,
            T::Err: std::error::Error + Send + Sync + 'static,
        {
            let raw = match env_override {
                Some(map) => map.get(key).cloned(),
                None => std::env::var(key).ok(),
            };
            raw.map(|v| {
                v.parse()
                    .with_context(|| format!("invalid value for {key}"))
            })
            .transpose()
        }

        if let Some(dir) = get::<PathBuf>("PROTOFETCH_CACHE_DIR", &env_override)? {
            config.cache.dir = Some(dir);
        }
        if let Some(protocol) = get::<Protocol>("PROTOFETCH_GIT_PROTOCOL", &env_override)? {
            config.git.protocol = Some(protocol);
        }
        if let Some(jobs) = get::<usize>("PROTOFETCH_JOBS", &env_override)? {
            config.jobs = Some(jobs);
        }
        if let Some(copy_jobs) = get::<usize>("PROTOFETCH_COPY_JOBS", &env_override)? {
            config.copy_jobs = Some(copy_jobs);
        }

        Ok(config)
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
    if let Ok(path) = std::env::var("XDG_CACHE_HOME") {
        let mut path = PathBuf::from(path);
        path.push("protofetch");
        return Ok(path);
    }
    if let Some(mut path) = home::home_dir() {
        path.push(".cache");
        path.push("protofetch");
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
                git: GitConfig { protocol: None },
                jobs: None,
                copy_jobs: None,
            }
        )
    }

    #[test]
    fn load_environment() {
        let env = HashMap::from([
            ("PROTOFETCH_CACHE_DIR".to_owned(), "/cache".to_owned()),
            ("PROTOFETCH_GIT_PROTOCOL".to_owned(), "ssh".to_owned()),
            ("PROTOFETCH_JOBS".to_owned(), "16".to_owned()),
            ("PROTOFETCH_COPY_JOBS".to_owned(), "4".to_owned()),
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
                },
                jobs: Some(16),
                copy_jobs: Some(4),
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
                },
                jobs: None,
                copy_jobs: None,
            }
        )
    }
}
