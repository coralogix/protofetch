use crate::model::{
    protofetch::{
        ContentRoot, Coordinate, Dependency as ProtofetchDependency, Descriptor, ModuleName,
        Protocol, Revision, RevisionSpecification, Rules,
    },
    ParseError,
};
use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path, str::FromStr};
use toml::{map::Map, Value};

#[derive(PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Dependency {
    pub target: String,
    pub protocol: Option<String>,
    pub revision: String,
    pub subgroup: Option<String>,
    pub branch: Option<String>,
    pub path: Option<String>,
    pub ignores: Vec<String>,
    pub includes: Vec<String>,
}

#[derive(PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize)]
pub struct ProtodepDescriptor {
    #[serde(rename = "proto_outdir")]
    pub proto_out_dir: String,
    pub dependencies: Vec<Dependency>,
}

impl ProtodepDescriptor {
    pub fn from_file(path: &Path) -> Result<ProtodepDescriptor, ParseError> {
        debug!(
            "Attempting to read descriptor from protodep file {}",
            path.display()
        );
        let contents = std::fs::read_to_string(path)?;

        let descriptor = ProtodepDescriptor::from_toml_str(&contents);
        if let Err(err) = &descriptor {
            error!("Could not build a valid descriptor from a protodep toml file: {err}")
        }
        descriptor
    }

    pub fn from_toml_str(data: &str) -> Result<ProtodepDescriptor, ParseError> {
        let mut toml_value = toml::from_str::<Map<String, Value>>(data)?;

        let proto_out_dir = toml_value
            .remove("proto_outdir")
            .ok_or_else(|| ParseError::MissingKey("proto_outdir".to_string()))
            .and_then(|v| v.try_into::<String>().map_err(|e| e.into()))?;

        let dependencies = toml_value
            .get("dependencies")
            .and_then(|x| x.as_array())
            .get_or_insert(&vec![])
            .iter()
            .cloned()
            .map(|v| v.try_into::<Dependency>())
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ProtodepDescriptor {
            proto_out_dir,
            dependencies,
        })
    }

    pub fn into_proto_fetch(self) -> Result<Descriptor, ParseError> {
        let mut used_names =
            HashSet::from_iter(["name", "description", "proto_out_dir"].map(str::to_owned));
        let dependencies = self
            .dependencies
            .into_iter()
            .map(|dependency| convert_dependency(dependency, &mut used_names))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Descriptor {
            name: ModuleName::from("generated"),
            description: Some("Generated from protodep file".to_string()),
            proto_out_dir: self.proto_out_dir.into(),
            dependencies,
        })
    }
}

fn convert_dependency(
    dependency: Dependency,
    used_names: &mut HashSet<String>,
) -> Result<ProtofetchDependency, ParseError> {
    let protocol = dependency
        .protocol
        .as_deref()
        .map(Protocol::from_str)
        .transpose()?;
    let mut coordinate = Coordinate::from_url_protocol(&dependency.target, protocol)?;
    let mut rules = Rules::default();
    if coordinate.forge.eq_ignore_ascii_case("github.com") {
        if let Some((index, _)) = coordinate.path.match_indices('/').nth(1) {
            rules
                .content_roots
                .insert(ContentRoot::from_string(&coordinate.path[index + 1..]));
            coordinate.path.truncate(index);
        }
    }

    let repository = repository_name(&coordinate);
    let mut name = repository.to_owned();
    let mut suffix = 2;
    while !used_names.insert(name.clone()) {
        name = format!("{repository}-{suffix}");
        suffix += 1;
    }

    Ok(ProtofetchDependency {
        name: ModuleName::new(name),
        coordinate,
        specification: RevisionSpecification {
            revision: Revision::pinned(dependency.revision),
            branch: dependency.branch,
        },
        rules,
    })
}

