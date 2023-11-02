use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::ParseError;

use super::{Coordinate, ModuleName, RevisionSpecification};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockFile {
    pub dependencies: Vec<LockedDependency>,
}

const VERSION: i64 = 2;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct VersionedLockFile<'a> {
    pub version: i64,
    #[serde(flatten)]
    pub content: &'a LockFile,
}

impl LockFile {
    pub fn from_file(file: &Path) -> Result<LockFile, ParseError> {
        LockFile::from_str(&std::fs::read_to_string(file)?)
    }

    pub fn from_str(s: &str) -> Result<LockFile, ParseError> {
        let mut table = toml::from_str::<toml::Table>(s)?;
        match table.remove("version") {
            Some(toml::Value::Integer(VERSION)) => table.try_into::<LockFile>().map_err(Into::into),
            Some(other) => Err(ParseError::UnsupportedLockFileVersion(other)),
            None => Err(ParseError::OldLockFileVersion(1)),
        }
    }

    pub fn to_string(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(&VersionedLockFile {
            version: VERSION,
            content: self,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct LockedCoordinateRevisionSpecification {
    #[serde(flatten)]
    pub coordinate: Option<Coordinate>,
    #[serde(flatten)]
    pub specification: RevisionSpecification,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct LockedDependency {
    pub name: ModuleName,
    #[serde(flatten)]
    pub coordinate: Coordinate,
    #[serde(flatten)]
    pub specification: RevisionSpecification,
    pub commit_hash: String,
}

#[cfg(test)]
mod tests {
    use toml::toml;

    use crate::model::protofetch::{Protocol, Revision};

    use super::*;

    use pretty_assertions::assert_eq;

    #[test]
    fn load_save_lock_file() {
        let text = toml::to_string_pretty(&toml! {
            version = 2

            [[dependencies]]
            name = "dep1"
            forge = "example.com"
            organization = "org"
            repository = "dep1"
            protocol = "https"
            revision = "1.0.0"
            branch = "main"
            commit_hash = "hash1"

            [[dependencies]]
            name = "dep2"
            forge = "example.com"
            organization = "org"
            repository = "dep2"
            commit_hash = "hash2"
        })
        .unwrap();
        let data = LockFile {
            dependencies: vec![
                LockedDependency {
                    name: ModuleName::new("dep1".to_string()),
                    commit_hash: "hash1".to_string(),
                    coordinate: Coordinate::from_url_protocol(
                        "example.com/org/dep1",
                        Some(Protocol::Https),
                    )
                    .unwrap(),
                    specification: RevisionSpecification {
                        revision: Revision::pinned("1.0.0"),
                        branch: Some("main".to_owned()),
                    },
                },
                LockedDependency {
                    name: ModuleName::new("dep2".to_string()),
                    commit_hash: "hash2".to_string(),
                    coordinate: Coordinate::from_url("example.com/org/dep2").unwrap(),
                    specification: RevisionSpecification::default(),
                },
            ],
        };
        let parsed = LockFile::from_str(&text).unwrap();
        let formatted = data.to_string().unwrap();
        assert_eq!(parsed, data);
        assert_eq!(formatted, text);
    }

    #[test]
    fn load_lock_file_v1() {
        let text = toml::to_string_pretty(&toml! {
            module_name = "foo"
        })
        .unwrap();
        LockFile::from_str(&text).expect_err("should not parse v1 lock file");
    }
}
