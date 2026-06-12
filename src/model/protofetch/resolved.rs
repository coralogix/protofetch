use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::{
    AllowPolicies, ContentRoot, Coordinate, DenyPolicies, ModuleName, RevisionSpecification, Rules,
};

pub struct ResolvedModule {
    pub module_name: ModuleName,
    pub dependencies: Vec<ResolvedDependency>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResolvedDependency {
    pub name: ModuleName,
    pub commit_hash: String,
    pub coordinate: Coordinate,
    pub specification: RevisionSpecification,
    pub dependencies: BTreeSet<ModuleName>,
    pub rules: Vec<Rules>,
}

impl ResolvedDependency {
    pub fn is_pruned(&self) -> bool {
        !self.rules.is_empty() && self.rules.iter().all(|r| r.prune)
    }

    pub fn is_transitive(&self) -> bool {
        !self.rules.is_empty() && self.rules.iter().all(|r| r.transitive)
    }

    /// Returns true if `path` (pre-zoom, relative to the dep's cache dir) is
    /// accepted by at least one occurrence's (allow ∧ ¬deny) policy after
    /// zooming with that occurrence's content roots.
    /// An empty `rules` means no filtering — allow all.
    pub fn is_file_allowed(&self, path: &Path) -> bool {
        self.rules.is_empty()
            || self
                .rules
                .iter()
                .any(|rules| Self::is_file_allowed_by_rules(rules, path))
    }

    pub fn is_file_allowed_by_rules(rules: &Rules, path: &Path) -> bool {
        let zoomed = zoom_in_content_roots(&rules.content_roots, path);
        AllowPolicies::should_allow_file(&rules.allow_policies, &zoomed)
            && !DenyPolicies::should_deny_file(&rules.deny_policies, &zoomed)
    }
}

fn zoom_in_content_roots(content_roots: &BTreeSet<ContentRoot>, path: &Path) -> PathBuf {
    if content_roots.is_empty() {
        return path.to_path_buf();
    }
    // Reverse iteration gives priority to longer (more specific) content roots.
    content_roots
        .iter()
        .rev()
        .find(|r| path.starts_with(&r.value))
        .and_then(|r| path.strip_prefix(&r.value).ok())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| path.to_path_buf())
}
