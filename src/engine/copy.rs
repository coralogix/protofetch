use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::BufRead as _,
    path::{Component, Path, PathBuf},
};

use log::{debug, trace, warn};
use rayon::{iter::IntoParallelRefIterator, prelude::*, ThreadPoolBuildError, ThreadPoolBuilder};
use thiserror::Error;

use crate::{
    cache::RepositoryCache,
    engine::model::{ResolvedDependency, ResolvedModule, ResolvedRootModule},
    git::backend::types::{GitOid, TreeEntry, TreeEntryKind},
    model::protofetch::{Coordinate, DenyPolicies, ModuleName},
};

#[derive(Error, Debug)]
pub enum ProtoError {
    #[error("Bad proto path. {0}")]
    BadPath(String),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error(transparent)]
    Cache(anyhow::Error),
    #[error(transparent)]
    ThreadPool(#[from] ThreadPoolBuildError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProtoSource {
    // Relative path from the base to the content root.
    content_root: PathBuf,
    // Relative path to the .proto file from the content root
    package_path: PathBuf,
    // Object ID of the file's blob.
    oid: GitOid,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct ProtoFileMapping<'m> {
    // Relative path of the content root to the repository root
    content_root: PathBuf,
    // Relative path of the proto file to the content root
    package_path: PathBuf,
    // The source module name
    module: &'m ModuleName,
    // The coordinate of the module this file belongs to, used for error reporting.
    coordinate: &'m Coordinate,
    // The commit the module is pinned to; with the coordinate it identifies
    // the source repository.
    commit_hash: &'m str,
    // Object ID of the file's blob.
    oid: GitOid,
}

struct Task<'m> {
    dependency: &'m ResolvedDependency,
    prune_context: Option<PruneContext<'m>>,
    include_additional_transitive: bool,
}

pub fn copy<C>(
    cache: C,
    resolved: ResolvedRootModule,
    proto_dir: PathBuf,
    parallelism: usize,
) -> Result<(), ProtoError>
where
    C: RepositoryCache + Clone + 'static,
{
    ThreadPoolBuilder::new()
        .num_threads(parallelism.max(1))
        .build()?
        .install(|| {
            // Every distinct commit is listed once, grouped by repository
            // so each one is opened a single time.
            let mut commits: HashMap<&Coordinate, Vec<&str>> = HashMap::new();
            for module in &resolved.modules {
                let commits = commits.entry(&module.coordinate).or_default();
                if !commits.contains(&module.commit_hash.as_str()) {
                    commits.push(&module.commit_hash);
                }
            }
            let files = commits
                .par_iter()
                .map(|(coordinate, commits)| {
                    let trees = cache
                        .list_trees(coordinate, commits)
                        .map_err(ProtoError::Cache)?;
                    commits
                        .iter()
                        .zip(trees)
                        .map(|(commit, tree)| Ok(((*coordinate, *commit), proto_files(tree)?)))
                        .collect::<Result<Vec<_>, ProtoError>>()
                })
                .collect::<Result<Vec<_>, ProtoError>>()?
                .into_iter()
                .flatten()
                .collect();

            let modules = resolved
                .modules
                .iter()
                .map(|m| (m.name.clone(), m))
                .collect::<HashMap<_, _>>();

            let mut context = Context {
                cache: &cache,
                files,
                modules,
                plan: Vec::new(),
                by_source: HashMap::new(),
                by_target: HashMap::new(),
                deny_policies: Vec::new(),
                active_prunes: Vec::new(),
                active_dependencies: Vec::new(),
                additional_transitive_dependencies: Vec::new(),
                unresolved_imports: Default::default(),
            };

            plan_copies(resolved.dependencies.iter().collect(), &mut context)?;

            context.report_unresolved_imports();

            copy_files(context.plan, proto_dir, &cache)
        })
}

