use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::File,
    io::{BufRead as _, BufReader},
    path::{Path, PathBuf},
};

use log::{debug, trace, warn};
use rayon::{iter::IntoParallelRefIterator, prelude::*, ThreadPoolBuildError, ThreadPoolBuilder};
use thiserror::Error;

use crate::{
    cache::RepositoryCache,
    engine::model::{ResolvedDependency, ResolvedModule, ResolvedRootModule},
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

#[derive(Debug, Clone)]
struct ProtoSource {
    // Base path containing the content root.
    worktree: PathBuf,
    // Relative path from the base to the content root.
    content_root: PathBuf,
    // Relative path to the .proto file from the content root
    package_path: PathBuf,
}

impl ProtoSource {
    fn full_path(&self) -> PathBuf {
        self.worktree
            .join(&self.content_root)
            .join(&self.package_path)
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct ProtoFileMapping<'m> {
    // Worktree of the dependency
    worktree: PathBuf,
    // Relative path of the content root to the worktree
    content_root: PathBuf,
    // Relative path of the proto file to the content root
    package_path: PathBuf,
    // The source module name
    module: &'m ModuleName,
    // The coordinate of the module this file belongs to, used for error reporting.
    coordinate: &'m Coordinate,
}

impl<'m> ProtoFileMapping<'m> {
    fn source_path(&self) -> PathBuf {
        self.worktree
            .join(&self.content_root)
            .join(&self.package_path)
    }
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
            let worktrees = resolved
                .modules
                .par_iter()
                .map(|module| {
                    cache
                        .create_worktree(&module.coordinate, &module.commit_hash)
                        .map(|path| (module.name.clone(), path))
                        .map_err(ProtoError::Cache)
                })
                .collect::<Result<HashMap<_, _>, _>>()?;

            let modules = resolved
                .modules
                .iter()
                .map(|m| (m.name.clone(), m))
                .collect::<HashMap<_, _>>();

            let mut context = Context {
                worktrees,
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

            copy_files(context.plan, proto_dir)
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

        let protos = collect_not_denied_files(dependency, context)?;
        let allowed_protos = filter_allowed_files(protos, dependency);

        for proto in allowed_protos {
            context.add_mapping(ProtoFileMapping {
                worktree: proto.worktree,
                content_root: proto.content_root,
                package_path: proto.package_path,
                module: &module.name,
                coordinate: &module.coordinate,
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

        let protos = collect_not_denied_files(dependency, context)?;
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

        let mut copied_files = false;
        let (child_prune_context, result) =
            context.with_maybe_prune_context(child_prune_context, |context| {
                let prune_context = context
                    .active_prune()
                    .expect("Pruning context must exist at this point");
                let mut queue = prune_context
                    .remaining
                    .iter()
                    .cloned()
                    .collect::<VecDeque<_>>();

                while let Some(needed) = queue.pop_front() {
                    if let Some(proto) = candidate_protos.remove(&needed) {
                        copied_files = true;
                        let prune_context = context.active_prune_mut().unwrap();
                        for dependency in extract_proto_dependencies(&proto.full_path())? {
                            if !prune_context.seen.contains(&dependency) {
                                queue.push_back(dependency.clone());
                                prune_context.remaining.insert(dependency);
                            }
                        }
                        context.add_mapping(ProtoFileMapping {
                            worktree: proto.worktree,
                            content_root: proto.content_root,
                            package_path: proto.package_path,
                            module: &module.name,
                            coordinate: &module.coordinate,
                        });
                    }
                }
                Ok::<_, ProtoError>(())
            });

        result?;

        Ok((child_prune_context, copied_files))
    })
}

fn collect_not_denied_files(
    dependency: &ResolvedDependency,
    context: &Context,
) -> Result<Vec<ProtoSource>, ProtoError> {
    let worktree = context.worktree(&dependency.name);

    let mut content_roots = dependency
        .rules
        .content_roots
        .iter()
        .map(|root| root.value.clone())
        .collect::<Vec<_>>();

    if content_roots.is_empty() {
        content_roots.push(PathBuf::default());
    }

    let protos = find_proto_files(worktree.clone(), content_roots)?;

    let protos = protos
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

    Ok(protos)
}

// Recursively finds all .proto files in the given directories and its subdirectories,
// returning their relative paths to the closest root.
// Provided worktree path must be absolute.
fn find_proto_files(
    worktree: PathBuf,
    mut content_roots: Vec<PathBuf>,
) -> Result<Vec<ProtoSource>, ProtoError> {
    fn rec(
        worktree: &Path,
        content_root: &Path,
        dir: &Path,
        files: &mut Vec<ProtoSource>,
        cutoff: &HashSet<PathBuf>,
    ) -> Result<(), ProtoError> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if !cutoff.contains(&path) {
                    rec(worktree, content_root, &path, files, cutoff)?;
                }
            } else if let Some(extension) = path.extension() {
                if extension == "proto" {
                    if let Ok(path) = path.strip_prefix(worktree) {
                        if let Ok(package_path) = path.strip_prefix(content_root) {
                            files.push(ProtoSource {
                                worktree: worktree.to_path_buf(),
                                content_root: content_root.to_path_buf(),
                                package_path: package_path.to_path_buf(),
                            });
                        }
                    }
                }
            }
        }

        Ok(())
    }

    content_roots.sort_by_key(|content_root| content_root.as_os_str().len());

    let mut files = Vec::<ProtoSource>::new();
    let mut cutoff = HashSet::<PathBuf>::new();

    for content_root in content_roots.into_iter().rev() {
        let root_path = worktree.join(&content_root);
        if root_path.is_dir() {
            rec(&worktree, &content_root, &root_path, &mut files, &cutoff)?;
        }
        cutoff.insert(root_path);
    }
    Ok(files)
}

/// Extracts the dependencies from a proto file, skipping google/protobuf imports,
/// since these are provided by default by protoc.
fn extract_proto_dependencies(file: &Path) -> Result<Vec<PathBuf>, ProtoError> {
    let mut dependencies = Vec::new();
    let mut reader = BufReader::new(File::open(file)?);
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        if line.starts_with("import ") {
            if let Some(dependency) = line.split_whitespace().nth(1) {
                let dependency = dependency.to_string().replace([';', '\"'], "");
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

fn copy_files(plan: Vec<ProtoFileMapping<'_>>, proto_dir: PathBuf) -> Result<(), ProtoError> {
    for mapping in plan {
        trace!(
            "Copying proto file {} from {}/ for dependency {} ({})",
            &mapping.package_path.to_string_lossy(),
            &mapping.content_root.to_string_lossy(),
            &mapping.module,
            &mapping.coordinate
        );
        let source = mapping
            .worktree
            .join(&mapping.content_root)
            .join(&mapping.package_path);
        let target = proto_dir.join(&mapping.package_path);
        let prefix = target.parent().ok_or_else(|| {
            ProtoError::BadPath(format!(
                "Bad parent dest file for {}",
                &target.to_string_lossy()
            ))
        })?;
        std::fs::create_dir_all(prefix)?;
        std::fs::copy(source, target)?;
    }
    Ok(())
}

fn files_differ(a: &Path, b: &Path) -> Result<bool, ProtoError> {
    let a = std::fs::read(a)?;
    let b = std::fs::read(b)?;
    Ok(a != b)
}

struct Context<'m> {
    worktrees: HashMap<ModuleName, PathBuf>,
    modules: HashMap<ModuleName, &'m ResolvedModule>,
    // The plan of which files to copy from where to where
    plan: Vec<ProtoFileMapping<'m>>,
    // Maps source paths to their index in the plan.
    by_source: HashMap<PathBuf, usize>,
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

    fn worktree(&self, name: &ModuleName) -> &PathBuf {
        self.worktrees
            .get(name)
            .expect("Internal invariant violated: no worktree for the already resolved module")
    }

    fn add_mapping(&mut self, mapping: ProtoFileMapping<'m>) {
        let mut source_path = mapping.worktree.clone();
        source_path.push(&mapping.content_root);
        source_path.push(&mapping.package_path);

        if let Some(existing) = self.by_source.get(&source_path) {
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
        if let Some(existing) = self.by_target.get(&mapping.package_path) {
            let existing = &self.plan[*existing];
            if existing.worktree != mapping.worktree
                || existing.content_root != mapping.content_root
            {
                if files_differ(&existing.source_path(), &mapping.source_path()).unwrap_or(true) {
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
        }

        for prune_context in &mut self.active_prunes {
            prune_context.remaining.remove(&mapping.package_path);
            prune_context.seen.insert(mapping.package_path.clone());
        }

        let index = self.plan.len();
        self.by_source.insert(source_path, index);
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
