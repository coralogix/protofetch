use std::{collections::HashMap, path::PathBuf, str::FromStr};

use config::{Config, ConfigError, File};
use serde::Deserialize;
use toml::Value;

#[derive(Deserialize, Debug)]
pub struct DepEntry {
    pub url: String,
    pub revision: String,
}

#[derive(Deserialize, Debug)]
pub struct ProtofetchConfig {
    pub version: String,
    pub out_dir: PathBuf,
    pub dep_entries: HashMap<String, DepEntry>,
}

impl ProtofetchConfig {
    pub fn load() -> Result<ProtofetchConfig, String> {
        let contents = std::fs::read_to_string("protofetch.toml").map_err(|err| err.to_string())?;
        let value = toml::from_str(&contents).map_err(|err| err.to_string())?;

        Self::parse(value)
    }

    fn parse(value: toml::Value) -> Result<ProtofetchConfig, String> {
        match value {
            Value::Table(map) => {
                let v = map
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0.0.1")
                    .to_string();
                let out_dir = map
                    .get("out_dir")
                    .and_then(|v| v.as_str())
                    .map(|v| PathBuf::from_str(v).unwrap())
                    .unwrap_or(PathBuf::from_str("./protobuf-deps").unwrap());
                let deps = map
                    .into_iter()
                    .filter(|entry| entry.0 != "version" && entry.0 != "out_dir")
                    .map(|(name, entry)| {
                        entry.try_into::<DepEntry>().map(|entry| (name, entry))
                    })
                    .collect::<Result<HashMap<String, DepEntry>, _>>()
                    .unwrap();

                Ok(ProtofetchConfig {
                    version: v,
                    out_dir,
                    dep_entries: deps,
                })
            }
            _ => Err("Unexpected".to_string()),
        }
    }
}
