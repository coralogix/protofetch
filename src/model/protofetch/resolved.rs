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
    pub rules: Rules,
    pub dependencies: BTreeSet<ModuleName>,
}
