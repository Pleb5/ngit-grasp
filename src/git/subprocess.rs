//! Git Subprocess Management
//!
//! This module provides utilities for spawning and managing Git subprocesses
//! for upload-pack and receive-pack operations.

use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::process::{Child, Command};

use super::protocol::GitService;

/// Git subprocess wrapper
pub struct GitSubprocess {
    child: Child,
}

impl GitSubprocess {
    /// Spawn a git subprocess for the given service and repository path
    ///
    /// # Arguments
    /// * `service` - The Git service (upload-pack or receive-pack)
    /// * `repo_path` - Path to the bare Git repository
    /// * `advertise` - If true, run with --advertise-refs flag
    pub fn spawn(
        service: GitService,
        repo_path: impl AsRef<Path>,
        advertise: bool,
    ) -> std::io::Result<Self> {
        let repo_path = repo_path.as_ref();
        
        let mut cmd = Command::new("git");
        cmd.arg(service.as_str());
        
        if advertise {
            cmd.arg("--advertise-refs");
        }
        
        cmd.arg("--stateless-rpc");
        cmd.arg(repo_path);
        
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        
        let child = cmd.spawn()?;
        
        Ok(Self { child })
    }

    /// Get a mutable reference to stdin
    pub fn stdin(&mut self) -> Option<&mut (impl AsyncWrite + Unpin)> {
        self.child.stdin.as_mut()
    }

    /// Get a mutable reference to stdout
    pub fn stdout(&mut self) -> Option<&mut (impl AsyncRead + Unpin)> {
        self.child.stdout.as_mut()
    }

    /// Get a mutable reference to stderr
    pub fn stderr(&mut self) -> Option<&mut (impl AsyncRead + Unpin)> {
        self.child.stderr.as_mut()
    }

    /// Take ownership of stdin
    pub fn take_stdin(&mut self) -> Option<impl AsyncWrite> {
        self.child.stdin.take()
    }

    /// Take ownership of stdout
    pub fn take_stdout(&mut self) -> Option<impl AsyncRead> {
        self.child.stdout.take()
    }

    /// Take ownership of stderr  
    pub fn take_stderr(&mut self) -> Option<impl AsyncRead> {
        self.child.stderr.take()
    }

    /// Wait for the subprocess to complete
    pub async fn wait(mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    /// Kill the subprocess
    pub async fn kill(&mut self) -> std::io::Result<()> {
        self.child.kill().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::process::Command as StdCommand;

    fn create_bare_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let status = StdCommand::new("git")
            .args(["init", "--bare"])
            .arg(dir.path())
            .status()
            .expect("Failed to run git init");
        assert!(status.success());
        dir
    }

    #[tokio::test]
    async fn test_spawn_upload_pack_advertise() {
        let repo = create_bare_repo();
        let mut proc = GitSubprocess::spawn(
            GitService::UploadPack,
            repo.path(),
            true,
        ).expect("Failed to spawn git");

        // Should have spawned successfully
        assert!(proc.stdout().is_some());
        assert!(proc.stdin().is_some());

        // Clean up
        let _ = proc.kill().await;
    }

    #[tokio::test]
    async fn test_spawn_receive_pack() {
        let repo = create_bare_repo();
        let mut proc = GitSubprocess::spawn(
            GitService::ReceivePack,
            repo.path(),
            false,
        ).expect("Failed to spawn git");

        assert!(proc.stdout().is_some());
        assert!(proc.stdin().is_some());

        let _ = proc.kill().await;
    }
}