use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Serialize};

use crate::model::ParseError;

use super::{Coordinate, DependencyName, Rules};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockFile {
    pub module_name: String,
    pub proto_out_dir: Option<String>,
    pub dependencies: Vec<LockedDependency>,
}

impl LockFile {
    pub fn from_file(loc: &Path) -> Result<LockFile, ParseError> {
        let contents = std::fs::read_to_string(loc)?;
        let lockfile = toml::from_str::<LockFile>(&contents)?;

        Ok(lockfile)
    }
}

#[derive(Hash, Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct LockedDependency {
    pub name: DependencyName,
    pub commit_hash: String,
    pub coordinate: Coordinate,
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub dependencies: BTreeSet<DependencyName>,
    pub rules: Rules,
}

#[cfg(test)]
mod tests {
    use crate::model::protofetch::{AllowPolicies, DenyPolicies, FilePolicy};

    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn load_lock_file() {
        let lock_file = LockFile {
            module_name: "test".to_string(),
            proto_out_dir: None,
            dependencies: vec![
                LockedDependency {
                    name: DependencyName::new("dep1".to_string()),
                    commit_hash: "hash1".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::from([DependencyName::new("dep2".to_string())]),
                    rules: Rules::new(
                        true,
                        false,
                        BTreeSet::new(),
                        AllowPolicies::new(BTreeSet::from([FilePolicy::try_from_str(
                            "/proto/example.proto",
                        )
                        .unwrap()])),
                        DenyPolicies::default(),
                    ),
                },
                LockedDependency {
                    name: DependencyName::new("dep2".to_string()),
                    commit_hash: "hash2".to_string(),
                    coordinate: Coordinate::default(),
                    dependencies: BTreeSet::new(),
                    rules: Rules::default(),
                },
            ],
        };
        let value_toml = toml::Value::try_from(&lock_file).unwrap();
        let string_fmt = toml::to_string_pretty(&value_toml).unwrap();

        let new_lock_file = toml::from_str::<LockFile>(&string_fmt).unwrap();
        assert_eq!(lock_file, new_lock_file)
    }
}
