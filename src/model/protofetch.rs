use derive_new::new;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    path::{Path, PathBuf},
};

use crate::model::ParseError;
use lazy_static::lazy_static;
use log::debug;
use toml::{map::Map, Value};

#[derive(new, PartialEq, Eq, Hash, Clone, Serialize, Deserialize, Ord, PartialOrd)]
pub struct Coordinate {
    pub forge: String,
    pub organization: String,
    pub repository: String,
    pub protocol: Protocol,
}

impl Coordinate {
    pub fn from_url(url: &str, protocol: Protocol) -> Result<Coordinate, ParseError> {
        let re: Regex =
            Regex::new(r"^(?P<forge>[^/]+)/(?P<organization>[^/]+)/(?P<repository>[^/]+)/?$")
                .unwrap();
        let url_parse_results = re.captures(url);
        let url_parse_results = url_parse_results.as_ref();

        Ok(Coordinate {
            forge: url_parse_results
                .and_then(|c| c.name("forge"))
                .map(|s| s.as_str().to_string())
                .ok_or_else(|| {
                    ParseError::MissingUrlComponent("forge".to_string(), url.to_string())
                })?,
            organization: url_parse_results
                .and_then(|c| c.name("organization"))
                .map(|s| s.as_str().to_string())
                .ok_or_else(|| {
                    ParseError::MissingUrlComponent("organization".to_string(), url.to_string())
                })?,
            repository: url_parse_results
                .and_then(|c| c.name("repository"))
                .map(|s| s.as_str().to_string())
                .ok_or_else(|| {
                    ParseError::MissingUrlComponent("repository".to_string(), url.to_string())
                })?,
            protocol,
        })
    }

    pub fn as_path(&self) -> PathBuf {
        let mut result = PathBuf::new();

        result.push(self.forge.clone());
        result.push(self.organization.clone());
        result.push(self.repository.clone());

        result
    }

    pub fn url(&self) -> String {
        match self.protocol {
            Protocol::Https => format!(
                "https://{}/{}/{}",
                self.forge, self.organization, self.repository
            ),
            Protocol::Ssh => format!(
                "git@{}:{}/{}.git",
                self.forge, self.organization, self.repository
            ),
        }
    }
}

impl Debug for Coordinate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let forge = match self.protocol {
            Protocol::Https => format!("https://{}/", self.forge),
            Protocol::Ssh => format!("git@{}:", self.forge),
        };

        write!(f, "{}{}/{}", forge, self.organization, self.repository)
    }
}

impl Display for Coordinate {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/{}",
            self.forge, self.organization, self.repository
        )
    }
}

#[derive(
    PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize, Ord, PartialOrd, EnumString,
)]
pub enum Protocol {
    #[serde(rename = "https")]
    #[strum(ascii_case_insensitive)]
    Https,
    #[serde(rename = "ssh")]
    #[strum(ascii_case_insensitive)]
    Ssh,
}

impl Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Protocol::Https => f.write_str("https"),
            Protocol::Ssh => f.write_str("ssh"),
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Ord, PartialOrd)]
pub enum Revision {
    #[allow(dead_code)]
    Semver {
        major: SemverComponent,
        minor: SemverComponent,
        patch: SemverComponent,
    },
    Arbitrary {
        revision: String,
    },
}

impl Display for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Revision::Semver {
                major,
                minor,
                patch,
            } => write!(f, "{}.{}.{}", major, minor, patch),
            Revision::Arbitrary { revision } => f.write_str(revision),
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Ord, PartialOrd)]
#[allow(dead_code)]
pub enum SemverComponent {
    Fixed(u8),
    Wildcard,
}

impl Display for SemverComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            SemverComponent::Fixed(c) => write!(f, "{}", c),
            SemverComponent::Wildcard => f.write_str("*"),
        }
    }
}

#[derive(new, Serialize, Debug, PartialEq, Eq, Ord, PartialOrd)]
pub struct Dependency {
    pub name: String,
    pub coordinate: Coordinate,
    pub revision: Revision,
}

#[derive(new, Serialize, PartialEq, Debug)]
pub struct Descriptor {
    pub name: String,
    pub description: Option<String>,
    pub proto_out_dir: Option<String>,
    pub dependencies: Vec<Dependency>,
}

