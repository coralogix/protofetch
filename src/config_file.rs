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
    pub dep_entries: Vec<DepEntry>,
}

impl Default for ProtofetchConfig {
    fn default() -> Self {
        Self {
            version: "0.0.1".to_string(),
            dep_entries: Vec::new(),
        }
    }
}

impl ProtofetchConfig {
    pub fn load() -> Result<ProtofetchConfig, String> {
        let contents = std::fs::read_to_string("protofetch.toml").map_err(|err| err.to_string())?;
        let value = toml::from_str(&contents).map_err(|err| err.to_string())?;

        return Self::parse(value);
    }

    fn parse(value: toml::Value) -> Result<ProtofetchConfig, String> {
        match value {
            Value::Table(map) => {
                let v = map
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0.0.1")
                    .to_string();
                let deps = map
                    .into_iter()
                    .filter(|entry| entry.0 != "version")
                    .map(|entry| entry.1.try_into::<DepEntry>())
                    .collect::<Result<Vec<DepEntry>, _>>()
                    .unwrap();

                return Ok(ProtofetchConfig {
                    version: v,
                    dep_entries: deps,
                });
            }
            _ => Err("Unexpected".to_string()),
        }
    }
}