fn plan_copies<'a, 'm: 'a>(
    dependencies: Vec<&'m ResolvedDependency>,
    context: &'a mut Context<'m>,
) -> Result<(), ProtoError> {
    let additional_transitive = dependencies
        .iter()
        .copied()
        .filter(|dep| dep.rules.transitive)
        .collect::<Vec<_>>();

    let mut tasks = Vec::with_capacity(dependencies.len());
    for dependency in dependencies {
        if dependency.rules.transitive && !context.is_pruning() {
            continue;
        }
        if context.is_active_dependency(dependency)
            && !context.is_additional_transitive_dependency(dependency)
        {
            debug!("Skipping circular dependency {}", dependency.name);
            continue;
        }
        let task = if dependency.rules.prune || context.is_pruning() {
            let (prune_context, copied_files) = plan_pruning(dependency, context)?;
            if !copied_files {
                continue;
            }
            Task {
                dependency,
                prune_context,
                include_additional_transitive: true,
            }
        } else {
            plan_not_pruning(dependency, context)?;
            Task {
                dependency,
                prune_context: None,
                include_additional_transitive: false,
            }
        };
        tasks.push(task);
    }

    for task in tasks {
        context.with_active_dependency(task.dependency, |context| {
            if task.include_additional_transitive {
                context.with_additional_transitive_dependencies(
                    additional_transitive.clone(),
                    |context| plan_children(task, context),
                )
            } else {
                plan_children(task, context)
            }
        })?;
    }

    Ok(())
}

fn plan_children<'a, 'm: 'a>(
    task: Task<'m>,
    context: &'a mut Context<'m>,
) -> Result<(), ProtoError> {
    let Task {
        dependency,
        prune_context,
        include_additional_transitive,
    } = task;

    let module = context.module(&dependency.name);
    let dependencies = if include_additional_transitive {
        module
            .dependencies
            .iter()
            .chain(context.additional_transitive_dependencies.iter().copied())
            .collect()
    } else {
        module.dependencies.iter().collect()
    };

    context.with_deny_policy(&dependency.rules.deny_policies, |context| {
        let (prune_context, result) = context
            .with_maybe_prune_context(prune_context, |context| plan_copies(dependencies, context));
        result?;
        if let Some(prune_context) = prune_context {
            context.extend_unresolved_imports(prune_context.root, prune_context.remaining);
        }
        Ok::<_, ProtoError>(())
    })
}

fn plan_not_pruning<'a, 'm: 'a>(
    dependency: &'m ResolvedDependency,
    context: &'a mut Context<'m>,
) -> Result<(), ProtoError> {
    context.with_deny_policy(&dependency.rules.deny_policies, |context| {
        let module = context.module(&dependency.name);
        debug!(
            "Processing regular dependency {} ({})",
            dependency.name, module.coordinate
        );

        let protos = collect_not_denied_files(dependency, context);
        let allowed_protos = filter_allowed_files(protos, dependency);

        for proto in allowed_protos {
            context.add_mapping(ProtoFileMapping {
                content_root: proto.content_root,
                package_path: proto.package_path,
                module: &module.name,
                coordinate: &module.coordinate,
                commit_hash: &module.commit_hash,
                oid: proto.oid,
            });
        }

        Ok(())
    })
}

