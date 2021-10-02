use std::collections::HashMap;
use std::str::{from_utf8, Utf8Error};

use git2::Repository;

use crate::cache::{CacheError, ProtofetchCache};
use crate::model::{Coordinate, Dependency, Descriptor, Revision};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Error while fetching repo from cache")]
    Cache(#[from] CacheError),
    #[error("Error while performing revparse")]
    Revparse(#[from] git2::Error),
    #[error("Error while decoding utf8 bytes from blob")]
    BlobRead(#[from] Utf8Error),
    #[error("Bad git object kind {kind} found for {revision} (expected blob)")]
    BadObjectKind { kind: String, revision: String },
    #[error("Missing `module.toml` for revision {revision}")]
    MissingDescriptor { revision: String },
    #[error("Error while parsing descriptor")]
    Parsing(#[from] crate::model::ParseError),
}

pub fn resolve_dependencies(
    cache: &ProtofetchCache,
    dependencies: &Vec<Dependency>,
) -> Result<HashMap<Coordinate, Revision>, FetchError> {
    fn go(
        cache: &ProtofetchCache,
        dep_map: &mut HashMap<Coordinate, Vec<Revision>>,
        dependencies: &Vec<Dependency>,
    ) -> Result<(), FetchError> {
        for dependency in dependencies {
            dep_map
                .entry(dependency.coordinate)
                .and_modify(|vec| vec.push(dependency.revision))
                .or_insert(vec![dependency.revision]);

            let repo = cache.clone_or_fetch(&dependency.coordinate)?;
            let descriptor = extract_descriptor(&repo, &dependency.revision)?;

            go(cache, dep_map, &descriptor.dependencies)?;
        }

        Ok(())
    }

    let mut dep_map: HashMap<Coordinate, Vec<Revision>> = HashMap::new();

    go(cache, &mut dep_map, dependencies)?;

    Ok(resolve_conflicts(dep_map))
}

fn extract_descriptor(repo: &Repository, revision: &Revision) -> Result<Descriptor, FetchError> {
    let rendered_revision = revision.to_string();
    let result = repo
        .revparse_single(&format!("{}:module.toml", rendered_revision))
        .map_err(|e| FetchError::Revparse(e))?;

    match result.kind() {
        Some(git2::ObjectType::Blob) => {
            let blob = result.peel_to_blob()?;
            let content = from_utf8(blob.content())?;
            let descriptor = Descriptor::from_str(content)?;

            Ok(descriptor)
        }
        Some(kind) => Err(FetchError::BadObjectKind {
            kind: kind.to_string(),
            revision: rendered_revision,
        }),
        None => Err(FetchError::MissingDescriptor {
            revision: rendered_revision,
        }),
    }
}

fn resolve_conflicts(dep_map: HashMap<Coordinate, Vec<Revision>>) -> HashMap<Coordinate, Revision> {
    dep_map
        .into_iter()
        .filter_map(|(k, v)| {
            if !v.is_empty() {
                Some((k, v.remove(0)))
            } else {
                None
            }
        })
        .collect()
}
