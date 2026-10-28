use std::sync::Arc;

use crate::{
    git::cache::ProtofetchGitCache,
    model::protofetch::{Coordinate, ModuleName, RevisionSpecification},
};

use super::{CommitAndDescriptor, ModuleResolver};

#[derive(Clone, Copy)]
pub enum ValidationMode {
    Cached,
    Remote,
}

pub struct GitModuleResolver {
    pub cache: Arc<ProtofetchGitCache>,
    pub validation: ValidationMode,
}

impl ModuleResolver for GitModuleResolver {
    fn resolve(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: Option<&str>,
        name: &ModuleName,
    ) -> anyhow::Result<CommitAndDescriptor> {
        let repository = self.cache.repository(coordinate)?;
        let commit_hash = if let Some(commit_hash) = commit_hash {
            match self.validation {
                ValidationMode::Cached => repository.fetch_commit(specification, commit_hash)?,
                ValidationMode::Remote => repository.validate_commit(specification, commit_hash)?,
            }
            commit_hash.to_owned()
        } else {
            repository.fetch(specification)?;
            repository.resolve_commit_hash(specification)?
        };
        let descriptor = repository.extract_descriptor(name, &commit_hash)?;
        Ok(CommitAndDescriptor {
            commit_hash,
            descriptor,
        })
    }
}