fn plan_pruning<'a, 'm: 'a>(
    dependency: &'m ResolvedDependency,
    context: &'a mut Context<'m>,
) -> Result<(Option<PruneContext<'m>>, bool), ProtoError> {
    context.with_deny_policy(&dependency.rules.deny_policies, |context| {
        let module = context.module(&dependency.name);
        debug!(
            "Processing pruned dependency {} ({})",
            dependency.name, module.coordinate
        );

        let protos = collect_not_denied_files(dependency, context);
        let allowed_protos = filter_allowed_files(protos.clone(), dependency);

        let allowed_proto_package_paths = allowed_protos
            .iter()
            .map(|proto| proto.package_path.clone())
            .collect::<HashSet<_>>();

        let local_roots = match context.active_prune() {
            None => allowed_proto_package_paths,
            Some(prune_context) => allowed_proto_package_paths
                .intersection(&prune_context.remaining)
                .cloned()
                .collect(),
        };

        let candidate_protos;
        let child_prune_context;

        if dependency.rules.prune {
            // For pruned dependencies we need to look outside of the allow_policies
            candidate_protos = protos;
            child_prune_context = Some(PruneContext::new(module, local_roots));
        } else {
            candidate_protos = allowed_protos;
            child_prune_context = None;
        };

        let mut candidate_protos = candidate_protos
            .into_iter()
            .map(|proto| (proto.package_path.clone(), proto))
            .collect::<HashMap<_, _>>();

        let (child_prune_context, result) =
            context.with_maybe_prune_context(child_prune_context, |context| {
                let prune_context = context
                    .active_prune()
                    .expect("Pruning context must exist at this point");
                let mut copied_files = false;
                let mut queue = prune_context.remaining.iter().cloned().collect::<Vec<_>>();

                // Breadth-first, one batched read per level of imports.
                while !queue.is_empty() {
                    let level = queue
                        .drain(..)
                        .filter_map(|needed| candidate_protos.remove(&needed))
                        .collect::<Vec<_>>();
                    let oids = level
                        .iter()
                        .map(|proto| proto.oid.clone())
                        .collect::<Vec<_>>();
                    let contents = context
                        .cache
                        .read_blobs(&module.coordinate, &oids)
                        .map_err(ProtoError::Cache)?;
                    for (proto, content) in level.into_iter().zip(contents) {
                        copied_files = true;
                        let prune_context = context
                            .active_prune_mut()
                            .expect("prune context is on the stack");
                        for dependency in extract_proto_dependencies(&content)? {
                            if !prune_context.seen.contains(&dependency) {
                                queue.push(dependency.clone());
                                prune_context.remaining.insert(dependency);
                            }
                        }
                        context.add_mapping(ProtoFileMapping {
                            content_root: proto.content_root,
                            package_path: proto.package_path,
                            module: &module.name,
                            coordinate: &module.coordinate,
                            commit_hash: &module.commit_hash,
                            oid: proto.oid,
                        });
                    }
                }
                Ok::<_, ProtoError>(copied_files)
            });

        let copied_files = result?;

        Ok((child_prune_context, copied_files))
    })
}

