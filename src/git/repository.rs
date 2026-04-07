use std::{borrow::Borrow, path::PathBuf, str::Utf8Error};

use crate::model::protofetch::{Descriptor, ModuleName, Revision, RevisionSpecification};
use gix::{bstr::BStr, object::Kind, Repository};
use gix_credentials::protocol::Context;
use log::{debug, warn};
use thiserror::Error;

use super::cache::ProtofetchGitCache;

#[derive(Error, Debug)]
pub enum ProtoRepoError {
    #[error("Error while performing revparse in dep {0} for commit {1}: {2}")]
    Revparse(ModuleName, String, Box<dyn std::error::Error + Send + Sync>),
    #[error("Git error: {0}")]
    GitError(#[from] Box<dyn std::error::Error + Send + Sync>),
    #[error("Error while decoding utf8 bytes from blob")]
    BlobRead(#[from] Utf8Error),
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
    #[error("Bad git object kind {kind} found for {commit_hash} (expected blob)")]
    BadObjectKind { kind: String, commit_hash: String },
    #[error("Branch {branch} was not found.")]
    BranchNotFound { branch: String },
    #[error("Revision {revision} does not belong to the branch {branch}.")]
    RevisionNotOnBranch { revision: String, branch: String },
    #[error("Error while canonicalizing path {path}: {error}")]
    Canonicalization { path: String, error: std::io::Error },
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("Object not found: {0}")]
    ObjectNotFound(String),
}

pub struct ProtoGitRepository<'a> {
    cache: &'a ProtofetchGitCache,
    git_repo: Repository,
    origin: String,
}

