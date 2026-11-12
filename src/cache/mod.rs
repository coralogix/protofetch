mod git;

use std::{path::PathBuf, sync::Arc};

use crate::{
    git::backend::types::TreeEntry,
    model::protofetch::{Coordinate, RevisionSpecification},
};

pub trait RepositoryCache: Send + Sync {
    fn fetch(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: &str,
    ) -> anyhow::Result<()>;

    fn create_worktree(
        &self,
        coordinate: &Coordinate,
        commit_hash: &str,
    ) -> anyhow::Result<PathBuf>;

    /// Lists the trees of `commits` of `coordinate`, in order, opening the
    /// repository once for all of them.
    fn list_trees(
        &self,
        coordinate: &Coordinate,
        commits: &[&str],
    ) -> anyhow::Result<Vec<Vec<TreeEntry>>>;
}

impl<T> RepositoryCache for Arc<T>
where
    T: RepositoryCache + ?Sized,
{
    fn fetch(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: &str,
    ) -> anyhow::Result<()> {
        T::fetch(self, coordinate, specification, commit_hash)
    }

    fn create_worktree(
        &self,
        coordinate: &Coordinate,
        commit_hash: &str,
    ) -> anyhow::Result<PathBuf> {
        T::create_worktree(self, coordinate, commit_hash)
    }

    fn list_trees(
        &self,
        coordinate: &Coordinate,
        commits: &[&str],
    ) -> anyhow::Result<Vec<Vec<TreeEntry>>> {
        T::list_trees(self, coordinate, commits)
    }
}
