use std::fmt;

/// Backend-agnostic representation of a git object ID (commit hash).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GitOid {
    hex: String,
}

impl GitOid {
    pub fn from_hex(hex: impl Into<String>) -> Self {
        Self { hex: hex.into() }
    }

    pub fn as_str(&self) -> &str {
        &self.hex
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex)
    }
}

impl From<git2::Oid> for GitOid {
    fn from(oid: git2::Oid) -> Self {
        Self {
            hex: oid.to_string(),
        }
    }
}
