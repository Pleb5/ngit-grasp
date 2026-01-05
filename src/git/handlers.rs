//! Git HTTP Protocol Handlers
//!
//! This module implements the HTTP handlers for Git Smart HTTP protocol.

use http_body_util::Full;
use hyper::{body::Bytes, Response, StatusCode};
use nostr_relay_builder::LocalRelay;
use nostr_sdk::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error, info, warn};

use super::protocol::{GitService, PktLine};
use super::subprocess::GitSubprocess;
use super::try_set_head_if_available;

use crate::git::authorization::{authorize_push, fetch_repository_data};
use crate::git::sync::sync_to_owner_repos;
use crate::nostr::builder::SharedDatabase;
use crate::nostr::events::{KIND_PR, KIND_PR_UPDATE, KIND_REPOSITORY_STATE};
use crate::purgatory::Purgatory;

/// Handle GET /info/refs?service=git-{upload,receive}-pack
///
/// This advertises the repository's refs to the client.
pub async fn handle_info_refs(
    repo_path: PathBuf,
    service: GitService,
) -> Result<Response<Full<Bytes>>, GitError> {
    debug!(
        "Handling info/refs for {:?} with service {:?}",
        repo_path, service
    );

    // Check if repository exists
    if !repo_path.exists() {
        warn!("Repository not found: {:?}", repo_path);
        return Err(GitError::RepositoryNotFound);
    }

    // Spawn git with --advertise-refs
    let mut git = GitSubprocess::spawn(service, &repo_path, true).map_err(|e| {
        error!("Failed to spawn git process: {}", e);
        GitError::ProcessSpawnFailed(e)
    })?;

    // Read the output from git
    let mut output = Vec::new();
    let mut stderr_output = Vec::new();

    if let Some(stdout) = git.take_stdout() {
        let mut stdout = stdout;
        stdout.read_to_end(&mut output).await.map_err(|e| {
            error!("Failed to read git output: {}", e);
            GitError::IoError(e)
        })?;
    }

    if let Some(stderr) = git.take_stderr() {
        let mut stderr = stderr;
        stderr.read_to_end(&mut stderr_output).await.map_err(|e| {
            error!("Failed to read git stderr: {}", e);
            GitError::IoError(e)
        })?;
    }

    // Wait for process to complete
    let status = git.wait().await.map_err(|e| {
        error!("Failed to wait for git process: {}", e);
        GitError::IoError(e)
    })?;

    if !status.success() {
        let stderr_str = String::from_utf8_lossy(&stderr_output);
        error!(
            "Git process failed with status: {:?}, stderr: {}",
            status, stderr_str
        );
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
        .body(Full::new(Bytes::from(response_body)))
        .unwrap())
}

/// Handle POST /git-upload-pack (clone/fetch)
pub async fn handle_upload_pack(
    repo_path: PathBuf,
    request_body: Bytes,
) -> Result<Response<Full<Bytes>>, GitError> {
    debug!("Handling upload-pack for {:?}", repo_path);

    if !repo_path.exists() {
        return Err(GitError::RepositoryNotFound);
    }

    // Spawn git upload-pack
    let mut git = GitSubprocess::spawn(GitService::UploadPack, &repo_path, false)
        .map_err(GitError::ProcessSpawnFailed)?;

    // Write request to git's stdin
    if let Some(mut stdin) = git.take_stdin() {
        stdin
            .write_all(&request_body)
            .await
            .map_err(GitError::IoError)?;
        // Close stdin to signal end of input
        drop(stdin);
    }

    // Read response from git's stdout
    let mut output = Vec::new();
    let mut stderr_output = Vec::new();

    if let Some(stdout) = git.take_stdout() {
        let mut stdout = stdout;
        stdout
            .read_to_end(&mut output)
            .await
            .map_err(GitError::IoError)?;
    }

    if let Some(stderr) = git.take_stderr() {
        let mut stderr = stderr;
        stderr
            .read_to_end(&mut stderr_output)
            .await
            .map_err(GitError::IoError)?;
    }

    // Wait for process
    let status = git.wait().await.map_err(GitError::IoError)?;

    if !status.success() {
        let stderr_str = String::from_utf8_lossy(&stderr_output);
        error!("Git upload-pack failed: {}", stderr_str);
        return Err(GitError::GitFailed(status.code()));
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", GitService::UploadPack.result_content_type())
        .header("cache-control", "no-cache")
        .body(Full::new(Bytes::from(output)))
        .unwrap())
}

