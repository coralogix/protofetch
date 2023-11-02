use anyhow::bail;
use log::debug;

use crate::model::protofetch::{
    lock::{LockFile, LockedCoordinate},
    Coordinate, ModuleName, RevisionSpecification,
};

use super::{CommitAndDescriptor, ModuleResolver};

pub struct LockFileModuleResolver<'a, R> {
    inner: R,
    lock_file: &'a LockFile,
    locked: bool,
}

impl<'a, R> LockFileModuleResolver<'a, R> {
    pub fn new(inner: R, lock_file: &'a LockFile, locked: bool) -> Self {
        Self {
            inner,
            lock_file,
            locked,
        }
    }
}

impl<'a, R> ModuleResolver for LockFileModuleResolver<'a, R>
where
    R: ModuleResolver,
{
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: Option<&str>,
        name: &ModuleName,
    ) -> anyhow::Result<CommitAndDescriptor> {
        let locked_coordinate = LockedCoordinate::from(coordinate);
        let dependency = self.lock_file.dependencies.iter().find(|dependency| {
            dependency.coordinate == locked_coordinate && &dependency.specification == specification
        });
        match dependency {
            Some(dependency) => {
                debug!(
                    "Dependency {} {} found in the lock file with commit {}",
                    coordinate, specification, dependency.commit_hash
                );
                let resolved = self.inner.resolve(
                    coordinate,
                    specification,
                    commit_hash.or(Some(&dependency.commit_hash)),
                    name,
                )?;
                if resolved.commit_hash != dependency.commit_hash {
                    bail!(
                        "Commit hash of {} {} changed: the lock file specifies {}, but the actual commit hash is {}",
                        coordinate,
                        specification,
                        dependency.commit_hash,
                        resolved.commit_hash
                    );
                }
                Ok(resolved)
            }
            None if self.locked => {
                bail!(
                    "No entry for {} {} in the lock file",
                    coordinate,
                    specification
                );
            }
            None => {
                debug!(
                    "Dependency {} {} not found in the lock file",
                    coordinate, specification
                );
                self.inner
                    .resolve(coordinate, specification, commit_hash, name)
            }
        }
    }
}
