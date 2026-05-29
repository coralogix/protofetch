//! End-to-end test infrastructure for `git-file-protocol` tests.
//!
//! This module is only compiled when the `git-file-protocol` feature is active.
//! It provides helpers to:
//! - Create real local git repositories with committed files (`TestRepo`).
//! - Run the full protofetch fetch pipeline against them (`TestWorld`).
//! - Snapshot the output directory as a single deterministic string for use
//!   with `insta` (`snapshot_tree`).

#![cfg(feature = "git-file-protocol")]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use git2::{build::CheckoutBuilder, IndexAddOption, Repository, Signature};
use protofetch::{LockMode, Protofetch};
use tempfile::TempDir;

/// A local git repository created by [`TestWorld::create_repo`].
pub struct TestRepo {
    /// Absolute filesystem path to the repository.
    path: PathBuf,
    /// Base path of the remotes temp dir — used to expand `<base>` in file content.
    remotes_path: PathBuf,
    /// All commits added so far, in order: `(branch, commit_hash)`.
    commits: Vec<(String, String)>,
}

impl TestRepo {
    /// Add a commit on `branch`, writing `files` into the working tree.
    ///
    /// If `branch` does not yet exist it is created from the most recent
    /// commit recorded in this repo (i.e. the tip of the last-used branch).
    /// Returns `&mut Self` for chaining.
    pub fn add_commit(&mut self, branch: &str, files: &[(&str, &str)]) -> &mut Self {
        let repo = Repository::open(&self.path).expect("open repo");
        let branch_ref = format!("refs/heads/{branch}");

        let parent = if let Ok(b) = repo.find_branch(branch, git2::BranchType::Local) {
            b.get().peel_to_commit().expect("peel to commit")
        } else {
            let (_, last_hash) = self.commits.last().expect("need at least one commit");
            let oid = git2::Oid::from_str(last_hash).expect("parse oid");
            let last = repo.find_commit(oid).expect("find commit");
            repo.branch(branch, &last, false).expect("create branch");
            last
        };

        repo.set_head(&branch_ref).expect("set HEAD");
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        repo.checkout_head(Some(&mut checkout))
            .expect("checkout branch");

        let base = self.remotes_path.to_string_lossy().replace('\\', "/");
        for (rel_path, content) in files {
            let abs = self.path.join(rel_path);
            if let Some(p) = abs.parent() {
                fs::create_dir_all(p).expect("create dir");
            }
            fs::write(&abs, content.replace("<base>", &base)).expect("write file");
        }

        let mut index = repo.index().expect("repo index");
        index
            .add_all(["*"], IndexAddOption::DEFAULT, None)
            .expect("git add");
        index.write().expect("write index");
        let tree_oid = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_oid).expect("find tree");

        let sig = Signature::now("Test", "test@example.com").expect("signature");
        let commit_oid = repo
            .commit(Some(&branch_ref), &sig, &sig, "commit", &tree, &[&parent])
            .expect("commit");

        self.commits
            .push((branch.to_string(), commit_oid.to_string()));
        self
    }
}

/// Owns all temporary directories for one end-to-end test scenario.
pub struct TestWorld {
    /// Temp dir holding the "remote" source repos.
    remotes: TempDir,
    /// Temp dir used as the protofetch project root.
    project: TempDir,
    /// Temp dir used as the protofetch cache.
    cache: TempDir,
    /// All repos created by [`TestWorld::create_repo`], in creation order.
    repos: Vec<TestRepo>,
}

impl TestWorld {
    pub fn new() -> Self {
        Self {
            remotes: TempDir::new().expect("remotes TempDir"),
            project: TempDir::new().expect("project TempDir"),
            cache: TempDir::new().expect("cache TempDir"),
            repos: Vec::new(),
        }
    }

