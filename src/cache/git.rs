use std::path::PathBuf;

use crate::{
    git::cache::ProtofetchGitCache,
    model::protofetch::{Coordinate, DependencyName, RevisionSpecification},
};

use super::RepositoryCache;

impl RepositoryCache for ProtofetchGitCache {
    fn fetch(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        _commit_hash: &str,
    ) -> anyhow::Result<()> {
        self.clone_or_update(coordinate)?
            .resolve_commit_hash(specification)?;
        Ok(())
    }

    fn create_worktree(
        &self,
        coordinate: &Coordinate,
        commit_hash: &str,
        name: &DependencyName,
    ) -> anyhow::Result<PathBuf> {
        let path = self
            .clone_or_update(coordinate)?
            .create_worktree(name, commit_hash)?;
        Ok(path)
    }
}
