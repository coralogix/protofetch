use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use log::{debug, info, trace};

use super::{error::GitBackendError, types::GitOid, GitBackend, GitRepository, WorktreeResult};

pub struct CliBackend {
    git_path: String,
}

impl CliBackend {
    pub fn new(git_path: String) -> Self {
        Self { git_path }
    }
}

pub struct CliRepository {
    repo_path: PathBuf,
    git_path: String,
}

fn base_git_cmd(git_path: &str) -> Command {
    let mut cmd = Command::new(git_path);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("LC_MESSAGES", "C");
    cmd.env_remove("LC_ALL");
    cmd.env_remove("LANGUAGE");
    cmd.args([
        "-c",
        "core.fsmonitor=false",
        "-c",
        "submodule.recurse=false",
    ]);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

impl CliRepository {
    fn git(&self) -> Command {
        let mut cmd = base_git_cmd(&self.git_path);
        cmd.arg("-C");
        cmd.arg(&self.repo_path);
        cmd
    }

    fn run(&self, cmd: &mut Command) -> Result<Vec<u8>, GitBackendError> {
        run_command(cmd)
    }

    fn run_str(&self, cmd: &mut Command) -> Result<String, GitBackendError> {
        run_command_str(cmd)
    }

    fn find_worktree(&self, name: &str) -> Result<Option<PathBuf>, GitBackendError> {
        let mut cmd = self.git();
        cmd.args(["worktree", "list", "--porcelain"]);
        let output = self.run(&mut cmd)?;
        parse_worktree_list(&output, name)
    }
}

fn run_command(cmd: &mut Command) -> Result<Vec<u8>, GitBackendError> {
    trace!("Running: {:?}", cmd);
    let output = cmd.output()?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let msg = if stderr.is_empty() { stdout } else { stderr };
        Err(GitBackendError::CommandFailed(msg))
    }
}

fn run_command_str(cmd: &mut Command) -> Result<String, GitBackendError> {
    let bytes = run_command(cmd)?;
    Ok(std::str::from_utf8(&bytes)?.trim().to_string())
}

fn optional_tag_refspec(refspec: &str) -> bool {
    let refspec = refspec.strip_prefix('+').unwrap_or(refspec);
    refspec.starts_with("refs/tags/")
}

fn should_retry_without_optional_tags(refspecs: &[String], message: &str) -> bool {
    refspecs.iter().any(|refspec| optional_tag_refspec(refspec))
        && refspecs
            .iter()
            .any(|refspec| !optional_tag_refspec(refspec))
        && message.contains("couldn't find remote ref refs/tags/")
}

#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> Result<PathBuf, GitBackendError> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    Ok(PathBuf::from(OsStr::from_bytes(bytes)))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: &[u8]) -> Result<PathBuf, GitBackendError> {
    Ok(PathBuf::from(std::str::from_utf8(bytes)?))
}

fn parse_worktree_list(output: &[u8], name: &str) -> Result<Option<PathBuf>, GitBackendError> {
    let lines: Vec<&[u8]> = output.split(|&b| b == b'\n').collect();
    for block in lines.split(|line| line.is_empty()) {
        let mut wt_path_bytes: Option<&[u8]> = None;
        let mut wt_branch: Option<&[u8]> = None;
        let mut wt_head: Option<&[u8]> = None;
        let mut detached = false;

        for &line in block {
            if let Some(path) = line.strip_prefix(b"worktree ") {
                wt_path_bytes = Some(path);
            } else if let Some(branch) = line.strip_prefix(b"branch refs/heads/") {
                wt_branch = Some(branch);
            } else if let Some(head) = line.strip_prefix(b"HEAD ") {
                wt_head = Some(head);
            } else if line == b"detached" {
                detached = true;
            }
        }

        if let Some(path_bytes) = wt_path_bytes {
            let branch_matches = wt_branch.map(|b| b == name.as_bytes()).unwrap_or(false);
            let head_matches = detached && wt_head.map(|h| h == name.as_bytes()).unwrap_or(false);

            if branch_matches || head_matches {
                return Ok(Some(bytes_to_path(path_bytes)?));
            }
        }
    }
    Ok(None)
}

impl GitRepository for CliRepository {
    fn remote_add(&self, name: &str, url: &str) -> Result<(), GitBackendError> {
        let mut cmd = self.git();
        cmd.args(["remote", "add", name, url]);
        self.run(&mut cmd)?;
        Ok(())
    }