/// Handle POST /git-receive-pack (push)
///
/// This includes GRASP authorization validation according to GRASP-01:
/// "MUST accept pushes via this service that match the latest repo state announcement
/// on the relay, respecting the recursive maintainer set."
///
/// Also per GRASP-01: "MUST set repository HEAD per repository state announcement
/// as soon as the git data related to that branch has been received."
///
/// Also purgatory GRASP-01: "Accepted repo state announcements, PRs and PR Updates
/// SHOULD be accepted with message "purgatory: won't be served until git data arrives"
/// and kepted in purgatory (not served) until the related git data arrives and
/// otherwise discarded after 30 minutes."
///
/// # Arguments
/// * `repo_path` - Path to the bare git repository
/// * `request_body` - The git pack data from the client
/// * `database` - Database reference for authorization queries
/// * `identifier` - The repository identifier (d tag) for authorization lookup
/// * `owner_pubkey` - The owner's public key (hex) from the URL path, scoping authorization
/// * `git_data_path` - Base path for git repositories (for syncing to other owner repos)
pub async fn handle_receive_pack(
    repo_path: PathBuf,
    request_body: Bytes,
    database: SharedDatabase,
    relay: LocalRelay,
    identifier: &str,
    owner_pubkey: &str,
    purgatory: Arc<Purgatory>,
    git_data_path: &str,
) -> Result<Response<Full<Bytes>>, GitError> {
    debug!("Handling receive-pack for {:?}", repo_path);

    if !repo_path.exists() {
        return Err(GitError::RepositoryNotFound);
    }

    // GRASP Authorization Check
    info!(
        "Authorizing push for {} owned by {} via database query",
        identifier, owner_pubkey
    );

    // check push is authorised
    let auth_result = match authorize_push(
        &database,
        identifier,
        owner_pubkey,
        &request_body,
        &purgatory,
        &repo_path,
    )
    .await
    {
        Ok(auth_result) => {
            if !auth_result.authorized {
                warn!("Push rejected for {}: {}", identifier, auth_result.reason);
                return Err(GitError::Unauthorized);
            }
            info!(
                "Push authorized for {} - {} maintainers, {} purgatory events: {}",
                identifier,
                auth_result.maintainers.len(),
                auth_result.purgatory_events.len(),
                auth_result.reason
            );
            auth_result
        }
        Err(e) => {
            warn!("Authorization check failed for {}: {}", identifier, e);
            return Err(GitError::Unauthorized);
        }
    };

    // Spawn git receive-pack
    let mut git = GitSubprocess::spawn(GitService::ReceivePack, &repo_path, false)
        .map_err(GitError::ProcessSpawnFailed)?;

    // Write request to git's stdin
    if let Some(mut stdin) = git.take_stdin() {
        stdin
            .write_all(&request_body)
            .await
            .map_err(GitError::IoError)?;
        drop(stdin);
    }

    // Read response from git's stdout
    let mut output = Vec::new();
    let mut stderr_output = Vec::new();

    if let Some(stdout) = git.take_stdout() {
        let mut stdout = stdout;
        stdout
            .read_to_end(&mut output)
            .await
            .map_err(GitError::IoError)?;
    }

    if let Some(stderr) = git.take_stderr() {
        let mut stderr = stderr;
        stderr
            .read_to_end(&mut stderr_output)
            .await
            .map_err(GitError::IoError)?;
    }

    // Wait for process
    let status = git.wait().await.map_err(GitError::IoError)?;

    if !status.success() {
        let stderr_str = String::from_utf8_lossy(&stderr_output);
        error!("Git receive-pack failed: {}", stderr_str);
        return Err(GitError::GitFailed(status.code()));
    }

    // GRASP-01: Set HEAD after git data is received
    // "MUST set repository HEAD per repository state announcement
    // as soon as the git data related to that branch has been received."
    if let Some(ref state) = auth_result.state {
        if let Some(head_ref) = &state.head {
            if let Some(branch_name) = state.get_head_branch() {
                if let Some(commit) = state.get_branch_commit(branch_name) {
                    match try_set_head_if_available(&repo_path, head_ref, commit) {
                        Ok(true) => {
                            info!("Set HEAD to {} after push to {:?}", head_ref, repo_path);
                        }
                        Ok(false) => {
                            debug!(
                                "HEAD commit {} not found after push, HEAD not updated",
                                commit
                            );
                        }
                        Err(e) => {
                            warn!("Failed to set HEAD after push: {}", e);
                        }
                    }
                }
            }
        }
    }

    // Save all events from purgatory that authorized this push and remove them from purgatory
    // This includes state events, PR events, and PR-update events
    info!(
        "Saving {} purgatory event(s) to database after successful push",
        auth_result.purgatory_events.len()
    );

    for event in &auth_result.purgatory_events {
        match database.save_event(event).await {
            Ok(_) => {
                // Remove from purgatory based on event kind
                if event.kind == Kind::from(KIND_REPOSITORY_STATE) {
                    info!("Saved purgatory state event {} to database", event.id);
                    purgatory.remove_state_event(identifier, &event.id);
                    info!("Removed saved state event {} from purgatory", event.id);
                } else if event.kind == Kind::from(KIND_PR)
                    || event.kind == Kind::from(KIND_PR_UPDATE)
                {
                    info!("Saved purgatory PR event {} to database", event.id);
                    // Extract event ID from the event itself (it's the event.id)
                    let event_id_hex = event.id.to_hex();
                    purgatory.remove_pr(&event_id_hex);
                    info!("Removed saved PR event {} from purgatory", event.id);
                }
                // Broadcast to WebSocket subscribers
                if relay.notify_event(event.clone()) {
                    info!(
                        "Broadcast purgatory event {} to websocket listeners",
                        event.id
                    );
                } else {
                    warn!(
                        "Failed to broadcast purgatory event {} to websocket listeners",
                        event.id
                    );
                }
            }
            Err(e) => {
                warn!(
                    "Failed to save purgatory event {} to database: {}",
                    event.id, e
                );
            }
        }
    }

    // TODO figure out what atomic pushes look like in GRASP (we cant accepted differnte state events changing different branches at the same time)

    // Sync git data to other owner repositories that authorize the same state event
    // This ensures all owners who share maintainers get the same git data
    if let Some(ref state) = auth_result.state {
        // Fetch repository data for sync
        match fetch_repository_data(&database, identifier).await {
            Ok(db_repo_data) => {
                let git_data_path_buf = std::path::PathBuf::from(git_data_path);
                let sync_result =
                    sync_to_owner_repos(&repo_path, state, &db_repo_data, &git_data_path_buf);

                if sync_result.repos_synced > 0 {
                    info!(
                        "Synced git data to {} other owner repositories for {}",
                        sync_result.repos_synced, identifier
                    );
                }

                if !sync_result.errors.is_empty() {
                    for (repo, error) in &sync_result.errors {
                        warn!("Error syncing to {}: {}", repo, error);
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Failed to fetch repository data for sync after push to {}: {}",
                    identifier, e
                );
            }
        }
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            "content-type",
            GitService::ReceivePack.result_content_type(),
        )
        .header("cache-control", "no-cache")
        .body(Full::new(Bytes::from(output)))
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
