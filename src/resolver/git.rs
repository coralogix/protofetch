use crate::{
    cache::{ProtofetchGitCache, RepositoryCache},
    model::protofetch::{Coordinate, DependencyName, Revision, RevisionSpecification},
};

use super::{ModuleResolver, ResolvedModule};

impl ModuleResolver for ProtofetchGitCache {
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: Option<&str>,
        name: &DependencyName,
    ) -> anyhow::Result<ResolvedModule> {
        let repository = self.clone_or_update(coordinate)?;
        let commit_hash = if specification.revision == Revision::Arbitrary {
            if let Some(commit_hash) = commit_hash {
                commit_hash.to_owned()
            } else {
                repository.resolve_commit_hash(specification)?
            }
        } else {
            repository.resolve_commit_hash(specification)?
        };
        let descriptor = repository.extract_descriptor(name, &commit_hash)?;
        Ok(ResolvedModule {
            commit_hash,
            descriptor,
        })
    }
}