    fn remote_get_url(&self, name: &str) -> Result<Option<String>, GitBackendError> {
        let mut cmd = self.git();
        cmd.args(["remote", "get-url", name]);
        trace!("Running: {:?}", cmd);
        let output = cmd.output()?;
        match output.status.code() {
            Some(0) => Ok(Some(
                std::str::from_utf8(&output.stdout)?.trim().to_string(),
            )),
            Some(2) => Ok(None),
            _ => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let msg = if stderr.is_empty() { stdout } else { stderr };
                Err(GitBackendError::CommandFailed(msg))
            }
        }
    }

    fn remote_set_url(&self, name: &str, url: &str) -> Result<(), GitBackendError> {
        let mut cmd = self.git();
        cmd.args(["remote", "set-url", name, url]);
        self.run(&mut cmd)?;
        Ok(())
    }

    fn fetch(&self, remote_name: &str, refspecs: &[String]) -> Result<(), GitBackendError> {
        debug!("Fetching {:?} from {}", refspecs, self.repo_path.display());
        let mut cmd = self.git();
        cmd.args(["fetch", "--no-tags"]);
        cmd.arg(remote_name);
        for refspec in refspecs {
            cmd.arg(refspec);
        }
        match self.run(&mut cmd) {
            Ok(_) => {}
            Err(GitBackendError::CommandFailed(message))
                if should_retry_without_optional_tags(refspecs, &message) =>
            {
                debug!(
                    "Retrying fetch without optional tag refspecs after missing tag: {}",
                    message
                );
                let mut cmd = self.git();
                cmd.args(["fetch", "--no-tags"]);
                cmd.arg(remote_name);
                for refspec in refspecs
                    .iter()
                    .filter(|refspec| !optional_tag_refspec(refspec))
                {
                    cmd.arg(refspec);
                }
                self.run(&mut cmd)?;
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    fn commit_exists(&self, oid: &str) -> Result<bool, GitBackendError> {
        if oid.is_empty() || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(GitBackendError::InvalidRef(format!(
                "Invalid commit oid: {}",
                oid
            )));
        }

        let mut cmd = self.git();
        cmd.args(["cat-file", "-t", oid]);
        match self.run_str(&mut cmd) {
            Ok(obj_type) => Ok(obj_type == "commit"),
            Err(GitBackendError::CommandFailed(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn revparse_commit(&self, spec: &str) -> Result<GitOid, GitBackendError> {
        let full_spec = format!("{}^{{commit}}", spec);
        let mut cmd = self.git();
        cmd.args(["rev-parse", &full_spec]);
        let hex = match self.run_str(&mut cmd) {
            Ok(hex) => hex,
            Err(GitBackendError::CommandFailed(_)) => {
                return Err(GitBackendError::InvalidRef(format!(
                    "Cannot resolve '{}' to a commit",
                    spec
                )));
            }
            Err(e) => return Err(e),
        };
        Ok(GitOid::from_hex(hex))
    }

    fn read_blob(&self, commit: &str, blob_path: &str) -> Result<Option<Vec<u8>>, GitBackendError> {
        let commit_spec = format!("{}^{{commit}}", commit);
        let mut cmd = self.git();
        cmd.args(["cat-file", "-e", &commit_spec]);
        self.run(&mut cmd)?;

        let mut cmd = self.git();
        cmd.args(["ls-tree", "--full-tree", commit, "--", blob_path]);
        if self.run(&mut cmd)?.is_empty() {
            return Ok(None);
        }

        let spec = format!("{}:{}", commit, blob_path);
        let mut cmd = self.git();
        cmd.args(["cat-file", "blob", &spec]);
        Ok(Some(self.run(&mut cmd)?))
    }

    fn is_ancestor(&self, ancestor: &GitOid, descendant: &GitOid) -> Result<bool, GitBackendError> {
        let mut cmd = self.git();
        cmd.args([
            "merge-base",
            "--is-ancestor",
            ancestor.as_str(),
            descendant.as_str(),
        ]);
        trace!("Running: {:?}", cmd);
        let output = cmd.output()?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let msg = if stderr.is_empty() { stdout } else { stderr };
                Err(GitBackendError::CommandFailed(msg))
            }
        }
    }

    fn create_worktree(
        &self,
        name: &str,
        worktree_path: &Path,
        commit: &str,
    ) -> Result<WorktreeResult, GitBackendError> {
        if let Some(existing_path) = self.find_worktree(name)? {
            let canonical = existing_path.canonicalize().map_err(|e| {
                GitBackendError::IO(std::io::Error::new(
                    e.kind(),
                    format!(
                        "Error while canonicalizing path {}: {}",
                        existing_path.display(),
                        e
                    ),
                ))
            })?;
            return Ok(WorktreeResult::Existing(
                canonical,
                Box::new(CliRepository {
                    repo_path: existing_path,
                    git_path: self.git_path.clone(),
                }),
            ));
        }

        info!(
            "Creating new worktree {} at {}.",
            name,
            worktree_path.display()
        );

        let mut cmd = self.git();
        cmd.args(["worktree", "add", "--detach"]);
        cmd.arg(worktree_path);
        cmd.arg(commit);
        self.run(&mut cmd)?;

        Ok(WorktreeResult::Created(Box::new(CliRepository {
            repo_path: worktree_path.to_path_buf(),
            git_path: self.git_path.clone(),
        })))
    }

    fn reset(&self, commit: &str) -> Result<(), GitBackendError> {
        let mut cmd = self.git();
        cmd.args(["reset", "--hard", commit]);
        self.run(&mut cmd)?;
        Ok(())
    }
}

impl GitBackend for CliBackend {
    fn init_bare(&self, path: &Path) -> Result<Box<dyn GitRepository>, GitBackendError> {
        trace!("Creating a new bare repository at {}", path.display());
        let mut cmd = base_git_cmd(&self.git_path);
        cmd.args(["init", "--bare"]);
        cmd.arg(path);
        run_command(&mut cmd)?;
        Ok(Box::new(CliRepository {
            repo_path: path.to_path_buf(),
            git_path: self.git_path.clone(),
        }))
    }

    fn open(&self, path: &Path) -> Result<Box<dyn GitRepository>, GitBackendError> {
        trace!("Opening existing repository at {}", path.display());
        let mut cmd = base_git_cmd(&self.git_path);
        cmd.arg("--git-dir");
        cmd.arg(path);
        cmd.args(["rev-parse", "--is-bare-repository"]);
        let is_bare = run_command_str(&mut cmd)
            .map_err(|_| GitBackendError::RepoNotFound(path.display().to_string()))?;
        if is_bare != "true" {
            return Err(GitBackendError::RepoNotFound(path.display().to_string()));
        }
        Ok(Box::new(CliRepository {
            repo_path: path.to_path_buf(),
            git_path: self.git_path.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn run_git(repo_path: &Path, args: &[&str]) -> String {
        let output = base_git_cmd("git")
            .arg("-C")
            .arg(repo_path)
            .args(args)
            .output()
            .expect("git command failed to start");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn repo_with_commit(path: &str) -> (tempfile::TempDir, CliRepository, String) {
        let tempdir = tempfile::tempdir().unwrap();
        run_git(tempdir.path(), &["init"]);

        let path = tempdir.path().join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"content").unwrap();

        run_git(tempdir.path(), &["add", "."]);
        run_git(
            tempdir.path(),
            &[
                "-c",
                "user.name=Protofetch Test",
                "-c",
                "user.email=protofetch@example.com",
                "commit",
                "-m",
                "initial",
            ],
        );
        let commit = run_git(tempdir.path(), &["rev-parse", "HEAD"]);
        let repo = CliRepository {
            repo_path: tempdir.path().to_path_buf(),
            git_path: "git".to_owned(),
        };
        (tempdir, repo, commit)
    }

    fn worktree_block(
        path: &[u8],
        branch: Option<&[u8]>,
        head: Option<&[u8]>,
        detached: bool,
    ) -> Vec<u8> {
        let mut block = Vec::new();
        block.extend_from_slice(b"worktree ");
        block.extend_from_slice(path);
        block.push(b'\n');
        if let Some(h) = head {
            block.extend_from_slice(b"HEAD ");
            block.extend_from_slice(h);
            block.push(b'\n');
        }
        if let Some(b) = branch {
            block.extend_from_slice(b"branch refs/heads/");
            block.extend_from_slice(b);
            block.push(b'\n');
        }
        if detached {
            block.extend_from_slice(b"detached\n");
        }
        block
    }

    fn join_blocks(blocks: &[Vec<u8>]) -> Vec<u8> {
        blocks.join(b"\n\n".as_ref())
    }

    #[test]
    fn find_worktree_by_branch_name() {
        let output = join_blocks(&[
            worktree_block(b"/repos/main", Some(b"main"), Some(b"abc123"), false),
            worktree_block(
                b"/repos/feature",
                Some(b"feature-xyz"),
                Some(b"def456"),
                false,
            ),
        ]);
        let result = parse_worktree_list(&output, "feature-xyz").unwrap();
        assert_eq!(result, Some(PathBuf::from("/repos/feature")));
    }

    #[test]
    fn find_worktree_by_detached_head() {
        let hash = "abc123def456abc123def456abc123def456abc123";
        let output = join_blocks(&[
            worktree_block(b"/repos/main", Some(b"main"), Some(b"deadbeef"), false),
            worktree_block(b"/repos/detached", None, Some(hash.as_bytes()), true),
        ]);
        let result = parse_worktree_list(&output, hash).unwrap();
        assert_eq!(result, Some(PathBuf::from("/repos/detached")));
    }

    #[test]
    fn find_worktree_returns_none_when_not_found() {
        let output = join_blocks(&[worktree_block(
            b"/repos/main",
            Some(b"main"),
            Some(b"abc123"),
            false,
        )]);
        let result = parse_worktree_list(&output, "nonexistent").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn missing_optional_tag_with_fallback_refspec_can_retry() {
        let refspecs = vec![
            "+refs/tags/deadbeef:refs/tags/deadbeef".to_owned(),
            "+refs/heads/*:refs/remotes/origin/*".to_owned(),
        ];

        assert!(should_retry_without_optional_tags(
            &refspecs,
            "fatal: couldn't find remote ref refs/tags/deadbeef"
        ));
    }

    #[test]
    fn missing_optional_tag_without_fallback_refspec_does_not_retry() {
        let refspecs = vec!["+refs/tags/v1:refs/tags/v1".to_owned()];

        assert!(!should_retry_without_optional_tags(
            &refspecs,
            "fatal: couldn't find remote ref refs/tags/v1"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn find_worktree_handles_non_utf8_path() {
        let non_utf8_path: &[u8] = b"/repos/caf\xe9dir";
        let output = join_blocks(&[worktree_block(
            non_utf8_path,
            Some(b"my-branch"),
            Some(b"abc123"),
            false,
        )]);
        let result = parse_worktree_list(&output, "my-branch").unwrap();
        use std::os::unix::ffi::OsStrExt;
        let expected = PathBuf::from(std::ffi::OsStr::from_bytes(non_utf8_path));
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn remote_get_url_returns_none_for_missing_remote() {
        let tempdir = tempfile::tempdir().unwrap();
        run_git(tempdir.path(), &["init"]);
        let repo = CliRepository {
            repo_path: tempdir.path().to_path_buf(),
            git_path: "git".to_owned(),
        };

        let url = repo.remote_get_url("origin").unwrap();

        assert_eq!(url, None);
    }

    #[test]
    fn remote_get_url_errors_for_command_failure() {
        let tempdir = tempfile::tempdir().unwrap();
        let repo = CliRepository {
            repo_path: tempdir.path().to_path_buf(),
            git_path: "git".to_owned(),
        };

        let result = repo.remote_get_url("origin");

        assert!(matches!(result, Err(GitBackendError::CommandFailed(_))));
    }

    #[test]
    fn open_succeeds_for_bare_repository() {
        let tempdir = tempfile::tempdir().unwrap();
        let backend = CliBackend::new("git".to_owned());

        backend.init_bare(tempdir.path()).unwrap();
        let result = backend.open(tempdir.path());

        assert!(result.is_ok());
    }

    #[test]
    fn open_rejects_directory_inside_parent_repository() {
        let tempdir = tempfile::tempdir().unwrap();
        run_git(tempdir.path(), &["init"]);
        let child = tempdir.path().join("cache/repo");
        fs::create_dir_all(&child).unwrap();
        let backend = CliBackend::new("git".to_owned());

        let result = backend.open(&child);

        assert!(matches!(result, Err(GitBackendError::RepoNotFound(_))));
    }

    #[test]
    fn commit_exists_errors_for_malformed_oid() {
        let (_tempdir, repo, _commit) = repo_with_commit("README.md");

        let result = repo.commit_exists("not-a-commit");

        assert!(matches!(result, Err(GitBackendError::InvalidRef(_))));
    }

    #[test]
    fn read_blob_returns_none_for_missing_path() {
        let (_tempdir, repo, commit) = repo_with_commit("README.md");

        let blob = repo.read_blob(&commit, "protofetch.toml").unwrap();

        assert_eq!(blob, None);
    }

    #[test]
    fn read_blob_errors_for_missing_commit() {
        let (_tempdir, repo, _commit) = repo_with_commit("README.md");

        let result = repo.read_blob(
            "0000000000000000000000000000000000000000",
            "protofetch.toml",
        );

        assert!(matches!(result, Err(GitBackendError::CommandFailed(_))));
    }

    #[test]
    fn read_blob_errors_when_path_is_tree() {
        let (_tempdir, repo, commit) = repo_with_commit("protofetch.toml/file");

        let result = repo.read_blob(&commit, "protofetch.toml");

        assert!(matches!(result, Err(GitBackendError::CommandFailed(_))));
    }
}
