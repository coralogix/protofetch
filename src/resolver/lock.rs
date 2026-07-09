use std::collections::BTreeMap;

use anyhow::bail;
use log::debug;

use crate::model::protofetch::{
    lock::{LockFile, LockedCoordinate},
    Coordinate, ModuleName, RevisionSpecification,
};

use super::{CommitAndDescriptor, ModuleResolver};

pub struct LockFileModuleResolver<R> {
    inner: R,
    lock_file: LockFile,
    locked: bool,
    updates: BTreeMap<String, Option<String>>,
}

impl<R> LockFileModuleResolver<R> {
    pub fn new(inner: R, lock_file: LockFile, locked: bool) -> Self {
        Self {
            inner,
            lock_file,
            locked,
            updates: BTreeMap::new(),
        }
    }

    pub fn new_selected(
        inner: R,
        lock_file: LockFile,
        updates: BTreeMap<String, Option<String>>,
    ) -> Self {
        Self {
            inner,
            lock_file,
            locked: false,
            updates,
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
        commit_hash: Option<&str>,
        name: &ModuleName,
    ) -> anyhow::Result<CommitAndDescriptor> {
        if let Some(precise) = self.updates.get(&name.to_string()) {
            debug!("Dependency {} selected for update", name);
            return self.inner.resolve(
                coordinate,
                specification,
                precise.as_deref().or(commit_hash),
                name,
            );
        }

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        model::protofetch::{
            lock::{LockFile, LockedCoordinate, LockedDependency},
            Coordinate, Descriptor, ModuleName, RevisionSpecification,
        },
        resolver::{CommitAndDescriptor, LockFileModuleResolver, ModuleResolver},
    };

    struct FakeResolver;

    impl ModuleResolver for FakeResolver {
        fn resolve(
            &self,
            _: &Coordinate,
            _: &RevisionSpecification,
            commit_hash: Option<&str>,
            name: &ModuleName,
        ) -> anyhow::Result<CommitAndDescriptor> {
            Ok(CommitAndDescriptor {
                commit_hash: commit_hash.unwrap_or("fresh").to_owned(),
                descriptor: Descriptor {
                    name: name.clone(),
                    description: None,
                    proto_out_dir: None,
                    dependencies: Vec::new(),
                },
            })
        }
    }

    fn coordinate() -> Coordinate {
        Coordinate::from_url("example.com/org/repo").unwrap()
    }

    fn lock_file() -> LockFile {
        LockFile {
            dependencies: vec![LockedDependency {
                name: ModuleName::from("repo"),
                coordinate: LockedCoordinate::from(&coordinate()),
                specification: RevisionSpecification::default(),
                commit_hash: "locked".to_owned(),
            }],
        }
    }

    #[test]
    fn unselected_dependency_uses_lock_file() {
        let resolver = LockFileModuleResolver::new_selected(
            FakeResolver,
            lock_file(),
            BTreeMap::from([("other".to_owned(), None)]),
        );

        let resolved = resolver
            .resolve(
                &coordinate(),
                &RevisionSpecification::default(),
                None,
                &ModuleName::from("repo"),
            )
            .unwrap();

        assert_eq!(resolved.commit_hash, "locked");
    }

    #[test]
    fn selected_dependency_bypasses_lock_file() {
        let resolver = LockFileModuleResolver::new_selected(
            FakeResolver,
            lock_file(),
            BTreeMap::from([("repo".to_owned(), None)]),
        );

        let resolved = resolver
            .resolve(
                &coordinate(),
                &RevisionSpecification::default(),
                None,
                &ModuleName::from("repo"),
            )
            .unwrap();

        assert_eq!(resolved.commit_hash, "fresh");
    }

    #[test]
    fn selected_dependency_uses_precise_commit() {
        let resolver = LockFileModuleResolver::new_selected(
            FakeResolver,
            lock_file(),
            BTreeMap::from([("repo".to_owned(), Some("precise".to_owned()))]),
        );

        let resolved = resolver
            .resolve(
                &coordinate(),
                &RevisionSpecification::default(),
                None,
                &ModuleName::from("repo"),
            )
            .unwrap();

        assert_eq!(resolved.commit_hash, "precise");
    }
}
