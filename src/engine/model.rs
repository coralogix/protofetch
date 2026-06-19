use crate::model::protofetch::{Coordinate, ModuleName, RevisionSpecification, Rules};

pub struct ResolvedRootModule {
    pub modules: Vec<ResolvedModule>,
    pub dependencies: Vec<ResolvedDependency>,
}

pub struct ResolvedModule {
    pub name: ModuleName,
    pub commit_hash: String,
    pub coordinate: Coordinate,
    pub specification: RevisionSpecification,
    pub dependencies: Vec<ResolvedDependency>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResolvedDependency {
    pub name: ModuleName,
    pub rules: Rules,
}
