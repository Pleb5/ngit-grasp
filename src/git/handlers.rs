//! Git HTTP Protocol Handlers
//!
//! This module implements the HTTP handlers for Git Smart HTTP protocol.

use std::path::PathBuf;
use hyper::{body::Bytes, Response, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error, warn};

use super::protocol::{GitService, PktLine};
use super::subprocess::GitSubprocess;

/// Handle GET /info/refs?service=git-{upload,receive}-pack
///
/// This advertises the repository's refs to the client.
pub async fn handle_info_refs(
    repo_path: PathBuf,
    service: GitService,
) -> Result<Response<String>, GitError> {
    debug!("Handling info/refs for {:?} with service {:?}", repo_path, service);

    // Check if repository exists
    if !repo_path.exists() {
        warn!("Repository not found: {:?}", repo_path);
        return Err(GitError::RepositoryNotFound);
    }

    // Spawn git with --advertise-refs
    let mut git = GitSubprocess::spawn(service, &repo_path, true)
        .map_err(|e| {
            error!("Failed to spawn git process: {}", e);
            GitError::ProcessSpawnFailed(e)
        })?;

    // Read the output from git
    let mut output = Vec::new();
    if let Some(stdout) = git.take_stdout() {
        let mut stdout = stdout;
        stdout.read_to_end(&mut output).await
            .map_err(|e| {
                error!("Failed to read git output: {}", e);
                GitError::IoError(e)
            })?;
    }

    // Wait for process to complete
    let status = git.wait().await
        .map_err(|e| {
            error!("Failed to wait for git process: {}", e);
            GitError::IoError(e)
        })?;

    if !status.success() {
        error!("Git process failed with status: {:?}", status);
        return Err(GitError::GitFailed(status.code()));
    }

    // Build response with pkt-line header
    let mut response_body = Vec::new();
    
    // First line: service advertisement
    let service_line = format!("# service={}\n", service.as_str());
    response_body.extend_from_slice(&PktLine::data(service_line.as_bytes()).encode());
    response_body.extend_from_slice(&PktLine::flush().encode());
    
    // Then the git output
    response_body.extend_from_slice(&output);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", service.advertisement_content_type())
        .header("cache-control", "no-cache")
        .body(String::from_utf8_lossy(&response_body).to_string())
        .unwrap())
}

/// Handle POST /git-upload-pack (clone/fetch)
pub async fn handle_upload_pack(
    repo_path: PathBuf,
    request_body: Bytes,
) -> Result<Response<String>, GitError> {
    debug!("Handling upload-pack for {:?}", repo_path);

    if !repo_path.exists() {
        return Err(GitError::RepositoryNotFound);
    }

    // Spawn git upload-pack
    let mut git = GitSubprocess::spawn(GitService::UploadPack, &repo_path, false)
        .map_err(GitError::ProcessSpawnFailed)?;

    // Write request to git's stdin
    if let Some(mut stdin) = git.take_stdin() {
        stdin.write_all(&request_body).await
            .map_err(GitError::IoError)?;
        // Close stdin to signal end of input
        drop(stdin);
    }

    // Read response from git's stdout
    let mut output = Vec::new();
    if let Some(stdout) = git.take_stdout() {
        let mut stdout = stdout;
        stdout.read_to_end(&mut output).await
            .map_err(GitError::IoError)?;
    }

    // Wait for process
    let status = git.wait().await
        .map_err(GitError::IoError)?;

    if !status.success() {
        return Err(GitError::GitFailed(status.code()));
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", GitService::UploadPack.result_content_type())
        .header("cache-control", "no-cache")
        .body(String::from_utf8_lossy(&output).to_string())
        .unwrap())
}

/// Handle POST /git-receive-pack (push)
///
/// This includes an authorization hook point where GRASP validation will be added.
pub async fn handle_receive_pack(
    repo_path: PathBuf,
    request_body: Bytes,
) -> Result<Response<String>, GitError> {
    debug!("Handling receive-pack for {:?}", repo_path);

    if !repo_path.exists() {
        return Err(GitError::RepositoryNotFound);
    }

    // TODO: Add GRASP authorization here
    // For now, we'll accept all pushes to enable testing
    debug!("Authorization check would go here (currently accepting all pushes)");

    // Spawn git receive-pack
    let mut git = GitSubprocess::spawn(GitService::ReceivePack, &repo_path, false)
        .map_err(GitError::ProcessSpawnFailed)?;

    // Write request to git's stdin
    if let Some(mut stdin) = git.take_stdin() {
        stdin.write_all(&request_body).await
            .map_err(GitError::IoError)?;
        drop(stdin);
    }

    // Read response from git's stdout
    let mut output = Vec::new();
    if let Some(stdout) = git.take_stdout() {
        let mut stdout = stdout;
        stdout.read_to_end(&mut output).await
            .map_err(GitError::IoError)?;
    }

    // Wait for process
    let status = git.wait().await
        .map_err(GitError::IoError)?;

    if !status.success() {
        return Err(GitError::GitFailed(status.code()));
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", GitService::ReceivePack.result_content_type())
        .header("cache-control", "no-cache")
        .body(String::from_utf8_lossy(&output).to_string())
        .unwrap())
}

/// Errors that can occur in Git handlers
#[derive(Debug)]
pub enum GitError {
    RepositoryNotFound,
    ProcessSpawnFailed(std::io::Error),
    IoError(std::io::Error),
    GitFailed(Option<i32>),
    Unauthorized,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RepositoryNotFound => write!(f, "repository not found"),
            Self::ProcessSpawnFailed(e) => write!(f, "failed to spawn git process: {}", e),
            Self::IoError(e) => write!(f, "IO error: {}", e),
            Self::GitFailed(code) => write!(f, "git process failed with code: {:?}", code),
            Self::Unauthorized => write!(f, "unauthorized"),
        }
    }
}

impl std::error::Error for GitError {}

impl GitError {
    /// Convert to HTTP status code
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::RepositoryNotFound => StatusCode::NOT_FOUND,
            Self::Unauthorized => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}