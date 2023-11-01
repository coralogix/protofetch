use std::collections::BTreeSet;

use super::{Coordinate, DependencyName, RevisionSpecification, Rules};

pub struct ResolvedModule {
    pub module_name: String,
    pub dependencies: Vec<ResolvedDependency>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResolvedDependency {
    pub name: DependencyName,
    pub commit_hash: String,
    pub coordinate: Coordinate,
    pub specification: RevisionSpecification,
    pub rules: Rules,
    pub dependencies: BTreeSet<DependencyName>,
}
