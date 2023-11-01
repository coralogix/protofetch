use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::ParseError;

use super::{Coordinate, ModuleName, RevisionSpecification};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockFile {
    pub dependencies: Vec<LockedDependency>,
}

impl LockFile {
    pub fn from_file(loc: &Path) -> Result<LockFile, ParseError> {
        let contents = std::fs::read_to_string(loc)?;
        let lockfile = toml::from_str::<LockFile>(&contents)?;

        Ok(lockfile)
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
    use crate::model::protofetch::{Protocol, Revision};

    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn load_lock_file() {
        let dependencies = vec![
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
        ];
        let lock_file = LockFile { dependencies };
        let value_toml = toml::Value::try_from(&lock_file).unwrap();
        let string_fmt = toml::to_string_pretty(&value_toml).unwrap();

        let new_lock_file = toml::from_str::<LockFile>(&string_fmt).unwrap();
        assert_eq!(lock_file, new_lock_file)
    }
}
