use std::{convert::TryFrom, fmt};

use serde::{Deserialize, Serialize};

use super::ParseError;

/// Backend-agnostic representation of a git object ID (commit hash).
/// Construction validates the 40-hex-digit shape; `from_git` is the
/// trusted path for oid strings that came from git itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GitOid {
    hex: String,
}

impl GitOid {
    /// For oid strings produced by git itself. Callers must not pass
    /// unvalidated input; everything else goes through `TryFrom`.
    pub fn from_git(hex: impl Into<String>) -> Self {
        Self { hex: hex.into() }
    }

    pub fn as_str(&self) -> &str {
        &self.hex
    }
}

impl TryFrom<&str> for GitOid {
    type Error = ParseError;

    fn try_from(hex: &str) -> Result<Self, Self::Error> {
        (hex.len() == 40 && hex.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| GitOid {
                hex: hex.to_owned(),
            })
            .ok_or_else(|| ParseError::InvalidCommitHash(hex.to_owned()))
    }
}

impl TryFrom<String> for GitOid {
    type Error = ParseError;

    fn try_from(hex: String) -> Result<Self, Self::Error> {
        Self::try_from(hex.as_str())
    }
}

impl From<GitOid> for String {
    fn from(oid: GitOid) -> Self {
        oid.hex
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex)
    }
}

#[cfg(test)]
mod tests {
    use super::GitOid;

    #[test]
    fn try_from_rejects_a_non_hex_oid() {
        assert!(GitOid::try_from("not-a-commit").is_err());
        assert!(GitOid::try_from("short").is_err());
    }

    #[test]
    fn try_from_accepts_a_40_hex_digit_oid() {
        let oid = GitOid::try_from("1".repeat(40).as_str()).unwrap();

        assert_eq!(oid.as_str(), "1".repeat(40));
    }
}
