mod git;

use crate::model::protofetch::{Coordinate, DependencyName, Descriptor, RevisionSpecification};

pub trait ModuleResolver {
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        name: &DependencyName,
    ) -> anyhow::Result<ResolvedModule>;
}

#[derive(Clone)]
pub struct ResolvedModule {
    pub commit_hash: String,
    pub descriptor: Descriptor,
}
