mod git;

use std::sync::Arc;

use crate::{
    git::backend::types::{GitOid, TreeEntry},
    model::protofetch::{Coordinate, RevisionSpecification},
};

pub trait RepositoryCache: Send + Sync {
    fn fetch(
        &self,
        coordinate: &Coordinate,
        specification: &RevisionSpecification,
        commit_hash: &str,
    ) -> anyhow::Result<()>;

    /// Lists the trees of `commits` of `coordinate`, in order, opening the
    /// repository once for all of them.
    fn list_trees(
        &self,
        coordinate: &Coordinate,
        commits: &[&str],
    ) -> anyhow::Result<Vec<Vec<TreeEntry>>>;

    /// Reads the blobs `oids` of `coordinate`, in order.
    fn read_blobs(&self, coordinate: &Coordinate, oids: &[GitOid]) -> anyhow::Result<Vec<Vec<u8>>>;
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

    fn list_trees(
        &self,
        coordinate: &Coordinate,
        commits: &[&str],
    ) -> anyhow::Result<Vec<Vec<TreeEntry>>> {
        T::list_trees(self, coordinate, commits)
    }

    fn read_blobs(&self, coordinate: &Coordinate, oids: &[GitOid]) -> anyhow::Result<Vec<Vec<u8>>> {
        T::read_blobs(self, coordinate, oids)
    }
}
