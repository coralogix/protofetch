use anyhow::bail;
use log::debug;

use crate::model::protofetch::{lock::LockFile, Coordinate, DependencyName, RevisionSpecification};

use super::{ModuleResolver, ResolvedModule};

pub struct LockFileModuleResolver<R> {
    inner: R,
    lock_file: LockFile,
    locked: bool,
}

impl<R> LockFileModuleResolver<R> {
    pub fn new(inner: R, lock_file: LockFile, locked: bool) -> Self {
        Self {
            inner,
            lock_file,
            locked,
        }
    }
}

impl<R> ModuleResolver for LockFileModuleResolver<R>
where
    R: ModuleResolver,
{
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        name: &DependencyName,
    ) -> anyhow::Result<ResolvedModule> {
        let dependency = self.lock_file.dependencies.iter().find(|dependency| {
            &dependency.coordinate == coordinate && &dependency.specification == specification
        });
        match dependency {
            Some(dependency) => {
                debug!(
                    "Dependency {} {} found in the lock file with commit {}",
                    coordinate, specification, dependency.commit_hash
                );
                let commit_hash = dependency.commit_hash.clone();
                let resolved = self.inner.resolve(coordinate, specification, name)?;
                if resolved.commit_hash != commit_hash {
                    bail!("Commit hash of {} {} changed: the lock file specifies {}, but the actual commit hash is {}", coordinate, specification, commit_hash, resolved.commit_hash);
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
                self.inner.resolve(coordinate, specification, name)
            }
        }
    }
}
