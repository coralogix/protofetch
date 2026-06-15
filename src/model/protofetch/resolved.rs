use std::collections::BTreeSet;

use super::{Coordinate, ModuleName, RevisionSpecification, Rules};

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
}
