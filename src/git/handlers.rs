//! Git HTTP Protocol Handlers
//!
//! This module implements the HTTP handlers for Git Smart HTTP protocol.

use std::path::PathBuf;
use hyper::{body::Bytes, Response, StatusCode};
use http_body_util::Full;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error, info, warn};

use super::authorization::{
    AuthorizationContext, AuthorizationResult, parse_pushed_refs, validate_push_refs,
};
use super::protocol::{GitService, PktLine};
use super::subprocess::GitSubprocess;
use super::{try_set_head_if_available};

use crate::nostr::events::RepositoryState;

/// Handle GET /info/refs?service=git-{upload,receive}-pack
///
/// This advertises the repository's refs to the client.
pub async fn handle_info_refs(
    repo_path: PathBuf,
    service: GitService,
) -> Result<Response<Full<Bytes>>, GitError> {
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
    let mut stderr_output = Vec::new();
    
    if let Some(stdout) = git.take_stdout() {
        let mut stdout = stdout;
        stdout.read_to_end(&mut output).await
            .map_err(|e| {
                error!("Failed to read git output: {}", e);
                GitError::IoError(e)
            })?;
    }
    
    if let Some(stderr) = git.take_stderr() {
        let mut stderr = stderr;
        stderr.read_to_end(&mut stderr_output).await
            .map_err(|e| {
                error!("Failed to read git stderr: {}", e);
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
        let stderr_str = String::from_utf8_lossy(&stderr_output);
        error!("Git process failed with status: {:?}, stderr: {}", status, stderr_str);
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
        stdin.write_all(&request_body).await
            .map_err(GitError::IoError)?;
        // Close stdin to signal end of input
        drop(stdin);
    }

    // Read response from git's stdout
    let mut output = Vec::new();
    let mut stderr_output = Vec::new();
    
    if let Some(stdout) = git.take_stdout() {
        let mut stdout = stdout;
        stdout.read_to_end(&mut output).await
            .map_err(GitError::IoError)?;
    }
    
    if let Some(stderr) = git.take_stderr() {
        let mut stderr = stderr;
        stderr.read_to_end(&mut stderr_output).await
            .map_err(GitError::IoError)?;
    }

    // Wait for process
    let status = git.wait().await
        .map_err(GitError::IoError)?;

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

/// Authorization parameters for push operations
#[derive(Debug, Clone)]
pub struct PushAuthParams {
    /// The relay URL for fetching events (e.g., "ws://localhost:8080")
    pub relay_url: String,
    /// The npub of the repository owner
    pub owner_npub: String,
    /// The repository identifier (d tag)
    pub identifier: String,
}

/// Handle POST /git-receive-pack (push)
///
/// This includes GRASP authorization validation according to GRASP-01:
/// "MUST accept pushes via this service that match the latest repo state announcement
/// on the relay, respecting the recursive maintainer set."
///
/// Also per GRASP-01: "MUST set repository HEAD per repository state announcement
/// as soon as the git data related to that branch has been received."
pub async fn handle_receive_pack(
    repo_path: PathBuf,
    request_body: Bytes,
    auth_params: Option<PushAuthParams>,
) -> Result<Response<Full<Bytes>>, GitError> {
    debug!("Handling receive-pack for {:?}", repo_path);

    if !repo_path.exists() {
        return Err(GitError::RepositoryNotFound);
    }

    // Keep track of state for HEAD setting after push
    let mut authorized_state: Option<RepositoryState> = None;

    // GRASP Authorization Check
    if let Some(ref params) = auth_params {
        info!(
            "Authorizing push for {}/{} via {}",
            params.owner_npub, params.identifier, params.relay_url
        );

        match authorize_push(params, &request_body).await {
            Ok(auth_result) => {
                if !auth_result.authorized {
                    warn!(
                        "Push rejected for {}/{}: {}",
                        params.owner_npub, params.identifier, auth_result.reason
                    );
                    return Err(GitError::Unauthorized);
                }
                info!(
                    "Push authorized for {}/{} - {} maintainers",
                    params.owner_npub,
                    params.identifier,
                    auth_result.maintainers.len()
                );
                // Save the state for HEAD setting after push
                authorized_state = auth_result.state;
            }
            Err(e) => {
                warn!(
                    "Authorization check failed for {}/{}: {}",
                    params.owner_npub, params.identifier, e
                );
                return Err(GitError::Unauthorized);
            }
        }
    } else {
        debug!("No authorization parameters provided - accepting push");
    }

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
    let mut stderr_output = Vec::new();
    
    if let Some(stdout) = git.take_stdout() {
        let mut stdout = stdout;
        stdout.read_to_end(&mut output).await
            .map_err(GitError::IoError)?;
    }
    
    if let Some(stderr) = git.take_stderr() {
        let mut stderr = stderr;
        stderr.read_to_end(&mut stderr_output).await
            .map_err(GitError::IoError)?;
    }

    // Wait for process
    let status = git.wait().await
        .map_err(GitError::IoError)?;

    if !status.success() {
        let stderr_str = String::from_utf8_lossy(&stderr_output);
        error!("Git receive-pack failed: {}", stderr_str);
        return Err(GitError::GitFailed(status.code()));
    }

    // GRASP-01: Set HEAD after git data is received
    // "MUST set repository HEAD per repository state announcement
    // as soon as the git data related to that branch has been received."
    if let Some(state) = authorized_state {
        if let Some(head_ref) = &state.head {
            if let Some(branch_name) = state.get_head_branch() {
                if let Some(commit) = state.get_branch_commit(branch_name) {
                    match try_set_head_if_available(&repo_path, head_ref, commit) {
                        Ok(true) => {
                            info!(
                                "Set HEAD to {} after push to {:?}",
                                head_ref, repo_path
                            );
                        }
                        Ok(false) => {
                            debug!(
                                "HEAD commit {} not found after push, HEAD not updated",
                                commit
                            );
                        }
                        Err(e) => {
                            warn!(
                                "Failed to set HEAD after push: {}",
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", GitService::ReceivePack.result_content_type())
        .header("cache-control", "no-cache")
        .body(Full::new(Bytes::from(output)))
        .unwrap())
}

/// Perform GRASP authorization for a push operation
///
/// This function:
/// 1. Fetches announcement and state events from the relay
/// 2. Collects all authorized publishers from announcements
/// 3. Gets the latest authorized state
/// 4. Validates that pushed refs match the state
async fn authorize_push(
    params: &PushAuthParams,
    request_body: &Bytes,
) -> anyhow::Result<AuthorizationResult> {
    use nostr_sdk::ClientBuilder;
    use std::time::Duration;

    debug!(
        "Fetching events for identifier {} from relay {}",
        params.identifier, params.relay_url
    );

    // Create a Nostr client to fetch events
    let client = ClientBuilder::default().build();
    client.add_relay(&params.relay_url).await?;
    client.connect().await;

    // Create filter for repository events
    let filter = AuthorizationContext::create_filter(&params.identifier);

    // Fetch events with timeout
    let events = client.fetch_events(filter, Duration::from_secs(5))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch events: {}", e))?;

    let events: Vec<_> = events.into_iter().collect();
    debug!("Fetched {} events from relay", events.len());

    if events.is_empty() {
        return Ok(AuthorizationResult::denied(
            "No repository announcement or state events found on relay",
        ));
    }

    // Create authorization context
    let ctx = AuthorizationContext::new(events);

    // Get the authorized state (no owner_pubkey needed - self-contained check)
    let auth_result = ctx.get_authorized_state(&params.identifier)?;

    if !auth_result.authorized {
        return Ok(auth_result);
    }

    // Parse refs from the push request
    let pushed_refs = parse_pushed_refs(request_body);
    debug!("Parsed {} refs from push request", pushed_refs.len());
    for (old_oid, new_oid, ref_name) in &pushed_refs {
        debug!("  {} {} -> {}", ref_name, old_oid, new_oid);
    }

    // Validate refs against state
    if let Some(ref state) = auth_result.state {
        debug!("Validating against state with {} branches", state.branches.len());
        
        // If we have a state event but couldn't parse any refs, reject the push.
        // This protects against parsing failures allowing unauthorized pushes.
        if pushed_refs.is_empty() && !state.branches.is_empty() {
            warn!("No refs parsed from push request but state event has branches - rejecting");
            return Ok(AuthorizationResult::denied(
                "Failed to parse refs from push request - cannot validate against state"
            ));
        }
        
        if let Err(e) = validate_push_refs(state, &pushed_refs) {
            warn!("Ref validation failed: {}", e);
            return Ok(AuthorizationResult::denied(format!(
                "Ref validation failed: {}",
                e
            )));
        }
        debug!("Ref validation passed");
    } else {
        warn!("No state in auth_result - cannot validate refs");
    }

    Ok(auth_result)
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