fn collect_not_denied_files(
    dependency: &ResolvedDependency,
    context: &mut Context,
) -> Vec<ProtoSource> {
    let module = context.module(&dependency.name);

    let mut content_roots = dependency
        .rules
        .content_roots
        .iter()
        .map(|root| root.value.clone())
        .collect::<Vec<_>>();

    if content_roots.is_empty() {
        content_roots.push(PathBuf::default());
    }

    let key = (&module.coordinate, module.commit_hash.as_str());
    let files = context
        .files
        .get(&key)
        .expect("no tree listed for a resolved module");

    let protos = proto_sources(files, content_roots)
        .into_iter()
        .filter(|proto| {
            if context.should_deny_file(&proto.package_path) {
                trace!(
                    "Denied proto file {} for dependency {}",
                    proto.package_path.display(),
                    dependency.name,
                );
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>();

    protos
}

// Upper bound on chained symlinks. A link nested under its own target
// would otherwise add aliases forever.
const MAX_SYMLINK_DEPTH: usize = 32;

// The .proto paths of a tree with their blob object IDs, including aliases
// through symlinks inside the repository, as a checkout would expose them.
fn proto_files(entries: Vec<TreeEntry>) -> Result<BTreeMap<PathBuf, GitOid>, ProtoError> {
    let mut files = BTreeMap::new();
    let mut links = Vec::new();
    for entry in entries {
        match entry.kind {
            TreeEntryKind::Blob => {
                files.insert(entry.path, entry.oid);
            }
            // Links leaving the repository, or nested under their own
            // target (a walk would loop on them), are skipped.
            TreeEntryKind::Symlink { target } => match resolve_link(&entry.path, &target) {
                Some(target) if !entry.path.starts_with(&target) => {
                    links.push((entry.path, target))
                }
                _ => debug!("Skipping symlink {}", entry.path.display()),
            },
        }
    }
    // Each pass aliases every file under a link's target (or the target
    // itself) under the link, following one more level of chained links.
    for _ in 0..MAX_SYMLINK_DEPTH {
        let aliases = links
            .iter()
            .flat_map(|(link, target)| {
                files.iter().filter_map(move |(path, oid)| {
                    let rest = path.strip_prefix(target).ok()?;
                    let alias = if rest.as_os_str().is_empty() {
                        (*link).clone()
                    } else {
                        link.join(rest)
                    };
                    Some((alias, oid.clone()))
                })
            })
            .filter(|(alias, _)| !files.contains_key(alias))
            .collect::<Vec<_>>();
        if aliases.is_empty() {
            files.retain(|path, _| path.extension().is_some_and(|ext| ext == "proto"));
            return Ok(files);
        }
        files.extend(aliases);
    }
    Err(ProtoError::BadPath(
        "symlink expansion did not terminate".to_owned(),
    ))
}

// Resolves a symlink target relative to the link's directory, or `None`
// if it is absolute or leaves the repository.
fn resolve_link(path: &Path, target: &Path) -> Option<PathBuf> {
    let mut resolved = PathBuf::new();
    for component in path.parent()?.join(target).components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            Component::ParentDir if resolved.pop() => {}
            _ => return None,
        }
    }
    Some(resolved)
}

// Attributes each enumerated path to its deepest content root, matching the
// previous filesystem walk: the deepest root wins, paths outside every root
// are skipped, and a path equal to a root is not a file inside it.
fn proto_sources(
    files: &BTreeMap<PathBuf, GitOid>,
    mut content_roots: Vec<PathBuf>,
) -> Vec<ProtoSource> {
    content_roots.sort_by_key(|content_root| content_root.as_os_str().len());
    files
        .iter()
        .filter_map(|(path, oid)| {
            content_roots
                .iter()
                .rev()
                .find_map(|content_root| {
                    path.strip_prefix(content_root)
                        .ok()
                        .filter(|package_path| !package_path.as_os_str().is_empty())
                        .map(|package_path| (content_root, package_path))
                })
                .map(|(content_root, package_path)| ProtoSource {
                    content_root: content_root.clone(),
                    package_path: package_path.to_path_buf(),
                    oid: oid.clone(),
                })
        })
        .collect()
}

/// Extracts the dependencies from a proto file, skipping google/protobuf imports,
/// since these are provided by default by protoc.
fn extract_proto_dependencies(content: &[u8]) -> Result<Vec<PathBuf>, ProtoError> {
    let mut dependencies = Vec::new();
    let mut reader = content;
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        if let Some(rest) = line.strip_prefix("import ") {
            // import = "import" [ "weak" | "public" ] strLit ";"
            let rest = rest.trim_start();
            let rest = ["public", "weak"]
                .iter()
                .find_map(|modifier| {
                    rest.strip_prefix(modifier)
                        .filter(|r| r.starts_with(char::is_whitespace))
                })
                .unwrap_or(rest)
                .trim_start();
            let dependency = rest
                .chars()
                .next()
                .filter(|quote| *quote == '"' || *quote == '\'')
                .and_then(|quote| rest[1..].split(quote).next());
            if let Some(dependency) = dependency {
                if !dependency.starts_with("google/protobuf/") {
                    dependencies.push(PathBuf::from(dependency));
                }
            }
        }
        line.clear();
    }
    Ok(dependencies)
}

fn filter_allowed_files(
    protos: Vec<ProtoSource>,
    dependency: &ResolvedDependency,
) -> Vec<ProtoSource> {
    protos
        .into_iter()
        .filter(|proto| {
            dependency
                .rules
                .allow_policies
                .should_allow_file(&proto.package_path)
        })
        .collect::<Vec<_>>()
}

// Targets in the plan are unique paths (see Context::add_mapping), so the
// writes can run in parallel, one batched read per repository. On
// case-insensitive filesystems, targets differing only by case still share a
// file, and which one wins is not deterministic.
fn copy_files(
    plan: Vec<ProtoFileMapping<'_>>,
    proto_dir: PathBuf,
    cache: &dyn RepositoryCache,
) -> Result<(), ProtoError> {
    debug_assert!(
        plan.iter()
            .map(|mapping| &mapping.package_path)
            .collect::<HashSet<_>>()
            .len()
            == plan.len(),
        "Internal invariant violated: duplicate targets in the copy plan"
    );
    let mut by_coordinate: HashMap<&Coordinate, Vec<ProtoFileMapping>> = HashMap::new();
    for mapping in plan {
        by_coordinate
            .entry(mapping.coordinate)
            .or_default()
            .push(mapping);
    }
    by_coordinate
        .into_par_iter()
        .try_for_each(|(coordinate, mappings)| {
            let oids = mappings
                .iter()
                .map(|mapping| mapping.oid.clone())
                .collect::<Vec<_>>();
            let contents = cache
                .read_blobs(coordinate, &oids)
                .map_err(ProtoError::Cache)?;
            for (mapping, content) in mappings.iter().zip(contents) {
                trace!(
                    "Copying proto file {} from {}/ for dependency {} ({})",
                    &mapping.package_path.to_string_lossy(),
                    &mapping.content_root.to_string_lossy(),
                    &mapping.module,
                    &mapping.coordinate
                );
                let target = proto_dir.join(&mapping.package_path);
                let prefix = target.parent().ok_or_else(|| {
                    ProtoError::BadPath(format!(
                        "Bad parent dest file for {}",
                        &target.to_string_lossy()
                    ))
                })?;
                std::fs::create_dir_all(prefix)?;
                std::fs::write(target, content)?;
            }
            Ok(())
        })
}