impl Descriptor {
    pub fn from_file(path: &Path) -> Result<Descriptor, ParseError> {
        debug!(
            "Attempting to read descriptor from protofetch file {}",
            path.display()
        );
        let contents = std::fs::read_to_string(path)?;

        let descriptor = Descriptor::from_toml_str(&contents);
        if let Err(err) = &descriptor {
            error!(
                "Could not build a valid descriptor from a protofetch toml file due to err {err}"
            )
        }
        descriptor
    }

    pub fn from_toml_str(data: &str) -> Result<Descriptor, ParseError> {
        let mut toml_value = toml::from_str::<HashMap<String, Value>>(data)?;

        let name = toml_value
            .remove("name")
            .ok_or_else(|| ParseError::MissingKey("name".to_string()))
            .and_then(|v| v.try_into::<String>().map_err(|e| e.into()))?;

        let description = toml_value
            .remove("description")
            .map(|v| v.try_into::<String>())
            .map_or(Ok(None), |v| v.map(Some))?;

        let proto_out_dir = toml_value
            .remove("proto_out_dir")
            .map(|v| v.try_into::<String>())
            .map_or(Ok(None), |v| v.map(Some))?;

        let dependencies = toml_value
            .into_iter()
            .map(|(k, v)| parse_dependency(k, &v))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Descriptor::new(
            name,
            description,
            proto_out_dir,
            dependencies,
        ))
    }

    pub fn to_toml(self) -> Value {
        let mut description = Map::new();
        description.insert("name".to_string(), Value::String(self.name));
        if let Some(d) = self.description {
            description.insert("description".to_string(), Value::String(d));
        }
        if let Some(proto_out) = self.proto_out_dir {
            description.insert("proto_out_dir".to_string(), Value::String(proto_out));
        }

        for d in self.dependencies {
            let mut dependency = Map::new();
            dependency.insert(
                "protocol".to_string(),
                Value::String(d.coordinate.protocol.to_string()),
            );
            dependency.insert("url".to_string(), Value::String(d.coordinate.to_string()));
            dependency.insert(
                "revision".to_string(),
                Value::String(d.revision.to_string()),
            );
            description.insert(d.name, Value::Table(dependency));
        }
        Value::Table(description)
    }
}

fn parse_dependency(name: String, value: &toml::Value) -> Result<Dependency, ParseError> {
    let protocol = match value.get("protocol") {
        None => Protocol::Https,
        Some(toml) => toml.clone().try_into::<Protocol>()?,
    };

    let coordinate = value
        .get("url")
        .ok_or_else(|| ParseError::MissingKey("url".to_string()))
        .and_then(|x| x.clone().try_into::<String>().map_err(|e| e.into()))
        .and_then(|url| Coordinate::from_url(&url, protocol))?;

    let revision = parse_revision(
        value
            .get("revision")
            .ok_or_else(|| ParseError::MissingKey("revision".to_string()))?,
    )?;

    Ok(Dependency {
        name,
        coordinate,
        revision,
    })
}

lazy_static! {
    static ref SEMVER_REGEX: Regex =
        Regex::new(r"^v?(?P<major>\d+)(?:\.(?P<minor>\d+)(?:\.(?P<patch>\d+))?)?$").unwrap();
}

fn parse_revision(value: &toml::Value) -> Result<Revision, ParseError> {
    let revstring = value.clone().try_into::<String>()?;

    Ok(Revision::Arbitrary {
        revision: revstring,
    })
}

fn _parse_semver(revstring: &str) -> Result<Revision, ParseError> {
    let results = SEMVER_REGEX.captures(revstring);

    Ok(
        match (
            results.as_ref().and_then(|c| c.name("major")),
            results.as_ref().and_then(|c| c.name("minor")),
            results.as_ref().and_then(|c| c.name("patch")),
        ) {
            (Some(major), Some(minor), Some(patch)) => Revision::Semver {
                major: SemverComponent::Fixed(major.as_str().parse::<u8>()?),
                minor: SemverComponent::Fixed(minor.as_str().parse::<u8>()?),
                patch: SemverComponent::Fixed(patch.as_str().parse::<u8>()?),
            },
            (Some(major), Some(minor), _) => Revision::Semver {
                major: SemverComponent::Fixed(major.as_str().parse::<u8>()?),
                minor: SemverComponent::Fixed(minor.as_str().parse::<u8>()?),
                patch: SemverComponent::Wildcard,
            },
            (Some(major), _, _) => Revision::Semver {
                major: SemverComponent::Fixed(major.as_str().parse::<u8>()?),
                minor: SemverComponent::Wildcard,
                patch: SemverComponent::Wildcard,
            },
            _ => todo!(),
        },
    )
}

