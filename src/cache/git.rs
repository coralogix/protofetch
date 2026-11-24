use std::path::PathBuf;

use crate::{
    git::{backend::types::GitOid, cache::ProtofetchGitCache},
    model::protofetch::{Coordinate, RevisionSpecification},
};

use super::{RepositoryCache, TreeEntry};

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
    ) -> anyhow::Result<PathBuf> {
        let path = self
            .repository(coordinate)?
            .create_worktree(coordinate, commit_hash)?;
        Ok(path)
    }

    fn list_trees(
        &self,
        coordinate: &Coordinate,
        commits: &[&str],
    ) -> anyhow::Result<Vec<Vec<TreeEntry>>> {
        let repository = self.repository(coordinate)?;
        Ok(commits
            .iter()
            .map(|commit| repository.list_tree(commit))
            .collect::<Result<_, _>>()?)
    }

    fn read_blobs(&self, coordinate: &Coordinate, oids: &[GitOid]) -> anyhow::Result<Vec<Vec<u8>>> {
        Ok(self.repository(coordinate)?.read_blobs(oids)?)
    }
}
