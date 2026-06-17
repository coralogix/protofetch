//! End-to-end test infrastructure.
//! Provides helpers to create real local git repositories, run the full
//! protofetch fetch pipeline, and snapshot the output directory.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use git2::{build::CheckoutBuilder, IndexAddOption, Repository, Signature};
use insta::{assert_snapshot, Settings};
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

struct FixtureCommit {
    repo: String,
    branch: String,
    index: usize,
    path: PathBuf,
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

    /// Run a file-backed end-to-end fixture from `tests/e2e/<name>`.
    fn run(name: &str, lock_mode: LockMode) -> FetchResult {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/e2e")
            .join(name);
        let mut world = Self::new();
        world.load_fixture_repos(&fixture);

        let manifest = fs::read_to_string(fixture.join("protofetch.toml"))
            .expect("read fixture protofetch.toml");
        let initial_lock = fs::read_to_string(fixture.join("protofetch.lock")).ok();
        let result = world.fetch_files(&manifest, initial_lock.as_deref(), lock_mode);

        let mut settings = Settings::clone_current();
        settings.set_snapshot_path(fixture.join("snapshots"));
        settings.set_prepend_module_to_snapshot(false);
        settings.set_omit_expression(true);
        settings.bind(|| {
            assert_snapshot!("output", result.snapshot_tree());
            assert_snapshot!("lockfile", result.snapshot_lockfile());
        });

        result
    }

    fn load_fixture_repos(&mut self, fixture: &Path) {
        let mut commits = Vec::new();
        collect_fixture_commits(fixture, fixture, &mut commits);
        commits.sort_by(|a, b| {
            a.repo
                .cmp(&b.repo)
                .then_with(|| a.index.cmp(&b.index))
                .then_with(|| a.branch.cmp(&b.branch))
        });

        let mut created = BTreeSet::new();
        for commit in commits {
            let files = read_fixture_files(&commit.path)
                .into_iter()
                .map(|(path, content)| {
                    if path == "protofetch.toml" {
                        (path, prepare_manifest(&content, self.remotes.path()))
                    } else {
                        (path, content)
                    }
                })
                .collect::<Vec<_>>();
            let files = files
                .iter()
                .map(|(path, content)| (path.as_str(), content.as_str()))
                .collect::<Vec<_>>();

            if created.insert(commit.repo.clone()) {
                assert_eq!(
                    commit.index, 1,
                    "first fixture commit for {} must use commit index 1",
                    commit.repo
                );
                assert_eq!(
                    commit.branch, "main",
                    "first fixture commit for {} must be on main",
                    commit.repo
                );
                self.create_repo(&commit.repo, &files);
            } else {
                self.repo_mut(&commit.repo)
                    .add_commit(&commit.branch, &files);
            }
        }
    }

    fn repo_mut(&mut self, name: &str) -> &mut TestRepo {
        let path = self.remotes.path().join(name);
        self.repos
            .iter_mut()
            .find(|repo| repo.path == path)
            .expect("fixture repo exists")
    }

    /// Create a local git repository at `<remotes>/<name>` containing the
    /// given files and return its absolute path + commit info.
    ///
    /// `name` is a relative path such as `"repo1"` — it may include
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

    fn fetch_files(
        &self,
        manifest: &str,
        initial_lock: Option<&str>,
        lock_mode: LockMode,
    ) -> FetchResult {
        fs::write(
            self.project.path().join("protofetch.toml"),
            resolve_labels(
                &prepare_manifest(manifest, self.remotes.path()),
                self.remotes.path(),
                &self.repos,
            ),
        )
        .expect("write protofetch.toml");

        if let Some(initial_lock) = initial_lock {
            fs::write(
                self.project.path().join("protofetch.lock"),
                resolve_labels(initial_lock, self.remotes.path(), &self.repos),
            )
            .expect("write initial protofetch.lock");
        }

        self.fetch_project(lock_mode)
    }

    fn fetch_project(&self, lock_mode: LockMode) -> FetchResult {
        let pf = Protofetch::builder()
            .root(self.project.path().to_path_buf())
            .cache_directory(self.cache.path().to_path_buf())
            .jobs(4)
            .copy_jobs(2)
            .try_build()
            .expect("build Protofetch");

        pf.fetch(lock_mode).expect("protofetch fetch");

        let commits = self
            .repos
            .iter()
            .flat_map(|repo| repo.commits.iter().cloned())
            .collect::<Vec<_>>();
        let output_dir = self.project.path().join("proto_src");
        let lock_path = self.project.path().join("protofetch.lock");
        let remotes_path = self.remotes.path().to_path_buf();

        FetchResult {
            output_snapshot: snapshot_tree(&output_dir),
            lockfile_snapshot: snapshot_lockfile(&lock_path, &remotes_path, &commits),
        }
    }
}