#[derive(new, Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedDependency {
    pub name: String,
    pub commit_hash: String,
    pub coordinate: Coordinate,
}

#[test]
fn load_valid_file_one_dep() {
    let str = r#"
name = "test_file"
description = "this is a description"
[dependency1]
  protocol = "https"
  url = "github.com/org/repo"
  revision = "1.0.0"
"#;
    let expected = Descriptor {
        name: "test_file".to_string(),
        description: Some("this is a description".to_string()),
        proto_out_dir: None,
        dependencies: vec![Dependency {
            name: "dependency1".to_string(),
            coordinate: Coordinate {
                forge: "github.com".to_string(),
                organization: "org".to_string(),
                repository: "repo".to_string(),
                protocol: Protocol::Https,
            },
            revision: Revision::Arbitrary {
                revision: "1.0.0".to_string(),
            },
        }],
    };
    assert_eq!(Descriptor::from_toml_str(str).unwrap(), expected);
}

#[test]
fn load_valid_file_multiple_dep() {
    let str = r#"
name = "test_file"
[dependency1]
  protocol = "https"
  url = "github.com/org/repo"
  revision = "1.0.0"
[dependency2]
  protocol = "https"
  url = "github.com/org/repo"
  revision = "2.0.0"
[dependency3]
  protocol = "https"
  url = "github.com/org/repo"
  revision = "3.0.0"
"#;
    let mut expected = Descriptor {
        name: "test_file".to_string(),
        description: None,
        proto_out_dir: None,
        dependencies: vec![
            Dependency {
                name: "dependency1".to_string(),
                coordinate: Coordinate {
                    forge: "github.com".to_string(),
                    organization: "org".to_string(),
                    repository: "repo".to_string(),
                    protocol: Protocol::Https,
                },
                revision: Revision::Arbitrary {
                    revision: "1.0.0".to_string(),
                },
            },
            Dependency {
                name: "dependency2".to_string(),
                coordinate: Coordinate {
                    forge: "github.com".to_string(),
                    organization: "org".to_string(),
                    repository: "repo".to_string(),
                    protocol: Protocol::Https,
                },
                revision: Revision::Arbitrary {
                    revision: "2.0.0".to_string(),
                },
            },
            Dependency {
                name: "dependency3".to_string(),
                coordinate: Coordinate {
                    forge: "github.com".to_string(),
                    organization: "org".to_string(),
                    repository: "repo".to_string(),
                    protocol: Protocol::Https,
                },
                revision: Revision::Arbitrary {
                    revision: "3.0.0".to_string(),
                },
            },
        ],
    };
    assert_eq!(
        Descriptor::from_toml_str(str).unwrap().dependencies.sort(),
        expected.dependencies.sort()
    );
}

#[test]
fn load_file_no_deps() {
    let str = r#"name = "test_file""#;
    let expected = Descriptor {
        name: "test_file".to_string(),
        description: None,
        proto_out_dir: None,
        dependencies: vec![],
    };
    assert_eq!(Descriptor::from_toml_str(str).unwrap(), expected);
}

#[test]
fn load_invalid_protocol() {
    let str = r#"
name = "test_file"
[dependency1]
  protocol = "ftp"
  url = "github.com/org/repo"
  revision = "1.0.0"
"#;
    assert_eq!(Descriptor::from_toml_str(str).is_err(), true);
}

#[test]
fn load_invalid_url() {
    let str = r#"
name = "test_file"
[dependency1]
  protocol = "ftp"
  url = "github.com/org"
  revision = "1.0.0"
"#;
    assert_eq!(Descriptor::from_toml_str(str).is_err(), true);
}

#[test]
fn build_coordinate() {
    let str = "github.com/coralogix/cx-api-users";
    let expected = Coordinate::new(
        "github.com".into(),
        "coralogix".into(),
        "cx-api-users".into(),
        Protocol::Https,
    );
    assert_eq!(
        Coordinate::from_url(str, Protocol::Https).unwrap(),
        expected
    );
}

#[test]
fn build_coordinate_slash() {
    let str = "github.com/coralogix/cx-api-users/";
    let expected = Coordinate::new(
        "github.com".into(),
        "coralogix".into(),
        "cx-api-users".into(),
        Protocol::Https,
    );
    assert_eq!(
        Coordinate::from_url(str, Protocol::Https).unwrap(),
        expected
    );
}