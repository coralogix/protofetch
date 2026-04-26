use std::{
    path::{Path, PathBuf},
    process::Command,
};

use log::{debug, info, trace};

use super::{error::GitBackendError, types::GitOid, GitBackend, GitRepository, WorktreeResult};

pub struct BinaryBackend {
    git_path: String,
}

impl BinaryBackend {
    pub fn new() -> Self {
        Self {
            git_path: std::env::var("GIT_PATH").unwrap_or_else(|_| "git".to_string()),
        }
    }
}

pub struct BinaryRepository {
    repo_path: PathBuf,
    git_path: String,
}

impl BinaryRepository {
    fn git(&self) -> Command {
        let mut cmd = Command::new(&self.git_path);
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.arg("-C");
        cmd.arg(&self.repo_path);
        cmd
    }

    fn run(&self, cmd: &mut Command) -> Result<String, GitBackendError> {
        run_command(cmd)
    }
}

fn run_command(cmd: &mut Command) -> Result<String, GitBackendError> {
    trace!("Running: {:?}", cmd);
    let output = cmd.output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let msg = if stderr.is_empty() { stdout } else { stderr };
        Err(GitBackendError::CommandFailed(msg))
    }
}

impl GitRepository for BinaryRepository {
    fn remote_add(&self, name: &str, url: &str) -> Result<(), GitBackendError> {
        let mut cmd = self.git();
        cmd.args(["remote", "add", name, url]);
        self.run(&mut cmd)?;
        Ok(())
    }

    fn remote_get_url(&self, name: &str) -> Result<Option<String>, GitBackendError> {
        let mut cmd = self.git();
        cmd.args(["remote", "get-url", name]);
        match self.run(&mut cmd) {
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
        match self.run(&mut cmd) {
            Ok(obj_type) => Ok(obj_type == "commit"),
            Err(GitBackendError::CommandFailed(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn revparse_commit(&self, spec: &str) -> Result<GitOid, GitBackendError> {
        let full_spec = format!("{}^{{commit}}", spec);
        let mut cmd = self.git();
        cmd.args(["rev-parse", &full_spec]);
        let hex = self.run(&mut cmd).map_err(|_| {
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

        Ok(WorktreeResult::Created(Box::new(BinaryRepository {
            repo_path: worktree_path.to_path_buf(),
            git_path: self.git_path.clone(),
        })))
    }

    fn find_worktree(&self, name: &str) -> Result<Option<PathBuf>, GitBackendError> {
        let mut cmd = self.git();
        cmd.args(["worktree", "list", "--porcelain"]);
        let output = self.run(&mut cmd)?;

        // Parse porcelain output:
        // worktree /path/to/worktree
        // HEAD <hash>
        // branch refs/heads/<name>
        // detached
        // <empty line>
        for block in output.split("\n\n") {
            let mut wt_path = None;
            let mut wt_branch = None;
            let mut wt_head = None;
            let mut detached = false;
            for line in block.lines() {
                if let Some(path) = line.strip_prefix("worktree ") {
                    wt_path = Some(PathBuf::from(path));
                }
                if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                    wt_branch = Some(branch.to_string());
                }
                if let Some(head) = line.strip_prefix("HEAD ") {
                    wt_head = Some(head.to_string());
                }
                if line == "detached" {
                    detached = true;
                }
            }
            if let Some(path) = wt_path {
                if wt_branch.as_deref() == Some(name) {
                    return Ok(Some(path));
                }
                if detached && wt_head.as_deref() == Some(name) {
                    return Ok(Some(path));
                }
            }
        }

        Ok(None)
    }

    fn reset(&self, commit: &str) -> Result<(), GitBackendError> {
        let mut cmd = self.git();
        cmd.args(["reset", "--hard", commit]);
        self.run(&mut cmd)?;
        Ok(())
    }
}

impl GitBackend for BinaryBackend {
    fn init_bare(&self, path: &Path) -> Result<Box<dyn GitRepository>, GitBackendError> {
        trace!("Creating a new bare repository at {}", path.display());
        let mut cmd = Command::new(&self.git_path);
        cmd.args(["init", "--bare"]);
        cmd.arg(path);
        run_command(&mut cmd)?;
        Ok(Box::new(BinaryRepository {
            repo_path: path.to_path_buf(),
            git_path: self.git_path.clone(),
        }))
    }

    fn open(&self, path: &Path) -> Result<Box<dyn GitRepository>, GitBackendError> {
        trace!("Opening existing repository at {}", path.display());
        let mut cmd = Command::new(&self.git_path);
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.arg("-C");
        cmd.arg(path);
        cmd.args(["rev-parse", "--git-dir"]);
        run_command(&mut cmd)
            .map_err(|_| GitBackendError::RepoNotFound(path.display().to_string()))?;
        Ok(Box::new(BinaryRepository {
            repo_path: path.to_path_buf(),
            git_path: self.git_path.clone(),
        }))
    }
}
