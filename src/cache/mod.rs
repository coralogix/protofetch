mod git;

use std::path::PathBuf;

use crate::model::protofetch::{Coordinate, DependencyName, RevisionSpecification};

pub trait RepositoryCache {
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
        name: &DependencyName,
    ) -> anyhow::Result<PathBuf>;
}
