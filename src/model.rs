use core::num::ParseIntError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt::Display,
    path::{Path, PathBuf},
};
use thiserror::Error;
use toml::Value;

#[derive(PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize)]
pub struct Coordinate {
    pub forge: String,
    pub organization: String,
    pub repository: String,
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

impl Coordinate {
    pub fn as_path(&self) -> PathBuf {
        let mut result = PathBuf::new();

        result.push(self.forge.clone());
        result.push(self.organization.clone());
        result.push(self.repository.clone());

        result
    }

    pub fn url(&self) -> String {
        format!(
            "https://{}/{}/{}",
            self.forge, self.organization, self.repository
        )
    }
}

#[derive(Debug, Clone)]
pub enum Revision {
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

#[derive(Debug, Clone)]
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

#[derive(Debug)]
pub struct Dependency {
    pub name: String,
    pub coordinate: Coordinate,
    pub revision: Revision,
}

#[derive(Debug)]
pub struct Descriptor {
    pub name: String,
    pub dependencies: Vec<Dependency>,
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("TOML parsing error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Parse error")]
    Parse(#[from] ParseIntError),
    #[error("Missing TOML key `{0}` while parsing")]
    MissingKey(String),
    #[error("Missing url component `{0}` in string `{1}`")]
    MissingUrlComponent(String, String),
}

impl Descriptor {
    pub fn from_file(path: &Path) -> Result<Descriptor, ParseError> {
        let contents = std::fs::read_to_string(path)?;

        Descriptor::from_str(&contents)
    }

    pub fn from_str(data: &str) -> Result<Descriptor, ParseError> {
        let mut toml_value = toml::from_str::<HashMap<String, Value>>(data)?;

        let name = toml_value
            .remove("name")
            .ok_or_else(|| ParseError::MissingKey("name".to_string()))
            .and_then(|v| v.try_into::<String>().map_err(|e| e.into()))?;

        let dependencies = toml_value
            .into_iter()
            .map(|(k, v)| parse_dependency(k, &v))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Descriptor { name, dependencies })
    }
}

fn parse_dependency(name: String, value: &toml::Value) -> Result<Dependency, ParseError> {
    let coordinate = parse_coordinate(
        value
            .get("url")
            .ok_or_else(|| ParseError::MissingKey("url".to_string()))?,
    )?;
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

fn parse_coordinate(value: &toml::Value) -> Result<Coordinate, ParseError> {
    let url = value.clone().try_into::<String>()?;
    let re: Regex =
        Regex::new(r"^(?P<forge>[^/]+)/(?P<organization>[^/]+)/(?P<repository>[^/]+)$").unwrap();
    let url_parse_results = re.captures(&url);
    let url_parse_results = url_parse_results.as_ref();

    Ok(Coordinate {
        forge: url_parse_results
            .and_then(|c| c.name("forge"))
            .map(|s| s.as_str().to_string())
            .ok_or_else(|| ParseError::MissingUrlComponent("forge".to_string(), url.clone()))?,
        organization: url_parse_results
            .and_then(|c| c.name("organization"))
            .map(|s| s.as_str().to_string())
            .ok_or_else(|| {
                ParseError::MissingUrlComponent("organization".to_string(), url.clone())
            })?,
        repository: url_parse_results
            .and_then(|c| c.name("repository"))
            .map(|s| s.as_str().to_string())
            .ok_or_else(|| {
                ParseError::MissingUrlComponent("repository".to_string(), url.clone())
            })?,
    })
}

use lazy_static::lazy_static;

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

fn _parse_semver(revstring: &String) -> Result<Revision, ParseError> {
    let results = SEMVER_REGEX.captures(&revstring);

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFile {
    pub module_name: String,
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
