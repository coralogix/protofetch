pub mod lock;

use derive_new::new;
use regex::Regex;
use serde::{de::Visitor, Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    path::{Path, PathBuf},
};
use strum::EnumString;

use crate::model::ParseError;
use log::{debug, error};
use std::{collections::BTreeSet, hash::Hash};
use toml::{map::Map, Value};

#[derive(PartialEq, Eq, Hash, Clone, Serialize, Deserialize, Ord, PartialOrd)]
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
    Default, PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize, Ord, PartialOrd, EnumString,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Ord, PartialOrd)]
pub enum Revision {
    Pinned {
        revision: String,
    },
    #[default]
    Arbitrary,
}

impl Revision {
    pub fn pinned(revision: impl Into<String>) -> Revision {
        Revision::Pinned {
            revision: revision.into(),
        }
    }

    fn is_arbitrary(&self) -> bool {
        self == &Self::Arbitrary
    }
}

impl Serialize for Revision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Revision::Pinned { revision } => serializer.serialize_str(revision),
            Revision::Arbitrary => serializer.serialize_unit(),
        }
    }
}

impl<'de> Deserialize<'de> for Revision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RevisionVisitor;

        impl<'de> Visitor<'de> for RevisionVisitor {
            type Value = Revision;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Revision::Arbitrary)
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Revision::pinned(v))
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Revision::pinned(v))
            }
        }

        deserializer.deserialize_any(RevisionVisitor)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RevisionSpecification {
    #[serde(skip_serializing_if = "Revision::is_arbitrary", default)]
    pub revision: Revision,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub branch: Option<String>,
}

#[derive(new, Clone, Serialize, Deserialize, Debug, Ord, PartialOrd, PartialEq, Eq, Hash)]
pub struct Rules {
    pub prune: bool,
    pub transitive: bool,
    #[serde(skip_serializing_if = "BTreeSet::is_empty", default)]
    pub content_roots: BTreeSet<ContentRoot>,
    #[serde(skip_serializing_if = "AllowPolicies::is_empty", default)]
    pub allow_policies: AllowPolicies,
    #[serde(skip_serializing_if = "DenyPolicies::is_empty", default)]
    pub deny_policies: DenyPolicies,
}