impl ProtoGitRepository<'_> {
    pub fn new(
        cache: &ProtofetchGitCache,
        git_repo: Repository,
        origin: String,
    ) -> ProtoGitRepository {
        ProtoGitRepository {
            cache,
            git_repo,
            origin,
        }
    }

    pub fn fetch(&self, specification: &RevisionSpecification) -> anyhow::Result<()> {
        let mut refspecs = Vec::with_capacity(3);
        if let Revision::Pinned { revision } = &specification.revision {
            refspecs.push(format!("+refs/tags/{}:refs/tags/{}", revision, revision));
            // Some protofetch.toml files specify branch in the revision field,
            // or do not specify the branch at all, so we need to fetch all branches.
            refspecs.push("+refs/heads/*:refs/remotes/origin/*".to_owned());
        }
        if let Some(branch) = &specification.branch {
            refspecs.push(format!(
                "+refs/heads/{}:refs/remotes/origin/{}",
                branch, branch
            ));
        }

        debug!("Fetching {:?} from {}", refspecs, self.origin);
        self.cache.fetch_repo(&self.git_repo, &refspecs)?;
        Ok(())
    }

    pub fn fetch_commit(
        &self,
        specification: &RevisionSpecification,
        commit_hash: &str,
    ) -> anyhow::Result<()> {
        let oid = gix::ObjectId::from_hex(commit_hash.as_bytes())
            .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;

        // Check if commit already exists
        if self.git_repo.find_object(oid).is_ok() {
            return Ok(());
        }

        debug!("Fetching {} from {}", commit_hash, self.origin);
        if let Err(error) = self
            .cache
            .fetch_repo(&self.git_repo, &[commit_hash.to_string()])
        {
            warn!(
                "Failed to fetch a single commit {}, falling back to a full fetch: {}",
                commit_hash, error
            );
            self.fetch(specification)?;
        }

        Ok(())
    }

    pub fn extract_descriptor(
        &self,
        dep_name: &ModuleName,
        commit_hash: &str,
    ) -> Result<Descriptor, ProtoRepoError> {
        use gix::revision::spec::parse::{self, single};

        let spec = format!("{commit_hash}:protofetch.toml");
        let spec_ref: &BStr = spec.as_str().into();

        let result = self.git_repo.rev_parse_single(spec_ref);

        match result {
            // Check if it's a "not found" error
            Err(single::Error::Parse(parse::Error::PathNotFound { .. })) => {
                log::debug!("Couldn't find protofetch.toml, assuming module has no dependencies");
                Ok(Descriptor {
                    name: dep_name.clone(),
                    description: None,
                    proto_out_dir: None,
                    dependencies: Vec::new(),
                })
            }
            Err(e) => Err(ProtoRepoError::Revparse(
                dep_name.to_owned(),
                commit_hash.to_owned(),
                Box::new(e),
            )),
            Ok(id) => {
                let obj = self
                    .git_repo
                    .find_object(id)
                    .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;

                match obj.kind {
                    Kind::Blob => {
                        let content = std::str::from_utf8(&obj.data)?;
                        let descriptor = Descriptor::from_toml_str(content)?;
                        Ok(descriptor)
                    }
                    kind => Err(ProtoRepoError::BadObjectKind {
                        kind: format!("{:?}", kind),
                        commit_hash: commit_hash.to_owned(),
                    }),
                }
            }
        }
    }

    pub fn resolve_commit_hash(
        &self,
        specification: &RevisionSpecification,
    ) -> Result<String, ProtoRepoError> {
        let RevisionSpecification { branch, revision } = specification;
        let oid = match (branch, revision) {
            (None, Revision::Arbitrary) => self.commit_hash_for_obj_str("HEAD")?,
            (None, Revision::Pinned { revision }) => self.commit_hash_for_obj_str(revision)?,
            (Some(branch), Revision::Arbitrary) => self
                .commit_hash_for_obj_str(&format!("origin/{branch}"))
                .map_err(|_| ProtoRepoError::BranchNotFound {
                    branch: branch.to_owned(),
                })?,
            (Some(branch), Revision::Pinned { revision }) => {
                let branch_commit = self
                    .commit_hash_for_obj_str(&format!("origin/{branch}"))
                    .map_err(|_| ProtoRepoError::BranchNotFound {
                        branch: branch.to_owned(),
                    })?;
                let revision_commit = self.commit_hash_for_obj_str(revision)?;
                if self.is_ancestor(revision_commit, branch_commit)? {
                    revision_commit
                } else {
                    return Err(ProtoRepoError::RevisionNotOnBranch {
                        revision: revision.to_owned(),
                        branch: branch.to_owned(),
                    });
                }
            }
        };
        Ok(oid.to_string())
    }

    pub fn create_worktree(
        &self,
        name: &ModuleName,
        commit_hash: &str,
    ) -> Result<PathBuf, ProtoRepoError> {
        let base_path = self.cache.worktrees_path().join(name.as_str());

        if !base_path.exists() {
            std::fs::create_dir_all(&base_path)?;
        }

        let worktree_path = base_path.join(PathBuf::from(commit_hash));

        debug!(
            "Setting up worktree for {} at {}",
            name,
            worktree_path.display()
        );

        if worktree_path.exists() {
            // Worktree directory already exists, verify it's correct
            let canonical_path =
                worktree_path
                    .canonicalize()
                    .map_err(|e| ProtoRepoError::Canonicalization {
                        path: worktree_path.to_string_lossy().to_string(),
                        error: e,
                    })?;
            log::debug!(
                "Found existing worktree for {} at {}",
                name,
                canonical_path.to_string_lossy()
            );
        } else {
            log::info!(
                "Creating new worktree for {} at {}.",
                name,
                worktree_path.to_string_lossy()
            );

            // Create the worktree directory
            std::fs::create_dir_all(&worktree_path)?;

            // Get the commit object
            let oid = gix::ObjectId::from_hex(commit_hash.as_bytes())
                .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;

            let commit = self
                .git_repo
                .find_object(oid)
                .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?
                .try_into_commit()
                .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;

            let tree_id = commit
                .tree_id()
                .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;
            let tree = self
                .git_repo
                .find_object(tree_id)
                .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?
                .try_into_tree()
                .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;

            // Recursively extract tree contents
            self.extract_tree_to_path(&tree, &worktree_path)?;
        }

        Ok(worktree_path)
    }

    fn extract_tree_to_path(
        &self,
        tree: &gix::Tree<'_>,
        dest: &std::path::Path,
    ) -> Result<(), ProtoRepoError> {
        for entry in tree.iter() {
            let entry = entry.map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;
            let entry_path = dest.join(entry.filename().to_string());

            match entry.mode().kind() {
                gix::object::tree::EntryKind::Tree => {
                    std::fs::create_dir_all(&entry_path)?;
                    let subtree = self
                        .git_repo
                        .find_object(entry.oid())
                        .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?
                        .try_into_tree()
                        .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;
                    self.extract_tree_to_path(&subtree, &entry_path)?;
                }
                gix::object::tree::EntryKind::Blob
                | gix::object::tree::EntryKind::BlobExecutable => {
                    let blob = self
                        .git_repo
                        .find_object(entry.oid())
                        .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;
                    std::fs::write(&entry_path, &blob.data)?;
                }
                _ => {
                    // Skip symlinks and other special entries
                }
            }
        }
        Ok(())
    }

    fn commit_hash_for_obj_str(&self, refspec: &str) -> Result<gix::ObjectId, ProtoRepoError> {
        let spec_ref: &BStr = refspec.into();
        let id = self
            .git_repo
            .rev_parse_single(spec_ref)
            .map_err(|e| ProtoRepoError::ObjectNotFound(format!("{}: {}", refspec, e)))?;

        // Peel to commit
        let obj = self
            .git_repo
            .find_object(id)
            .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;

        let commit = obj
            .peel_to_commit()
            .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;
        Ok(commit.id)
    }

    // Check if `a` is an ancestor of `b`
    fn is_ancestor(&self, a: gix::ObjectId, b: gix::ObjectId) -> Result<bool, ProtoRepoError> {
        // Use merge_base to check ancestry
        let merge_base = self
            .git_repo
            .merge_base(a, b)
            .map_err(|e| ProtoRepoError::GitError(Box::new(e)))?;

        Ok(merge_base == a)
    }
}
