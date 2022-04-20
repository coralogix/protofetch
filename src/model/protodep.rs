use crate::model::{
    protofetch::{Coordinate, Dependency as ProtofetchDependency, Descriptor, Protocol, Revision},
    ParseError,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};
use toml::Value;
use std::str::FromStr;

#[derive(PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Dependency {
    pub target: String,
    pub protocol: String,
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
        let contents = std::fs::read_to_string(path)?;

        ProtodepDescriptor::from_str(&contents)
    }

    pub fn from_str(data: &str) -> Result<ProtodepDescriptor, ParseError> {
        let mut toml_value = toml::from_str::<HashMap<String, Value>>(data)?;

        let proto_out_dir = toml_value
            .remove("proto_outdir")
            .ok_or_else(|| ParseError::MissingKey("proto_outdir".to_string()))
            .and_then(|v| v.try_into::<String>().map_err(|e| e.into()))?;

        let dependencies = toml_value
            .get("dependencies")
            .and_then(|x| x.as_array())
            .get_or_insert(&vec![])
            .to_vec()
            .into_iter()
            .map(|v| v.try_into::<Dependency>())
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ProtodepDescriptor {
            proto_out_dir,
            dependencies,
        })
    }

    pub fn to_proto_fetch(self: Self) -> Result<Descriptor, ParseError> {
        fn convert_dependency(d: Dependency) -> Result<ProtofetchDependency, ParseError> {
            let protocol: Protocol = Protocol::from_str(&d.protocol)?;
            let coordinate = Coordinate::from_url(d.target.as_str(), protocol)?;
            let revision = Revision::Arbitrary {
                revision: d.revision,
            };
            Ok(ProtofetchDependency {
                name: coordinate.repository.clone(),
                coordinate,
                revision,
            })
        }

        let dependencies = self
            .dependencies
            .into_iter()
            .map(convert_dependency)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Descriptor {
            name: "change name".to_string(),
            description: Some("Generated from protodep file".to_string()),
            dependencies,
        })
    }
}

#[test]
fn load_valid_file_one_dep() {
    let str = r#"
proto_outdir = "./proto"

[[dependencies]]
  target = "github.com/opensaasstudio/plasma/protobuf"
  branch = "master"
  protocol = "ssh"
  revision = "1.0.0"
"#;

    let expected = ProtodepDescriptor {
        proto_out_dir: "./proto".to_string(),
        dependencies: vec![Dependency {
            target: "github.com/opensaasstudio/plasma/protobuf".to_string(),
            subgroup: None,
            branch: Some("master".to_string()),
            revision: "1.0.0".to_string(),
            path: None,
            ignores: vec![],
            includes: vec![],
            protocol: "ssh".to_string(),
        }],
    };

    assert_eq!(ProtodepDescriptor::from_str(str).unwrap(), expected);
}

#[test]
fn load_valid_file_multiple_dep() {
    let str = r#"
proto_outdir = "./proto"

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
  protocol = "ssh"
  revision = "3.0.0"
"#;

    let expected = ProtodepDescriptor {
        proto_out_dir: "./proto".to_string(),
        dependencies: vec![
            Dependency {
                target: "github.com/opensaasstudio/plasma/protobuf".to_string(),
                subgroup: None,
                branch: Some("master".to_string()),
                revision: "1.0.0".to_string(),
                path: None,
                ignores: vec![],
                includes: vec![],
                protocol: "ssh".to_string(),
            },
            Dependency {
                target: "github.com/opensaasstudio/plasma1/protobuf".to_string(),
                subgroup: None,
                branch: Some("master".to_string()),
                revision: "2.0.0".to_string(),
                path: None,
                ignores: vec![],
                includes: vec![],
                protocol: "https".to_string(),
            },
            Dependency {
                target: "github.com/opensaasstudio/plasma2/protobuf".to_string(),
                subgroup: None,
                branch: None,
                revision: "3.0.0".to_string(),
                path: None,
                ignores: vec![],
                includes: vec![],
                protocol: "ssh".to_string(),
            },
        ],
    };

    assert_eq!(ProtodepDescriptor::from_str(str).unwrap(), expected);
}

#[test]
fn load_valid_file_no_dep() {
    let str = r#"
proto_outdir = "./proto"
"#;

    let expected = ProtodepDescriptor {
        proto_out_dir: "./proto".to_string(),
        dependencies: vec![],
    };

    assert_eq!(ProtodepDescriptor::from_str(str).unwrap(), expected);
}

#[test]
fn migrate_protodep_to_protofetch_file() {
    let protodep_toml = r#"
proto_outdir = "./proto"

[[dependencies]]
  target = "github.com/opensaasstudio/plasma"
  branch = "master"
  protocol = "ssh"
  revision = "1.5.0"
"#;

    let protofetch_toml = r#"
name = "change name"
description = "Generated from protodep file"
[plasma]
  url="github.com/opensaasstudio/plasma"
  protocol = "ssh"
  revision = "1.5.0"
"#;
    let descriptor = ProtodepDescriptor::from_str(protodep_toml).unwrap().to_proto_fetch().unwrap();
    let toml = toml::to_string(&descriptor.to_toml()).unwrap();

    let expected = Descriptor::from_toml_str(protofetch_toml).unwrap();
    let result = Descriptor::from_toml_str(&toml).unwrap();
    assert_eq!(result, expected);
}
