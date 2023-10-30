use crate::{
    cache::{ProtofetchGitCache, RepositoryCache},
    model::protofetch::{Coordinate, DependencyName, RevisionSpecification},
};

use super::{ModuleResolver, ResolvedModule};

impl ModuleResolver for ProtofetchGitCache {
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        name: &DependencyName,
    ) -> anyhow::Result<ResolvedModule> {
        let repository = self.clone_or_update(coordinate)?;
        let commit_hash = repository.resolve_commit_hash(specification)?;
        let descriptor = repository.extract_descriptor(name, &commit_hash)?;
        Ok(ResolvedModule {
            commit_hash,
            descriptor,
        })
    }
}