    /// Create a local git repository at `<remotes>/<name>` containing the
    /// given files and return its absolute path + commit info.
    ///
    /// `name` is a relative path such as `"org/repo1"` — it may include
    /// subdirectory components; the final directory becomes the repo root.
    ///
    /// `files` is a slice of `(relative-path-inside-repo, content)` pairs.
    pub fn create_repo(&mut self, name: &str, files: &[(&str, &str)]) -> &mut TestRepo {
        let repo_path = self.remotes.path().join(name);
        fs::create_dir_all(&repo_path).expect("create repo dir");

        let repo = Repository::init(&repo_path).expect("git init");

        // Write files, expanding <base> to the remotes root path.
        let base = self.remotes.path().to_string_lossy().replace('\\', "/");
        for (rel_path, content) in files {
            let abs = repo_path.join(rel_path);
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent).expect("create file parent dir");
            }
            fs::write(&abs, content.replace("<base>", &base)).expect("write file");
        }

        // Stage all files.
        let mut index = repo.index().expect("repo index");
        index
            .add_all(["*"], IndexAddOption::DEFAULT, None)
            .expect("git add");
        index.write().expect("write index");
        let tree_oid = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_oid).expect("find tree");

        // Commit.
        let sig = Signature::now("Test", "test@example.com").expect("signature");
        let commit_oid = repo
            .commit(
                Some("refs/heads/main"),
                &sig,
                &sig,
                "Initial commit",
                &tree,
                &[],
            )
            .expect("commit");

        self.repos.push(TestRepo {
            path: repo_path,
            remotes_path: self.remotes.path().to_path_buf(),
            commits: vec![("main".to_string(), commit_oid.to_string())],
        });
        self.repos.last_mut().unwrap()
    }

    /// Run protofetch fetch with no pre-existing lock file (`LockMode::Update`).
    ///
    /// Any `file`-protocol dependency's `url` is prefixed with the remotes base
    /// path automatically, so tests can write logical names like `"org/repo1"`.
    pub fn fetch(&self, manifest: toml::Table) -> FetchResult<'_> {
        self.fetch_impl(manifest, LockMode::Update, None)
    }

    /// Run protofetch fetch with a pre-existing lock file.
    ///
    /// `initial_lock` uses the same notation as [`FetchResult::snapshot_lockfile`]:
    /// `<base>` for the remotes path and `<commit:branch:N>` for commit hashes.
    /// Labels are resolved using all repos in this world, in creation order.
    pub fn fetch_with_initial_lock(
        &self,
        manifest: toml::Table,
        lock_mode: LockMode,
        initial_lock: toml::Table,
    ) -> FetchResult<'_> {
        self.fetch_impl(manifest, lock_mode, Some(initial_lock))
    }

    fn fetch_impl(
        &self,
        mut manifest: toml::Table,
        lock_mode: LockMode,
        initial_lock: Option<toml::Table>,
    ) -> FetchResult<'_> {
        let base = self.remotes.path().to_string_lossy().replace('\\', "/");

        // Pre-process manifest: inject `protocol = "file"` and prefix urls with
        // the remotes base path for every dependency entry.
        let reserved = ["name", "description", "proto_out_dir"];
        for (key, value) in manifest.iter_mut() {
            if reserved.contains(&key.as_str()) {
                continue;
            }
            if let toml::Value::Table(dep) = value {
                dep.entry("protocol")
                    .or_insert_with(|| toml::Value::String("file".to_string()));
                if let Some(toml::Value::String(url)) = dep.get_mut("url") {
                    *url = format!("{}/{}", base, url);
                }
            }
        }
        let raw = toml::to_string_pretty(&manifest).expect("serialize manifest");
        fs::write(
            self.project.path().join("protofetch.toml"),
            resolve_labels(&raw, self.remotes.path(), &self.repos),
        )
        .expect("write protofetch.toml");

        // Pre-process and write initial lock file if provided.
        if let Some(lock) = initial_lock {
            let raw = toml::to_string_pretty(&lock).expect("serialize lock");
            let resolved = resolve_labels(&raw, self.remotes.path(), &self.repos);
            fs::write(self.project.path().join("protofetch.lock"), resolved)
                .expect("write initial protofetch.lock");
        }

        let pf = Protofetch::builder()
            .root(self.project.path().to_path_buf())
            .cache_directory(self.cache.path().to_path_buf())
            .jobs(4)
            .copy_jobs(2)
            .try_build()
            .expect("build Protofetch");

        pf.fetch(lock_mode).expect("protofetch fetch");

        FetchResult {
            output_dir: self.project.path().join("proto_src"),
            lock_path: self.project.path().join("protofetch.lock"),
            world: self,
        }
    }
}

