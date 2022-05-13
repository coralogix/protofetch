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

#[derive(
    new, SmartDefault, PartialEq, Eq, Hash, Clone, Serialize, Deserialize, Ord, PartialOrd,
)]
pub struct Coordinate {
    pub forge: String,
    pub organization: String,
    pub repository: String,
    pub protocol: Protocol,
    #[default(None)]
    pub branch: Option<String>,
}

impl Coordinate {
    pub fn from_url(
        url: &str,
        protocol: Protocol,
        branch: Option<String>,
    ) -> Result<Coordinate, ParseError> {
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
            branch,
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
    SmartDefault,
    PartialEq,
    Eq,
    Hash,
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Ord,
    PartialOrd,
    EnumString,
)]
pub enum Protocol {
    #[serde(rename = "https")]
    #[strum(ascii_case_insensitive)]
    Https,
    #[serde(rename = "ssh")]
    #[strum(ascii_case_insensitive)]
    #[default]
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

#[derive(new, Clone, Serialize, Deserialize, Debug, Ord, PartialOrd, PartialEq, Eq, Hash)]
pub struct Rules {
    pub prune: bool,
    pub transitive: bool,
    pub content_roots: Vec<ContentRoot>,
    pub allow_policies: AllowPolicies,
    pub deny_policies: DenyPolicies,
}

impl Default for Rules {
    fn default() -> Self {
        Rules::new(
            false,
            false,
            vec![],
            AllowPolicies::default(),
            DenyPolicies::default(),
        )
    }
}

/// A content root path for a repository.
#[derive(new, Ord, PartialOrd, PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize)]
pub struct ContentRoot {
    pub value: PathBuf,
}

impl ContentRoot {
    pub fn from_string(s: &str) -> ContentRoot {
        let path = PathBuf::from(s);
        let path = path.strip_prefix("/").unwrap_or(&path).to_path_buf();
        ContentRoot::new(path)
    }
}

#[derive(new, Ord, PartialOrd, PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize)]
pub struct AllowPolicies {
    policies: Vec<FilePolicy>,
}

impl Default for AllowPolicies {
    fn default() -> Self {
        AllowPolicies::new(vec![])
    }
}

impl AllowPolicies {
    pub fn should_allow_file(allow_policies: &Self, file: &Path) -> bool {
        if allow_policies.policies.is_empty() {
            true
        } else {
            !Self::filter(allow_policies, &vec![file.to_path_buf()]).is_empty()
        }
    }

    pub fn filter(allow_policies: &Self, paths: &Vec<PathBuf>) -> Vec<PathBuf> {
        FilePolicy::apply_file_policies(&allow_policies.policies, paths)
    }
}

#[derive(new, Ord, PartialOrd, PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize)]
pub struct DenyPolicies {
    policies: Vec<FilePolicy>,
}

impl DenyPolicies {
    pub fn deny_files(deny_policies: &Self, files: &Vec<PathBuf>) -> Vec<PathBuf> {
        if deny_policies.policies.is_empty() {
            files.clone()
        } else {
            let filtered = FilePolicy::apply_file_policies(&deny_policies.policies, files);
            files
                .iter()
                .cloned()
                .filter(|f| !filtered.contains(f))
                .collect()
        }
    }

    pub fn should_deny_file(deny_policies: &Self, file: &Path) -> bool {
        if deny_policies.policies.is_empty() {
            false
        } else {
            Self::deny_files(deny_policies, &vec![file.to_path_buf()]).is_empty()
        }
    }
}

impl Default for DenyPolicies {
    fn default() -> Self {
        DenyPolicies::new(vec![])
    }
}

#[derive(new, Ord, PartialOrd, PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize)]
/// Describes a policy to filter files or directories based on a policy kind and a path.
/// The field kind is necessary due to a limitation in toml serialization.
pub struct FilePolicy {
    pub kind: PolicyKind,
    pub path: PathBuf,
}

impl FilePolicy {
    pub fn try_from_str(s: &str) -> Result<Self, ParseError> {
        if s.starts_with("*/") && s.ends_with("/*") {
            Ok(FilePolicy::new(
                PolicyKind::SubPath,
                PathBuf::from(
                    s.strip_prefix('*')
                        .unwrap()
                        .strip_suffix("/*")
                        .unwrap()
                        .to_string(),
                ),
            ))
        } else if s.ends_with("/*") {
            let path = PathBuf::from(s.strip_suffix("/*").unwrap());
            let path = Self::add_leading_slash(&path);
            Ok(FilePolicy::new(PolicyKind::Prefix, path))
        } else if s.ends_with(".proto") {
            let path = Self::add_leading_slash(&PathBuf::from(s));
            Ok(FilePolicy::new(PolicyKind::File, path))
        } else {
            Err(ParseError::ParsePolicyRuleError(s.to_string()))
        }
    }

