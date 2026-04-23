use std::path::PathBuf;

use crate::{
    git::cache::ProtofetchGitCache,
    model::protofetch::{Coordinate, ModuleName, RevisionSpecification},
};

use super::RepositoryCache;

impl RepositoryCache for ProtofetchGitCache {
    fn fetch(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: &str,
    ) -> anyhow::Result<()> {
        // Hold per-repo lock for the entire fetch operation
        let lock = self.lock_repo(coordinate);
        let _guard = lock.lock();
        let repository = self.open_or_create_repo(coordinate)?;
        repository.fetch_commit(specification, commit_hash)?;
        Ok(())
    }

    fn create_worktree(
        &self,
        coordinate: &Coordinate,
        commit_hash: &str,
        name: &ModuleName,
    ) -> anyhow::Result<PathBuf> {
        // Hold per-repo lock for the entire worktree creation
        let lock = self.lock_repo(coordinate);
        let _guard = lock.lock();
        let repository = self.open_or_create_repo(coordinate)?;
        let path = repository.create_worktree(name, commit_hash)?;
        Ok(path)
    }
}