struct Context<'m> {
    cache: &'m dyn RepositoryCache,
    // The .proto files of every listed (repository, commit).
    files: HashMap<(&'m Coordinate, &'m str), BTreeMap<PathBuf, GitOid>>,
    modules: HashMap<ModuleName, &'m ResolvedModule>,
    // The plan of which files to copy from where to where
    plan: Vec<ProtoFileMapping<'m>>,
    // Maps source identities to their index in the plan.
    by_source: HashMap<(&'m Coordinate, &'m str, PathBuf), usize>,
    // Maps target paths to their index in the plan.
    by_target: HashMap<PathBuf, usize>,
    deny_policies: Vec<&'m DenyPolicies>,
    active_prunes: Vec<PruneContext<'m>>,
    active_dependencies: Vec<&'m ResolvedDependency>,
    additional_transitive_dependencies: Vec<&'m ResolvedDependency>,
    unresolved_imports: Vec<(&'m ResolvedModule, PathBuf)>,
}

impl<'m> Context<'m> {
    fn module(&self, name: &ModuleName) -> &'m ResolvedModule {
        self.modules
            .get(name)
            .expect("Internal invariant violated: no information about the already resolved module")
    }

    fn add_mapping(&mut self, mapping: ProtoFileMapping<'m>) {
        // The identity of a source file: its repository, and the path of the
        // file within it (content root plus package path).
        let source = (
            mapping.coordinate,
            mapping.commit_hash,
            mapping.content_root.join(&mapping.package_path),
        );

        if let Some(existing) = self.by_source.get(&source) {
            let existing = &self.plan[*existing];
            if existing.package_path != mapping.package_path {
                warn!(
                    "Discarded duplicate target {} in favor of {} for {} ({})",
                    mapping.package_path.display(),
                    existing.package_path.display(),
                    mapping.module,
                    mapping.coordinate
                );
                return;
            }
        }
        let already_planned = if let Some(existing) = self.by_target.get(&mapping.package_path) {
            let existing = &self.plan[*existing];
            if existing.coordinate != mapping.coordinate
                || existing.commit_hash != mapping.commit_hash
                || existing.content_root != mapping.content_root
            {
                // Object IDs are content addresses, so equal IDs guarantee
                // equal contents.
                if existing.oid != mapping.oid {
                    warn!(
                        "Discarded conflicting source {} ({}) in favor of {} ({}) for {}",
                        mapping.module,
                        mapping.coordinate,
                        existing.module,
                        existing.coordinate,
                        mapping.package_path.display(),
                    );
                } else {
                    debug!(
                        "Discarded conflicting identical source {} ({}) in favor of {} ({}) for {}",
                        mapping.module,
                        mapping.coordinate,
                        existing.module,
                        existing.coordinate,
                        mapping.package_path.display(),
                    );
                }
                return;
            }
            true
        } else {
            false
        };

        for prune_context in &mut self.active_prunes {
            prune_context.remaining.remove(&mapping.package_path);
            prune_context.seen.insert(mapping.package_path.clone());
        }

        // The same file reached again through another path in the dependency
        // graph still counts for pruning above, but is only copied once.
        if already_planned {
            return;
        }

        let index = self.plan.len();
        self.by_source.insert(source, index);
        self.by_target.insert(mapping.package_path.clone(), index);
        self.plan.push(mapping);
    }

    fn with_deny_policy<F, R>(&mut self, policy: &'m DenyPolicies, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        self.deny_policies.push(policy);
        let result = f(self);
        self.deny_policies.pop();
        result
    }

    fn with_maybe_prune_context<F, R>(
        &mut self,
        prune_context: Option<PruneContext<'m>>,
        f: F,
    ) -> (Option<PruneContext<'m>>, R)
    where
        F: FnOnce(&mut Self) -> R,
    {
        match prune_context {
            None => (None, f(self)),
            Some(prune_context) => {
                self.active_prunes.push(prune_context);
                let result = f(self);
                let prune_context = self.active_prunes.pop().unwrap();
                (Some(prune_context), result)
            }
        }
    }

    fn with_active_dependency<F, R>(&mut self, dependency: &'m ResolvedDependency, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        self.active_dependencies.push(dependency);
        let result = f(self);
        self.active_dependencies.pop();
        result
    }

    fn with_additional_transitive_dependencies<F, R>(
        &mut self,
        dependencies: Vec<&'m ResolvedDependency>,
        f: F,
    ) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        let len = self.additional_transitive_dependencies.len();
        for dependency in dependencies {
            if !self
                .additional_transitive_dependencies
                .contains(&dependency)
            {
                self.additional_transitive_dependencies.push(dependency);
            }
        }
        let result = f(self);
        self.additional_transitive_dependencies.truncate(len);
        result
    }

    fn is_active_dependency(&self, dependency: &ResolvedDependency) -> bool {
        self.active_dependencies.contains(&dependency)
    }

    fn is_additional_transitive_dependency(&self, dependency: &ResolvedDependency) -> bool {
        self.additional_transitive_dependencies
            .contains(&dependency)
    }

    fn should_deny_file(&self, file: &Path) -> bool {
        self.deny_policies
            .iter()
            .any(|deny_policy| deny_policy.should_deny_file(file))
    }

    fn is_pruning(&self) -> bool {
        !self.active_prunes.is_empty()
    }

    fn active_prune(&self) -> Option<&PruneContext<'m>> {
        self.active_prunes.last()
    }

    fn active_prune_mut(&mut self) -> Option<&mut PruneContext<'m>> {
        self.active_prunes.last_mut()
    }

    fn extend_unresolved_imports(&mut self, module: &'m ResolvedModule, imports: HashSet<PathBuf>) {
        self.unresolved_imports
            .extend(imports.into_iter().map(|import| (module, import)));
    }

    fn report_unresolved_imports(&self) {
        for (module, import) in &self.unresolved_imports {
            if self.by_target.contains_key(import) {
                debug!(
                    "Import {} was not resolved when pruning {} ({}), but it is still present in the final set of copied files",
                    import.display(),
                    module.name,
                    module.coordinate
                );
            } else {
                warn!(
                    "Import {} was not resolved when pruning {} ({})",
                    import.display(),
                    module.name,
                    module.coordinate
                );
            }
        }
    }
}