    fn add_leading_slash(p: &Path) -> PathBuf {
        if !p.starts_with("/") {
            PathBuf::from(format!("/{}", p.to_string_lossy()))
        } else {
            p.to_path_buf()
        }
    }

    pub fn apply_file_policies(policies: &Vec<FilePolicy>, paths: &Vec<PathBuf>) -> Vec<PathBuf> {
        if policies.is_empty() {
            return paths.clone();
        }
        let mut result = Vec::new();
        for path in paths {
            let path = Self::add_leading_slash(path);
            for policy in policies {
                match policy.kind {
                    PolicyKind::File => {
                        if path == policy.path {
                            result.push(path.clone());
                        }
                    }
                    PolicyKind::Prefix => {
                        if path.starts_with(&policy.path) {
                            result.push(path.clone());
                        }
                    }
                    PolicyKind::SubPath => {
                        if path
                            .to_string_lossy()
                            .contains(&policy.path.to_string_lossy().to_string())
                        {
                            result.push(path.clone());
                        }
                    }
                }
            }
        }
        result
    }
}

#[derive(Ord, PartialOrd, PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize)]
pub enum PolicyKind {
    /// /path/to/file.proto
    File,
    /// /prefix/*
    Prefix,
    /// */subpath/*
    SubPath,
}

#[derive(new, Clone, Hash, Deserialize, Serialize, Debug, PartialEq, Eq, Ord, PartialOrd)]
pub struct DependencyName {
    pub value: String,
}

#[derive(new, Serialize, Debug, PartialEq, PartialOrd, Ord, Eq)]
pub struct Dependency {
    pub name: DependencyName,
    pub coordinate: Coordinate,
    pub revision: Revision,
    pub rules: Rules,
}

#[derive(new, Serialize, PartialEq, Debug, PartialOrd, Ord, Eq)]
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
            description.insert(d.name.value, Value::Table(dependency));
        }
        Value::Table(description)
    }
}

fn parse_dependency(name: String, value: &toml::Value) -> Result<Dependency, ParseError> {
    let protocol = match value.get("protocol") {
        None => Protocol::Https,
        Some(toml) => toml.clone().try_into::<Protocol>()?,
    };

    let name = DependencyName::new(name);

    let branch = value
        .get("branch")
        .map(|v| v.clone().try_into::<String>())
        .map_or(Ok(None), |v| v.map(Some))?;

    let coordinate = value
        .get("url")
        .ok_or_else(|| ParseError::MissingKey("url".to_string()))
        .and_then(|x| x.clone().try_into::<String>().map_err(|e| e.into()))
        .and_then(|url| Coordinate::from_url(&url, protocol, branch))?;

    let revision = parse_revision(
        value
            .get("revision")
            .ok_or_else(|| ParseError::MissingKey("revision".to_string()))?,
    )?;

    let prune = value
        .get("prune")
        .map(|v| v.clone().try_into::<bool>())
        .map_or(Ok(None), |v| v.map(Some))?
        .unwrap_or(false);

    let content_roots = value
        .get("content_roots")
        .map(|v| v.clone().try_into::<Vec<String>>())
        .map_or(Ok(None), |v| v.map(Some))?
        .unwrap_or_default()
        .into_iter()
        .map(|str| ContentRoot::from_string(&str))
        .collect::<Vec<_>>();

    let transitive = value
        .get("transitive")
        .map(|v| v.clone().try_into::<bool>())
        .map_or(Ok(None), |v| v.map(Some))?
        .unwrap_or(false);

    let allow_policies = AllowPolicies::new(parse_policies(value, "allow_policies")?);
    let deny_policies = DenyPolicies::new(parse_policies(value, "deny_policies")?);

    let rules = Rules::new(
        prune,
        transitive,
        content_roots,
        allow_policies,
        deny_policies,
    );

    Ok(Dependency {
        name,
        coordinate,
        revision,
        rules,
    })
}

fn parse_policies(toml: &Value, source: &str) -> Result<Vec<FilePolicy>, ParseError> {
    toml.get(source)
        .map(|v| v.clone().try_into::<Vec<String>>())
        .map_or(Ok(None), |v| v.map(Some))?
        .unwrap_or_default()
        .into_iter()
        .map(|s| FilePolicy::try_from_str(&s))
        .collect::<Result<Vec<_>, _>>()
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

#[derive(Hash, Eq, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LockedDependency {
    pub name: DependencyName,
    pub commit_hash: String,
    pub coordinate: Coordinate,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencyName>,
    pub rules: Rules,
}