impl Default for Rules {
    fn default() -> Self {
        Rules::new(
            false,
            false,
            BTreeSet::new(),
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
    policies: BTreeSet<FilePolicy>,
}

impl Default for AllowPolicies {
    fn default() -> Self {
        AllowPolicies::new(BTreeSet::new())
    }
}

impl AllowPolicies {
    pub fn is_empty(allow_policies: &Self) -> bool {
        allow_policies.policies.is_empty()
    }

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
    policies: BTreeSet<FilePolicy>,
}

impl DenyPolicies {
    pub fn is_empty(deny_policies: &Self) -> bool {
        deny_policies.policies.is_empty()
    }

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
        DenyPolicies::new(BTreeSet::new())
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

    pub fn apply_file_policies(
        policies: &BTreeSet<FilePolicy>,
        paths: &Vec<PathBuf>,
    ) -> Vec<PathBuf> {
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

#[derive(new, Debug, PartialEq, PartialOrd, Ord, Eq)]
pub struct Dependency {
    pub name: DependencyName,
    pub coordinate: Coordinate,
    pub specification: RevisionSpecification,
    pub rules: Rules,
}

#[derive(new, PartialEq, Debug, PartialOrd, Ord, Eq)]
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

    pub fn into_toml(self) -> Value {
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
            if let Revision::Pinned { revision } = d.specification.revision {
                dependency.insert("revision".to_owned(), Value::String(revision));
            }
            if let Some(branch) = d.specification.branch {
                dependency.insert("branch".to_owned(), Value::String(branch));
            }
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
        .and_then(|url| Coordinate::from_url(&url, protocol))?;

    let revision = match value.get("revision") {
        Some(revision) => parse_revision(revision)?,
        None => Revision::Arbitrary,
    };

    let specification = RevisionSpecification { revision, branch };

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
        .collect::<BTreeSet<_>>();

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
        specification,
        rules,
    })
}

fn parse_policies(toml: &Value, source: &str) -> Result<BTreeSet<FilePolicy>, ParseError> {
    toml.get(source)
        .map(|v| v.clone().try_into::<Vec<String>>())
        .map_or(Ok(None), |v| v.map(Some))?
        .unwrap_or_default()
        .into_iter()
        .map(|s| FilePolicy::try_from_str(&s))
        .collect::<Result<BTreeSet<_>, _>>()
}

fn parse_revision(value: &toml::Value) -> Result<Revision, ParseError> {
    let revstring = value.clone().try_into::<String>()?;

    Ok(Revision::Pinned {
        revision: revstring,
    })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use pretty_assertions::assert_eq;

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
                },
                specification: RevisionSpecification {
                    revision: Revision::pinned("1.0.0"),
                    branch: None,
                },
                rules: Default::default(),
            }],
        };
        assert_eq!(Descriptor::from_toml_str(str).unwrap(), expected);
    }

    #[test]
    fn load_valid_file_no_revision() {
        let str = r#"
            name = "test_file"
            description = "this is a description"
            proto_out_dir= "./path/to/proto_out"
            [dependency1]
                protocol = "https"
                url = "github.com/org/repo"
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
                },
                specification: RevisionSpecification {
                    revision: Revision::Arbitrary,
                    branch: None,
                },
                rules: Default::default(),
            }],
        };
        assert_eq!(Descriptor::from_toml_str(str).unwrap(), expected);
        assert_eq!(expected.into_toml(), toml::Value::from_str(str).unwrap())
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
                },
                specification: RevisionSpecification {
                    revision: Revision::pinned("1.0.0"),
                    branch: None,
                },
                rules: Rules {
                    prune: true,
                    content_roots: BTreeSet::from([ContentRoot::from_string("src")]),
                    transitive: false,
                    allow_policies: AllowPolicies::new(BTreeSet::from([
                        FilePolicy::new(PolicyKind::File, PathBuf::from("/foo/proto/file.proto")),
                        FilePolicy::new(PolicyKind::Prefix, PathBuf::from("/foo/other")),
                        FilePolicy::new(PolicyKind::SubPath, PathBuf::from("/some/path")),
                    ])),
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
        let expected = Descriptor {
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
                    },
                    specification: RevisionSpecification {
                        revision: Revision::pinned("1.0.0"),
                        branch: None,
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
                    },
                    specification: RevisionSpecification {
                        revision: Revision::pinned("2.0.0"),
                        branch: None,
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
                    },
                    specification: RevisionSpecification {
                        revision: Revision::pinned("3.0.0"),
                        branch: None,
                    },
                    rules: Default::default(),
                },
            ],
        };

        let mut res = Descriptor::from_toml_str(str).unwrap().dependencies;
        res.sort();

        let mut exp = expected.dependencies;
        exp.sort();

        assert_eq!(res, exp);
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
        assert_eq!(expected.into_toml(), toml::Value::from_str(str).unwrap())
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
        assert_eq!(
            Coordinate::from_url(str, Protocol::Https).unwrap(),
            Coordinate {
                forge: "github.com".to_owned(),
                organization: "coralogix".to_owned(),
                repository: "cx-api-users".to_owned(),
                protocol: Protocol::Https,
            }
        );
    }

    #[test]
    fn build_coordinate_slash() {
        let str = "github.com/coralogix/cx-api-users/";
        assert_eq!(
            Coordinate::from_url(str, Protocol::Https).unwrap(),
            Coordinate {
                forge: "github.com".to_owned(),
                organization: "coralogix".to_owned(),
                repository: "cx-api-users".to_owned(),
                protocol: Protocol::Https,
            }
        );
    }

    #[test]
    fn test_allow_policies_rule_filter() {
        let rules = AllowPolicies::new(BTreeSet::from([
            FilePolicy::try_from_str("/foo/proto/file.proto").unwrap(),
            FilePolicy::try_from_str("/foo/other/*").unwrap(),
            FilePolicy::try_from_str("*/path/*").unwrap(),
        ]));

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
        let rules = AllowPolicies::new(BTreeSet::from([
            FilePolicy::try_from_str("/foo/proto/file.proto").unwrap(),
            FilePolicy::try_from_str("/foo/other/*").unwrap(),
            FilePolicy::try_from_str("*/path/*").unwrap(),
        ]));

        let path = vec![
            PathBuf::from("foo/proto/file.proto"),
            PathBuf::from("foo/other/file2.proto"),
        ];

        let res = AllowPolicies::filter(&rules, &path);
        assert_eq!(res.len(), 2);
    }

    #[test]
    fn test_allow_policies_rule_filter_edge_case_slash_rule() {
        let allow_policies = AllowPolicies::new(BTreeSet::from([
            FilePolicy::try_from_str("foo/proto/file.proto").unwrap(),
            FilePolicy::try_from_str("foo/other/*").unwrap(),
            FilePolicy::try_from_str("*/path/*").unwrap(),
        ]));

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
        let rules = DenyPolicies::new(BTreeSet::from([
            FilePolicy::try_from_str("/foo/proto/file.proto").unwrap(),
            FilePolicy::try_from_str("/foo/other/*").unwrap(),
            FilePolicy::try_from_str("*/path/*").unwrap(),
        ]));

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
        let rules = DenyPolicies::new(BTreeSet::from([
            FilePolicy::try_from_str("/foo/proto/file.proto").unwrap(),
            FilePolicy::try_from_str("/foo/other/*").unwrap(),
            FilePolicy::try_from_str("*/path/*").unwrap(),
        ]));

        let file = PathBuf::from("/foo/proto/file.proto");

        let res = DenyPolicies::should_deny_file(&rules, &file);
        assert!(res);
    }
}