pub fn run(name: &str) -> FetchResult {
    TestWorld::run(name, LockMode::Update)
}

pub fn run_locked(name: &str) -> FetchResult {
    TestWorld::run(name, LockMode::Locked)
}

fn prepare_manifest(manifest: &str, remotes_path: &Path) -> String {
    let mut manifest = manifest
        .parse::<toml::Table>()
        .expect("parse fixture manifest");
    let base = remotes_path.to_string_lossy().replace('\\', "/");

    let reserved = ["name", "description", "proto_out_dir"];
    for (key, value) in manifest.iter_mut() {
        if reserved.contains(&key.as_str()) {
            continue;
        }
        if let toml::Value::Table(dep) = value {
            dep.entry("protocol")
                .or_insert_with(|| toml::Value::String("file".to_string()));
            if let Some(toml::Value::String(url)) = dep.get_mut("url") {
                if url.starts_with("<base>/") {
                    *url = url.replacen("<base>", &base, 1);
                } else {
                    *url = format!("{base}/{url}");
                }
            }
        }
    }

    toml::to_string_pretty(&manifest).expect("serialize fixture manifest")
}

fn collect_fixture_commits(fixture: &Path, dir: &Path, commits: &mut Vec<FixtureCommit>) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };

    for entry in read_dir.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("snapshots") {
            continue;
        }
        if let Some(index) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse().ok())
        {
            let branch_path = path.parent().expect("commit has branch parent");
            let repo_path = branch_path.parent().expect("branch has repo parent");
            let branch = branch_path
                .file_name()
                .expect("branch name")
                .to_string_lossy()
                .into_owned();
            let repo = repo_path
                .strip_prefix(fixture)
                .expect("repo under fixture")
                .to_string_lossy()
                .replace('\\', "/");
            commits.push(FixtureCommit {
                repo,
                branch,
                index,
                path,
            });
        } else {
            collect_fixture_commits(fixture, &path, commits);
        }
    }
}

fn read_fixture_files(dir: &Path) -> Vec<(String, String)> {
    let mut entries = BTreeMap::new();
    collect_entries(dir, dir, &mut entries);
    entries.into_iter().collect()
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
pub struct FetchResult {
    output_snapshot: String,
    lockfile_snapshot: String,
}

impl FetchResult {
    /// Walk the output directory and produce a single deterministic snapshot string.
    ///
    /// Every file under `output_dir` is rendered as:
    /// ```text
    /// === relative/path/to/file ===
    /// <file contents>
    /// ```
    /// Files are visited in sorted order so the snapshot is stable.
    pub fn snapshot_tree(&self) -> String {
        self.output_snapshot.clone()
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
        self.lockfile_snapshot.clone()
    }
}

pub fn assert_output_contains(result: &FetchResult, paths: &[&str]) {
    let snapshot = result.snapshot_tree();
    for path in paths {
        assert!(
            snapshot.contains(&format!("=== {path} ===")),
            "expected output to contain {path}\n\n{snapshot}"
        );
    }
}

pub fn assert_output_excludes(result: &FetchResult, paths: &[&str]) {
    let snapshot = result.snapshot_tree();
    for path in paths {
        assert!(
            !snapshot.contains(&format!("=== {path} ===")),
            "expected output to exclude {path}\n\n{snapshot}"
        );
    }
}

fn snapshot_tree(output_dir: &Path) -> String {
    let mut entries: BTreeMap<String, String> = BTreeMap::new();
    collect_entries(output_dir, output_dir, &mut entries);

    entries
        .iter()
        .map(|(rel, content)| format!("=== {} ===\n{}", rel, content.trim_end_matches('\n')))
        .collect::<Vec<_>>()
        .join("\n\n")
        + "\n"
}

fn snapshot_lockfile(
    lock_path: &Path,
    remotes_path: &Path,
    commits: &[(String, String)],
) -> String {
    let mut hash_to_label: BTreeMap<&str, String> = BTreeMap::new();
    let mut branch_counter: BTreeMap<&str, usize> = BTreeMap::new();
    for (branch, hash) in commits {
        let n = branch_counter.entry(branch.as_str()).or_insert(0);
        *n += 1;
        hash_to_label.insert(hash.as_str(), format!("<commit:{branch}:{n}>"));
    }

    let content = fs::read_to_string(lock_path).expect("read protofetch.lock");
    let base = remotes_path.to_string_lossy().replace('\\', "/");
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