/// Resolve snapshot labels back to real values so a labelled lock file can be
/// written to disk as input to protofetch.
///
/// - `<base>` → actual remotes path
/// - `<commit:branch:N>` → actual commit hash (using the same per-branch
///   counter as [`FetchResult::lockfile_snapshot`])
fn resolve_labels(content: &str, remotes_path: &Path, repos: &[TestRepo]) -> String {
    let base = remotes_path.to_string_lossy().replace('\\', "/");
    let mut result = content.replace("<base>", &base);

    let mut branch_counter: BTreeMap<&str, usize> = BTreeMap::new();
    for repo in repos {
        for (branch, hash) in &repo.commits {
            let n = branch_counter.entry(branch.as_str()).or_insert(0);
            *n += 1;
            let label = format!("<commit:{branch}:{n}>");
            result = result.replace(&label, hash);
        }
    }
    result
}

/// Returned by [`TestWorld::fetch`]; provides access to all fetch outputs.
pub struct FetchResult<'w> {
    output_dir: PathBuf,
    lock_path: PathBuf,
    world: &'w TestWorld,
}

impl<'w> FetchResult<'w> {
    /// Walk the output directory and produce a single deterministic snapshot string.
    ///
    /// Every file under `output_dir` is rendered as:
    /// ```text
    /// === relative/path/to/file ===
    /// <file contents>
    /// ```
    /// Files are visited in sorted order so the snapshot is stable.
    pub fn snapshot_tree(&self) -> String {
        let mut entries: BTreeMap<String, String> = BTreeMap::new();
        collect_entries(&self.output_dir, &self.output_dir, &mut entries);

        let mut out = String::new();
        for (rel, content) in &entries {
            out.push_str(&format!("=== {} ===\n{}\n", rel, content));
        }
        out
    }

    /// Read the lock file and return a stable snapshot string.
    ///
    /// Two sources of non-determinism are redacted:
    /// - `url` values: the dynamic temp-dir prefix is replaced with `<base>`.
    /// - `commit_hash` / `revision` values: replaced with a deterministic label
    ///   derived from the commit's position across all repos in the world.
    ///
    /// Labels have the form `<commit:<branch>:<N>>` where N is a 1-based
    /// per-branch counter across all repos in creation order.  Unknown hashes
    /// are labelled `<commit:unknown>`.
    pub fn snapshot_lockfile(&self) -> String {
        let mut hash_to_label: BTreeMap<&str, String> = BTreeMap::new();
        let mut branch_counter: BTreeMap<&str, usize> = BTreeMap::new();
        for repo in &self.world.repos {
            for (branch, hash) in &repo.commits {
                let n = branch_counter.entry(branch.as_str()).or_insert(0);
                *n += 1;
                hash_to_label.insert(hash.as_str(), format!("<commit:{branch}:{n}>"));
            }
        }

        let content = fs::read_to_string(&self.lock_path).expect("read protofetch.lock");
        let base = self
            .world
            .remotes
            .path()
            .to_string_lossy()
            .replace('\\', "/");
        content
            .lines()
            .map(|line| {
                for prefix in ["commit_hash = \"", "revision = \""] {
                    if let Some(rest) = line.strip_prefix(prefix) {
                        let hash = rest.trim_end_matches('"');
                        let label = hash_to_label
                            .get(hash)
                            .cloned()
                            .unwrap_or_else(|| "<commit:unknown>".to_string());
                        let key = prefix.trim_end_matches(" = \"");
                        return format!("{key} = \"{label}\"");
                    }
                }
                line.replace(base.as_str(), "<base>")
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }
}

fn collect_entries(base: &Path, dir: &Path, entries: &mut BTreeMap<String, String>) {
    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    // Collect and sort for determinism.
    let mut children: Vec<PathBuf> = read_dir.filter_map(|e| e.ok().map(|e| e.path())).collect();
    children.sort();

    for path in children {
        if path.is_dir() {
            collect_entries(base, &path, entries);
        } else {
            let rel = path
                .strip_prefix(base)
                .expect("strip prefix")
                .to_string_lossy()
                .replace('\\', "/");
            let content = fs::read_to_string(&path).unwrap_or_else(|_| "<binary>".to_string());
            entries.insert(rel, content);
        }
    }
}
