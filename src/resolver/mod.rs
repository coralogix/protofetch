mod git;
mod lock;

use crate::model::protofetch::{Coordinate, DependencyName, Descriptor, RevisionSpecification};

pub use lock::LockFileModuleResolver;

pub trait ModuleResolver {
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: Option<&str>,
        name: &DependencyName,
    ) -> anyhow::Result<CommitAndDescriptor>;
}

#[derive(Clone)]
pub struct CommitAndDescriptor {
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
        commit_hash: Option<&str>,
        name: &DependencyName,
    ) -> anyhow::Result<CommitAndDescriptor> {
        T::resolve(self, coordinate, specification, commit_hash, name)
    }
}
