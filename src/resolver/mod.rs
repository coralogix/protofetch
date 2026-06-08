mod git;
mod lock;

use std::sync::Arc;

use crate::model::protofetch::{
    Coordinate, DependencyRoot, Descriptor, ModuleName, RevisionSpecification,
};

pub use lock::LockFileModuleResolver;

pub trait ModuleResolver: Send + Sync {
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: Option<&str>,
        name: &ModuleName,
        root: Option<&DependencyRoot>,
    ) -> anyhow::Result<CommitAndDescriptor>;
}

#[derive(Clone)]
pub struct CommitAndDescriptor {
    pub commit_hash: String,
    pub descriptor: Descriptor,
}

impl<T> ModuleResolver for &T
where
    T: ModuleResolver + ?Sized,
{
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: Option<&str>,
        name: &ModuleName,
        root: Option<&DependencyRoot>,
    ) -> anyhow::Result<CommitAndDescriptor> {
        T::resolve(self, coordinate, specification, commit_hash, name, root)
    }
}

impl<T> ModuleResolver for Arc<T>
where
    T: ModuleResolver + ?Sized,
{
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: Option<&str>,
        name: &ModuleName,
        root: Option<&DependencyRoot>,
    ) -> anyhow::Result<CommitAndDescriptor> {
        T::resolve(self, coordinate, specification, commit_hash, name, root)
    }
}
