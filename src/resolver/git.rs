use crate::{
    git::cache::ProtofetchGitCache,
    model::protofetch::{Coordinate, DependencyName, RevisionSpecification},
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
        let repository = self.repository(coordinate)?;
        let commit_hash = if let Some(commit_hash) = commit_hash {
            repository.fetch_commit(specification, commit_hash)?;
            commit_hash.to_owned()
        } else {
            repository.fetch(specification)?;
            repository.resolve_commit_hash(specification)?
        };
        let descriptor = repository.extract_descriptor(name, &commit_hash)?;
        Ok(ResolvedModule {
            commit_hash,
            descriptor,
        })
    }
}