fn repository_name(coordinate: &Coordinate) -> &str {
    let path = coordinate.path.trim_end_matches('/');
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    use pretty_assertions::assert_eq;

    #[test]
    fn repository_name_with_and_without_trailing_slash() {
        for path in ["repo", "group/repo", "group/repo/", "/tmp/repo/"] {
            let coordinate = Coordinate {
                forge: String::new(),
                path: path.to_owned(),
                protocol: None,
            };
            assert_eq!(repository_name(&coordinate), "repo", "{path}");
        }
    }

    #[test]
    fn load_valid_file_one_dep() {
        let str = r#"
proto_outdir = "./proto_out"

[[dependencies]]
  target = "github.com/opensaasstudio/plasma/protobuf"
  branch = "master"
  protocol = "ssh"
  revision = "1.0.0"
"#;

        let expected = ProtodepDescriptor {
            proto_out_dir: "./proto_out".to_string(),
            dependencies: vec![Dependency {
                target: "github.com/opensaasstudio/plasma/protobuf".to_string(),
                subgroup: None,
                branch: Some("master".to_string()),
                revision: "1.0.0".to_string(),
                path: None,
                ignores: vec![],
                includes: vec![],
                protocol: Some("ssh".to_string()),
            }],
        };

        assert_eq!(ProtodepDescriptor::from_toml_str(str).unwrap(), expected);
    }

    #[test]
    fn load_valid_file_multiple_dep() {
        let str = r#"
proto_outdir = "./proto_out"

[[dependencies]]
  target = "github.com/opensaasstudio/plasma/protobuf"
  branch = "master"
  protocol = "ssh"
  revision = "1.0.0"

[[dependencies]]
  target = "github.com/opensaasstudio/plasma1/protobuf"
  branch = "master"
  protocol = "https"
  revision = "2.0.0"

[[dependencies]]
  target = "github.com/opensaasstudio/plasma2/protobuf"
  revision = "3.0.0"
"#;

        let expected = ProtodepDescriptor {
            proto_out_dir: "./proto_out".to_string(),
            dependencies: vec![
                Dependency {
                    target: "github.com/opensaasstudio/plasma/protobuf".to_string(),
                    subgroup: None,
                    branch: Some("master".to_string()),
                    revision: "1.0.0".to_string(),
                    path: None,
                    ignores: vec![],
                    includes: vec![],
                    protocol: Some("ssh".to_string()),
                },
                Dependency {
                    target: "github.com/opensaasstudio/plasma1/protobuf".to_string(),
                    subgroup: None,
                    branch: Some("master".to_string()),
                    revision: "2.0.0".to_string(),
                    path: None,
                    ignores: vec![],
                    includes: vec![],
                    protocol: Some("https".to_string()),
                },
                Dependency {
                    target: "github.com/opensaasstudio/plasma2/protobuf".to_string(),
                    subgroup: None,
                    branch: None,
                    revision: "3.0.0".to_string(),
                    path: None,
                    ignores: vec![],
                    includes: vec![],
                    protocol: None,
                },
            ],
        };

        assert_eq!(ProtodepDescriptor::from_toml_str(str).unwrap(), expected);
    }

    #[test]
    fn load_valid_file_no_dep() {
        let str = r#"proto_outdir = "./proto_out""#;
        let expected = ProtodepDescriptor {
            proto_out_dir: "./proto_out".to_string(),
            dependencies: vec![],
        };

        assert_eq!(ProtodepDescriptor::from_toml_str(str).unwrap(), expected);
    }

    #[test]
    fn migrate_protodep_to_protofetch_file() {
        let protodep_toml = r#"
proto_outdir = "./proto_out"

[[dependencies]]
  target = "github.com/opensaasstudio/plasma"
  branch = "master"
  protocol = "ssh"
  revision = "1.5.0"

[[dependencies]]
  target = "github.com/org/api/proto/v1/"
  branch = "main"
  protocol = "https"
  revision = "2.0.0"

[[dependencies]]
  target = "github.com/org/api/proto/v2"
  revision = "3.0.0"

[[dependencies]]
  target = "gitlab.com/group/subgroup/api.git"
  revision = "4.0.0"
"#;

        let protofetch_toml = r#"
name = "generated"
description = "Generated from protodep file"
proto_out_dir = "./proto_out"
[plasma]
  url="github.com/opensaasstudio/plasma"
  protocol = "ssh"
  branch = "master"
  revision = "1.5.0"
[api]
  url = "github.com/org/api"
  content_roots = ["proto/v1"]
  protocol = "https"
  branch = "main"
  revision = "2.0.0"
[api-2]
  url = "github.com/org/api"
  content_roots = ["proto/v2"]
  revision = "3.0.0"
["api.git"]
  url = "gitlab.com/group/subgroup/api.git"
  revision = "4.0.0"
"#;
        let descriptor = ProtodepDescriptor::from_toml_str(protodep_toml)
            .unwrap()
            .into_proto_fetch()
            .unwrap();
        let toml = toml::to_string(&descriptor.into_toml()).unwrap();

        let expected = Descriptor::from_toml_str(protofetch_toml).unwrap();
        let result = Descriptor::from_toml_str(&toml).unwrap();
        assert_eq!(result, expected);
    }
}
