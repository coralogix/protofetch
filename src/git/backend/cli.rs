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
        match self.run_str(&mut cmd) {
            Ok(url) => Ok(Some(url)),
            Err(GitBackendError::CommandFailed(_)) => Ok(None),
            Err(e) => Err(e),
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
        cmd.arg("fetch");
        cmd.arg(remote_name);
        for refspec in refspecs {
            cmd.arg(refspec);
        }
        self.run(&mut cmd)?;
        Ok(())
    }

    fn commit_exists(&self, oid: &str) -> Result<bool, GitBackendError> {
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
        let hex = self.run_str(&mut cmd).map_err(|_| {
            GitBackendError::InvalidRef(format!("Cannot resolve '{}' to a commit", spec))
        })?;
        Ok(GitOid::from_hex(hex))
    }

    fn read_blob(&self, commit: &str, blob_path: &str) -> Result<Option<Vec<u8>>, GitBackendError> {
        let spec = format!("{}:{}", commit, blob_path);
        let mut cmd = self.git();
        cmd.args(["cat-file", "blob", &spec]);
        trace!("Running: {:?}", cmd);
        let output = cmd.output()?;
        if output.status.success() {
            Ok(Some(output.stdout))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not a valid object name")
                || stderr.contains("does not exist")
                || stderr.contains("Not a valid object name")
            {
                Ok(None)
            } else if output.status.code() == Some(128) {
                // git cat-file exits with 128 when the object is not found
                Ok(None)
            } else {
                Err(GitBackendError::CommandFailed(stderr.trim().to_string()))
            }
        }
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
            return Ok(WorktreeResult::Existing(canonical));
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

    fn find_worktree(&self, name: &str) -> Result<Option<PathBuf>, GitBackendError> {
        let mut cmd = self.git();
        cmd.args(["worktree", "list", "--porcelain"]);
        let output = self.run(&mut cmd)?;
        parse_worktree_list(&output, name)
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
        cmd.arg("-C");
        cmd.arg(path);
        cmd.args(["rev-parse", "--git-dir"]);
        run_command(&mut cmd)
            .map_err(|_| GitBackendError::RepoNotFound(path.display().to_string()))?;
        Ok(Box::new(CliRepository {
            repo_path: path.to_path_buf(),
            git_path: self.git_path.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
