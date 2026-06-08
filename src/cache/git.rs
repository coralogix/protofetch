use std::path::PathBuf;

use crate::{
    git::cache::ProtofetchGitCache,
    model::protofetch::{Coordinate, DependencyRoot, RevisionSpecification},
};

use super::RepositoryCache;

impl RepositoryCache for ProtofetchGitCache {
    fn fetch(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: &str,
    ) -> anyhow::Result<()> {
        let repository = self.repository(coordinate)?;
        repository.fetch_commit(specification, commit_hash)?;
        Ok(())
    }

    fn create_worktree(
        &self,
        coordinate: &Coordinate,
        commit_hash: &str,
        roots: &[DependencyRoot],
    ) -> anyhow::Result<PathBuf> {
        let path = self
            .repository(coordinate)?
            .create_worktree(coordinate, commit_hash, roots)?;
        Ok(path)
    }
}