#[test]
fn load_valid_file_one_dep() {
    let str = r#"
name = "test_file"
description = "this is a description"
proto_out_dir= "./path/to/proto_out"
[dependency1]
  protocol = "https"
  url = "github.com/org/repo"
  revision = "1.0.0"
"#;
    let expected = Descriptor {
        name: "test_file".to_string(),
        description: Some("this is a description".to_string()),
        proto_out_dir: Some("./path/to/proto_out".to_string()),
        dependencies: vec![Dependency {
            name: DependencyName::new("dependency1".to_string()),
            coordinate: Coordinate {
                forge: "github.com".to_string(),
                organization: "org".to_string(),
                repository: "repo".to_string(),
                protocol: Protocol::Https,
                branch: None,
            },
            revision: Revision::Arbitrary {
                revision: "1.0.0".to_string(),
            },
            rules: Default::default(),
        }],
    };
    assert_eq!(Descriptor::from_toml_str(str).unwrap(), expected);
}

#[test]
fn load_valid_file_one_dep_with_rules() {
    let str = r#"
name = "test_file"
description = "this is a description"
proto_out_dir= "./path/to/proto_out"
[dependency1]
  protocol = "https"
  url = "github.com/org/repo"
  revision = "1.0.0"
  prune = true
  content_roots = ["src"]
  allow_policies = ["/foo/proto/file.proto", "/foo/other/*", "*/some/path/*"]
"#;
    let expected = Descriptor {
        name: "test_file".to_string(),
        description: Some("this is a description".to_string()),
        proto_out_dir: Some("./path/to/proto_out".to_string()),
        dependencies: vec![Dependency {
            name: DependencyName::new("dependency1".to_string()),
            coordinate: Coordinate {
                forge: "github.com".to_string(),
                organization: "org".to_string(),
                repository: "repo".to_string(),
                protocol: Protocol::Https,
                branch: None,
            },
            revision: Revision::Arbitrary {
                revision: "1.0.0".to_string(),
            },
            rules: Rules {
                prune: true,
                content_roots: vec![ContentRoot::from_string("src")],
                transitive: false,
                allow_policies: AllowPolicies::new(vec![
                    FilePolicy::new(PolicyKind::File, PathBuf::from("/foo/proto/file.proto")),
                    FilePolicy::new(PolicyKind::Prefix, PathBuf::from("/foo/other")),
                    FilePolicy::new(PolicyKind::SubPath, PathBuf::from("/some/path")),
                ]),
                deny_policies: DenyPolicies::default(),
            },
        }],
    };
    assert_eq!(Descriptor::from_toml_str(str).unwrap(), expected);
}

#[test]
#[should_panic]
fn load_invalid_file_invalid_rule() {
    let str = r#"
name = "test_file"
description = "this is a description"
proto_out_dir= "./path/to/proto_out"
[dependency1]
  protocol = "https"
  url = "github.com/org/repo"
  revision = "1.0.0"
  prune = true
  content_roots = ["src"]
  allow_policies = ["/foo/proto/file.java"]
"#;
    Descriptor::from_toml_str(str).unwrap();
}

#[test]
fn load_valid_file_multiple_dep() {
    let str = r#"
name = "test_file"
proto_out_dir= "./path/to/proto_out"

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
        proto_out_dir: Some("./path/to/proto_out".to_string()),
        dependencies: vec![
            Dependency {
                name: DependencyName::new("dependency1".to_string()),
                coordinate: Coordinate {
                    forge: "github.com".to_string(),
                    organization: "org".to_string(),
                    repository: "repo".to_string(),
                    protocol: Protocol::Https,
                    branch: None,
                },
                revision: Revision::Arbitrary {
                    revision: "1.0.0".to_string(),
                },
                rules: Default::default(),
            },
            Dependency {
                name: DependencyName::new("dependency2".to_string()),
                coordinate: Coordinate {
                    forge: "github.com".to_string(),
                    organization: "org".to_string(),
                    repository: "repo".to_string(),
                    protocol: Protocol::Https,
                    branch: None,
                },
                revision: Revision::Arbitrary {
                    revision: "2.0.0".to_string(),
                },
                rules: Default::default(),
            },
            Dependency {
                name: DependencyName::new("dependency3".to_string()),
                coordinate: Coordinate {
                    forge: "github.com".to_string(),
                    organization: "org".to_string(),
                    repository: "repo".to_string(),
                    protocol: Protocol::Https,
                    branch: None,
                },
                revision: Revision::Arbitrary {
                    revision: "3.0.0".to_string(),
                },
                rules: Default::default(),
            },
        ],
    };
    // TODO this tests nothing
    assert_eq!(
        Descriptor::from_toml_str(str).unwrap().dependencies.sort(),
        expected.dependencies.sort()
    );
}

