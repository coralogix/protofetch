use std::num::ParseIntError;
use thiserror::Error;

pub mod protodep;
pub mod protofetch;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("IO error reading configuration toml: {0}")]
    IO(#[from] std::io::Error),
    #[error("TOML parsing error: {0}")]
    Toml(#[sfrom] toml::de::Error),
    #[error("Parse error")]
    Parse(#[from] ParseIntError),
    #[error("Enum parsing error: {0}")]
    Strum(#[from] strum::ParseError),
    #[error("Missing TOML key `{0}` while parsing")]
    MissingKey(String),
    #[error("AllowList rule is invalid: `{0}`")]
    ParseAllowlistRuleError(String),
    #[error("Missing url component `{0}` in string `{1}`")]
    MissingUrlComponent(String, String),
}
