use std::num::ParseIntError;
use thiserror::Error;

pub mod git_oid;
pub mod protodep;
pub mod protofetch;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("IO error reading configuration toml: {0}")]
    IO(#[from] std::io::Error),
    #[error("TOML parsing error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Parse error")]
    Parse(#[from] ParseIntError),
    #[error("Invalid protocol: {0}")]
    InvalidProtocol(String),
    #[error("Invalid commit hash: {0}")]
    InvalidCommitHash(String),
    #[error("Missing TOML key `{0}` while parsing")]
    MissingKey(String),
    #[error("{policy_type} rule is invalid: `{rule}`")]
    ParsePolicyRuleError {
        policy_type: &'static str,
        rule: String,
    },
    #[error("{policy_type} rule is invalid: `{rule}`")]
    ParsePolicyRegexRuleError {
        policy_type: &'static str,
        rule: String,
        #[source]
        error: regex_lite::Error,
    },
    #[error("Missing url component `{0}` in string `{1}`")]
    MissingUrlComponent(String, String),
    #[error("Unsupported lock file version {0}")]
    UnsupportedLockFileVersion(toml::Value),
    #[error("Old lock file version {0}, consider running \"protofetch update\"")]
    OldLockFileVersion(i64),
}