struct PruneContext<'m> {
    root: &'m ResolvedModule,
    remaining: HashSet<PathBuf>,
    seen: HashSet<PathBuf>,
}

impl<'m> PruneContext<'m> {
    fn new(root: &'m ResolvedModule, remaining: HashSet<PathBuf>) -> PruneContext<'m> {
        PruneContext {
            root,
            remaining,
            seen: HashSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::protofetch::RevisionSpecification;

    fn oid(n: u8) -> GitOid {
        GitOid::from_hex(format!("{n:040x}"))
    }

    fn blob(path: &str, n: u8) -> TreeEntry {
        TreeEntry {
            path: PathBuf::from(path),
            oid: oid(n),
            kind: TreeEntryKind::Blob,
        }
    }

    fn symlink(path: &str, target: &str, n: u8) -> TreeEntry {
        TreeEntry {
            path: PathBuf::from(path),
            oid: oid(n),
            kind: TreeEntryKind::Symlink {
                target: PathBuf::from(target),
            },
        }
    }

    fn paths(files: &BTreeMap<PathBuf, GitOid>) -> Vec<PathBuf> {
        files.keys().cloned().collect()
    }

    #[test]
    fn proto_files_follows_symlinked_directories() {
        let entries = vec![
            blob("real/pkg/a.proto", 1),
            blob("real/pkg/readme.md", 2),
            symlink("linked", "real", 3),
        ];

        let files = proto_files(entries).unwrap();
        assert_eq!(
            paths(&files),
            vec![
                PathBuf::from("linked/pkg/a.proto"),
                PathBuf::from("real/pkg/a.proto"),
            ]
        );
        assert_eq!(files[&PathBuf::from("linked/pkg/a.proto")], oid(1));
    }

    #[test]
    fn proto_files_includes_file_links_named_proto() {
        let entries = vec![
            blob("proto/one.proto", 1),
            symlink("alias.proto", "proto/one.proto", 2),
        ];

        assert_eq!(
            paths(&proto_files(entries).unwrap()),
            vec![
                PathBuf::from("alias.proto"),
                PathBuf::from("proto/one.proto"),
            ]
        );
    }

    #[test]
    fn proto_files_follows_file_link_chains() {
        // A link to a link: the old directory walk found `b.proto` because
        // its own name ends in .proto, whatever its target is named.
        let entries = vec![
            blob("real.proto", 1),
            symlink("a.txt", "real.proto", 2),
            symlink("b.proto", "a.txt", 3),
        ];

        let files = proto_files(entries).unwrap();
        assert_eq!(
            paths(&files),
            vec![PathBuf::from("b.proto"), PathBuf::from("real.proto")]
        );
        // The alias carries the real file's object ID, not the link's.
        assert_eq!(files[&PathBuf::from("b.proto")], oid(1));
    }

    #[test]
    fn proto_files_ignores_file_links_not_named_proto() {
        let entries = vec![
            blob("docs/AGENTS.md", 1),
            symlink("AGENTS.md", "docs/AGENTS.md", 2),
        ];

        assert!(proto_files(entries).unwrap().is_empty());
    }

    #[test]
    fn proto_files_skips_links_escaping_the_repository() {
        // `a/out` climbs above the root and back in; accepting it would
        // alias the real `pkg` tree under `a/out`.
        let entries = vec![
            blob("pkg/a.proto", 1),
            symlink("abs", "/etc/passwd", 2),
            symlink("up", "../outside", 3),
            symlink("a/out", "../../pkg", 4),
        ];

        assert_eq!(
            paths(&proto_files(entries).unwrap()),
            vec![PathBuf::from("pkg/a.proto")]
        );
    }

    #[test]
    fn proto_files_errors_on_mutually_nested_links() {
        let entries = vec![
            blob("b/f.proto", 1),
            symlink("a/l", "../b", 2),
            symlink("b/m", "../a", 3),
        ];

        assert!(matches!(
            proto_files(entries),
            Err(ProtoError::BadPath(message)) if message == "symlink expansion did not terminate"
        ));
    }

    #[test]
    fn proto_files_skips_self_nested_links() {
        // Without the guard, `a/l -> .` over `a/x.proto` would add
        // `a/l/x.proto`, `a/l/l/x.proto`, ... and never terminate.
        let entries = vec![blob("a/x.proto", 1), symlink("a/l", ".", 2)];

        assert_eq!(
            paths(&proto_files(entries).unwrap()),
            [PathBuf::from("a/x.proto")]
        );
    }

    #[test]
    fn proto_sources_prefers_the_deepest_content_root() {
        let files = BTreeMap::from([
            (PathBuf::from("pkg/a.proto"), oid(1)),
            (PathBuf::from("real/pkg/a.proto"), oid(2)),
        ]);
        let sources = proto_sources(&files, vec![PathBuf::default(), PathBuf::from("real")]);

        assert_eq!(
            sources,
            vec![
                ProtoSource {
                    content_root: PathBuf::default(),
                    package_path: PathBuf::from("pkg/a.proto"),
                    oid: oid(1),
                },
                ProtoSource {
                    content_root: PathBuf::from("real"),
                    package_path: PathBuf::from("pkg/a.proto"),
                    oid: oid(2),
                },
            ]
        );
    }

    #[test]
    fn proto_sources_skips_paths_equal_to_a_root() {
        // A path equal to a deeper root is attributed to the shallower one,
        // matching the previous filesystem walk.
        let files = BTreeMap::from([(PathBuf::from("x.proto"), oid(1))]);
        let sources = proto_sources(&files, vec![PathBuf::from("x.proto"), PathBuf::default()]);

        assert_eq!(
            sources,
            vec![ProtoSource {
                content_root: PathBuf::default(),
                package_path: PathBuf::from("x.proto"),
                oid: oid(1),
            }]
        );
    }

    #[test]
    fn proto_sources_attributes_symlinked_directories_to_the_deepest_root() {
        let entries = vec![
            blob("real/pkg/a.proto", 1),
            blob("real/pkg/readme.md", 2),
            symlink("linked", "real", 3),
        ];
        let files = proto_files(entries).unwrap();

        let sources = proto_sources(
            &files,
            vec![PathBuf::default(), PathBuf::from("linked/pkg")],
        );

        assert_eq!(
            sources,
            vec![
                ProtoSource {
                    content_root: PathBuf::from("linked/pkg"),
                    package_path: PathBuf::from("a.proto"),
                    oid: oid(1),
                },
                ProtoSource {
                    content_root: PathBuf::default(),
                    package_path: PathBuf::from("real/pkg/a.proto"),
                    oid: oid(1),
                }
            ]
        );
    }

    struct NoCache;

    impl RepositoryCache for NoCache {
        fn fetch(&self, _: &Coordinate, _: &RevisionSpecification, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn list_trees(&self, _: &Coordinate, _: &[&str]) -> anyhow::Result<Vec<Vec<TreeEntry>>> {
            unreachable!()
        }
        fn read_blobs(&self, _: &Coordinate, _: &[GitOid]) -> anyhow::Result<Vec<Vec<u8>>> {
            unreachable!()
        }
    }

    fn empty_context() -> Context<'static> {
        Context {
            cache: &NoCache,
            files: HashMap::new(),
            modules: HashMap::new(),
            plan: Vec::new(),
            by_source: HashMap::new(),
            by_target: HashMap::new(),
            deny_policies: Vec::new(),
            active_prunes: Vec::new(),
            active_dependencies: Vec::new(),
            additional_transitive_dependencies: Vec::new(),
            unresolved_imports: Vec::new(),
        }
    }

    fn mapping<'m>(
        module: &'m ModuleName,
        coordinate: &'m Coordinate,
        commit_hash: &'m str,
        oid_value: u8,
    ) -> ProtoFileMapping<'m> {
        ProtoFileMapping {
            content_root: PathBuf::default(),
            package_path: PathBuf::from("pkg/a.proto"),
            module,
            coordinate,
            commit_hash,
            oid: oid(oid_value),
        }
    }

    #[test]
    fn add_mapping_discards_a_conflicting_source_by_object_id() {
        let module = ModuleName::new("dep".to_owned());
        let coordinate = Coordinate {
            forge: String::new(),
            organization: String::new(),
            repository: "repo".to_owned(),
            protocol: None,
        };
        let mut context = empty_context();
        context.add_mapping(mapping(&module, &coordinate, "aaa", 1));
        // Same target from another commit, with a different blob: discarded.
        context.add_mapping(mapping(&module, &coordinate, "bbb", 2));

        assert_eq!(context.plan.len(), 1);
        assert_eq!(context.plan[0].oid, oid(1));

        // The same blob through another root is the same content, also
        // discarded, without changing the plan.
        context.add_mapping(mapping(&module, &coordinate, "aaa", 1));
        assert_eq!(context.plan.len(), 1);

        // A different target from the same source is added.
        let mut second = mapping(&module, &coordinate, "aaa", 1);
        second.package_path = PathBuf::from("pkg/b.proto");
        context.add_mapping(second);
        assert_eq!(context.plan.len(), 2);
    }

    #[test]
    fn extract_proto_dependencies_handles_import_modifiers() {
        let content = br#"syntax = "proto3";
import "plain.proto";
import public "foo/public.proto";
import weak "foo/weak.proto";
import "google/protobuf/empty.proto";
message Service {}
"#;

        assert_eq!(
            extract_proto_dependencies(content).unwrap(),
            vec![
                PathBuf::from("plain.proto"),
                PathBuf::from("foo/public.proto"),
                PathBuf::from("foo/weak.proto"),
            ]
        );
    }
}