#[test]
fn load_file_no_deps() {
    let str = r#"
    name = "test_file"
    proto_out_dir = "./path/to/proto_out"
    "#;
    let expected = Descriptor {
        name: "test_file".to_string(),
        description: None,
        proto_out_dir: Some("./path/to/proto_out".to_string()),
        dependencies: vec![],
    };
    assert_eq!(Descriptor::from_toml_str(str).unwrap(), expected);
}

#[test]
fn load_invalid_protocol() {
    let str = r#"
name = "test_file"
proto_out_dir = "./path/to/proto_out"
[dependency1]
  protocol = "ftp"
  url = "github.com/org/repo"
  revision = "1.0.0"
"#;
    assert!(Descriptor::from_toml_str(str).is_err());
}

#[test]
fn load_invalid_url() {
    let str = r#"
name = "test_file"
proto_out_dir = "./path/to/proto_out"
[dependency1]
  protocol = "ftp"
  url = "github.com/org"
  revision = "1.0.0"
"#;
    assert!(Descriptor::from_toml_str(str).is_err());
}

#[test]
fn build_coordinate() {
    let str = "github.com/coralogix/cx-api-users";
    let expected = Coordinate::new(
        "github.com".into(),
        "coralogix".into(),
        "cx-api-users".into(),
        Protocol::Https,
        None,
    );
    assert_eq!(
        Coordinate::from_url(str, Protocol::Https, None).unwrap(),
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
        None,
    );
    assert_eq!(
        Coordinate::from_url(str, Protocol::Https, None).unwrap(),
        expected
    );
}

#[test]
fn test_allow_policies_rule_filter() {
    let rules = AllowPolicies::new(vec![
        FilePolicy::try_from_str("/foo/proto/file.proto").unwrap(),
        FilePolicy::try_from_str("/foo/other/*").unwrap(),
        FilePolicy::try_from_str("*/path/*").unwrap(),
    ]);

    let path = vec![
        PathBuf::from("/foo/proto/file.proto"),
        PathBuf::from("/foo/other/file1.proto"),
        PathBuf::from("/some/path/file.proto"),
    ];

    let res = AllowPolicies::filter(&rules, &path);
    assert_eq!(res.len(), 3);
}

#[test]
fn test_allow_policies_rule_filter_edge_case_slash_path() {
    let rules = AllowPolicies::new(vec![
        FilePolicy::try_from_str("/foo/proto/file.proto").unwrap(),
        FilePolicy::try_from_str("/foo/other/*").unwrap(),
        FilePolicy::try_from_str("*/path/*").unwrap(),
    ]);

    let path = vec![
        PathBuf::from("foo/proto/file.proto"),
        PathBuf::from("foo/other/file2.proto"),
    ];

    let res = AllowPolicies::filter(&rules, &path);
    assert_eq!(res.len(), 2);
}

#[test]
fn test_allow_policies_rule_filter_edge_case_slash_rule() {
    let allow_policies = AllowPolicies::new(vec![
        FilePolicy::try_from_str("foo/proto/file.proto").unwrap(),
        FilePolicy::try_from_str("foo/other/*").unwrap(),
        FilePolicy::try_from_str("*/path/*").unwrap(),
    ]);

    let files = vec![
        PathBuf::from("/foo/proto/file.proto"),
        PathBuf::from("/foo/other/file2.proto"),
        PathBuf::from("/path/dep/file3.proto"),
    ];

    let res = AllowPolicies::filter(&allow_policies, &files);
    assert_eq!(res.len(), 3);
}

#[test]
fn test_deny_policies_rule_filter() {
    let rules = DenyPolicies::new(vec![
        FilePolicy::try_from_str("/foo/proto/file.proto").unwrap(),
        FilePolicy::try_from_str("/foo/other/*").unwrap(),
        FilePolicy::try_from_str("*/path/*").unwrap(),
    ]);

    let files = vec![
        PathBuf::from("/foo/proto/file.proto"),
        PathBuf::from("/foo/other/file1.proto"),
        PathBuf::from("/some/path/file.proto"),
    ];

    let res = DenyPolicies::deny_files(&rules, &files);
    assert_eq!(res.len(), 0);
}

#[test]
fn test_deny_policies_rule_filter_file() {
    let rules = DenyPolicies::new(vec![
        FilePolicy::try_from_str("/foo/proto/file.proto").unwrap(),
        FilePolicy::try_from_str("/foo/other/*").unwrap(),
        FilePolicy::try_from_str("*/path/*").unwrap(),
    ]);

    let file = PathBuf::from("/foo/proto/file.proto");

    let res = DenyPolicies::should_deny_file(&rules, &file);
    assert!(res);
}
