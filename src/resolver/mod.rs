mod git;
mod lock;

use crate::model::protofetch::{Coordinate, DependencyName, Descriptor, RevisionSpecification};

pub use lock::LockFileModuleResolver;

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

impl<T> ModuleResolver for &T
where
    T: ModuleResolver,
{
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        name: &DependencyName,
    ) -> anyhow::Result<ResolvedModule> {
        T::resolve(self, coordinate, specification, name)
    }
}
