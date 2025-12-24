//! Git HTTP Protocol Handlers
//!
//! This module implements the HTTP handlers for Git Smart HTTP protocol.

use http_body_util::Full;
use hyper::{body::Bytes, Response, StatusCode};
use nostr_sdk::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error, info, warn};

use super::authorization::{
    get_state_authorization_for_specific_owner_repo, parse_pushed_refs, validate_nostr_ref_pushes,
    validate_push_refs, AuthorizationResult,
};
use super::protocol::{GitService, PktLine};
use super::subprocess::GitSubprocess;
use super::try_set_head_if_available;

use crate::nostr::builder::SharedDatabase;
use crate::nostr::events::{RepositoryState, KIND_PR, KIND_PR_UPDATE, KIND_REPOSITORY_STATE};
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
pub async fn handle_receive_pack(
    repo_path: PathBuf,
    request_body: Bytes,
    database: SharedDatabase,
    identifier: &str,
    owner_pubkey: &str,
    purgatory: Arc<Purgatory>,
) -> Result<Response<Full<Bytes>>, GitError> {
    debug!("Handling receive-pack for {:?}", repo_path);

    if !repo_path.exists() {
        return Err(GitError::RepositoryNotFound);
    }

    // Keep track of state and events for processing after push
    let mut authorized_state: Option<RepositoryState> = None;
    let mut authorized_events: Vec<Event> = Vec::new();

    // GRASP Authorization Check
    info!(
        "Authorizing push for {} owned by {} via database query",
        identifier, owner_pubkey
    );

    match authorize_push(
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
                "Push authorized for {} - {} maintainers, {} purgatory events",
                identifier,
                auth_result.maintainers.len(),
                auth_result.purgatory_events.len()
            );
            // Save the state for HEAD setting after push
            authorized_state = auth_result.state.clone();
            // Save the purgatory events for database saving after push
            authorized_events = auth_result.purgatory_events;
        }
        Err(e) => {
            warn!("Authorization check failed for {}: {}", identifier, e);
            return Err(GitError::Unauthorized);
        }
    }

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
    if let Some(ref state) = authorized_state {
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
    if !authorized_events.is_empty() {
        info!(
            "Saving {} purgatory event(s) to database after successful push",
            authorized_events.len()
        );

        for event in &authorized_events {
            match database.save_event(event).await {
                Ok(_) => {
                    info!("Saved purgatory event {} to database", event.id);
                    // TODO let broadcast_success = local_relay.notify_event(event.clone());
                    warn!("TODO Here we need to broadcast on open websockets for live listeners. eventid; {}", event.id);
                    // Remove from purgatory based on event kind
                    if event.kind == Kind::from(KIND_REPOSITORY_STATE) {
                        purgatory.remove_state_event(identifier, &event.id);
                        info!("Removed state event {} from purgatory", event.id);
                    } else if event.kind == Kind::from(KIND_PR)
                        || event.kind == Kind::from(KIND_PR_UPDATE)
                    {
                        // Extract event ID from the event itself (it's the event.id)
                        let event_id_hex = event.id.to_hex();
                        purgatory.remove_pr(&event_id_hex);
                        info!("Removed PR event {} from purgatory", event.id);
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

/// Perform GRASP authorization for a push operation
///
/// This function queries the database directly (not via WebSocket):
/// 1. Parses the pushed refs from the git pack protocol
/// 2. Separates refs/nostr/ refs from normal refs
/// 3. For normal refs: validates against state events in purgatory
/// 4. For refs/nostr/ refs: validates event ID format and collects PR/PR-update events from purgatory
/// 5. Returns all authorizing events (state + PR/PR-update) in the result
async fn authorize_push(
    database: &SharedDatabase,
    identifier: &str,
    owner_pubkey: &str,
    request_body: &Bytes,
    purgatory: &Arc<Purgatory>,
    repo_path: &std::path::Path,
) -> anyhow::Result<AuthorizationResult> {
    debug!(
        "Authorizing push for {} owned by {} via database query",
        identifier, owner_pubkey
    );

    // Parse refs from the push request
    let pushed_refs = parse_pushed_refs(request_body);
    debug!("Parsed {} refs from push request", pushed_refs.len());
    for (old_oid, new_oid, ref_name) in &pushed_refs {
        debug!("  {} {} -> {}", ref_name, old_oid, new_oid);
    }

    // Separate refs/nostr/ refs from state refs
    let (nostr_refs, state_refs): (Vec<_>, Vec<_>) = pushed_refs
        .iter()
        .partition(|(_, _, ref_name)| ref_name.starts_with("refs/nostr/"));

    // Collect all purgatory events that authorize this push
    let mut purgatory_events = Vec::new();

    // Handle refs/nostr/ refs - validate and collect PR/PR-update events from purgatory
    if !nostr_refs.is_empty() {
        debug!(
            "Found {} refs/nostr/ refs - validating and collecting from purgatory",
            nostr_refs.len()
        );

        for (_, new_oid, ref_name) in &nostr_refs {
            // Extract event ID from ref name
            if let Some(event_id_hex) = ref_name.strip_prefix("refs/nostr/") {
                // Validate event ID format
                if EventId::parse(event_id_hex).is_err() {
                    warn!("Invalid event ID format in ref: {}", ref_name);
                    return Ok(AuthorizationResult::denied(format!(
                        "Invalid event ID format in ref: {}",
                        ref_name
                    )));
                }

                // Check purgatory for PR event
                if let Some(entry) = purgatory.find_pr(event_id_hex) {
                    if let Some(event) = entry.event {
                        // Verify commit matches
                        if entry.commit == *new_oid {
                            debug!(
                                "Found matching PR event {} in purgatory for ref {}",
                                event_id_hex, ref_name
                            );
                            purgatory_events.push(event);
                        } else {
                            warn!(
                                "PR event {} in purgatory has commit mismatch: expected {}, got {}",
                                event_id_hex, entry.commit, new_oid
                            );
                            return Ok(AuthorizationResult::denied(format!(
                                "PR event {} commit mismatch: expected {}, got {}",
                                event_id_hex, entry.commit, new_oid
                            )));
                        }
                    } else {
                        // Placeholder exists - allow push (git-data-first scenario)
                        debug!(
                            "Found placeholder already for PR event {} in purgatory - as we dont have the event and therefore dont know the required commit_id we allow overwriting with a different commit_id",
                            event_id_hex
                        );
                    }
                } else {
                    // No entry in purgatory - check database for existing event
                    let nostr_refs_owned = vec![(String::new(), new_oid.clone(), ref_name.clone())];
                    if let Err(e) = validate_nostr_ref_pushes(database, &nostr_refs_owned).await {
                        warn!("refs/nostr/ validation failed: {}", e);
                        return Ok(AuthorizationResult::denied(format!(
                            "refs/nostr/ validation failed: {}",
                            e
                        )));
                    }
                    debug!(
                        "No purgatory entry for {} - validated against database",
                        event_id_hex
                    );
                }
            }
        }
    }

    // Handle normal refs - validate against state events
    if !state_refs.is_empty() {
        debug!(
            "Found {} non-refs/nostr/ refs - checking state authorization",
            state_refs.len()
        );

        let auth_result = get_state_authorization_for_specific_owner_repo(
            database,
            identifier,
            owner_pubkey,
            purgatory,
            &pushed_refs, //it would be better to accept state_refs but thats in different format
            repo_path,
        )
        .await?;

        if !auth_result.authorized {
            return Ok(auth_result);
        }

        // Collect state events from purgatory
        purgatory_events.extend(auth_result.purgatory_events);

        // Validate refs against state
        let other_refs_owned: Vec<(String, String, String)> = state_refs
            .into_iter()
            .map(|(a, b, c)| (a.clone(), b.clone(), c.clone()))
            .collect();

        if let Some(ref state) = auth_result.state {
            debug!(
                "Validating against state with {} branches",
                state.branches.len()
            );

            if other_refs_owned.is_empty() && !state.branches.is_empty() {
                warn!("No refs parsed from push request but state event has branches - rejecting");
                return Ok(AuthorizationResult::denied(
                    "Failed to parse refs from push request - cannot validate against state",
                ));
            }

            if let Err(e) = validate_push_refs(state, &other_refs_owned) {
                warn!("Ref validation failed: {}", e);
                return Ok(AuthorizationResult::denied(format!(
                    "Ref validation failed: {}",
                    e
                )));
            }
            debug!("Ref validation passed");
        }

        // Return result with purgatory events
        return Ok(AuthorizationResult {
            authorized: true,
            reason: auth_result.reason,
            state: auth_result.state,
            maintainers: auth_result.maintainers,
            purgatory_events,
        });
    }

    // Only refs/nostr/ refs - return success with collected events
    Ok(AuthorizationResult {
        authorized: true,
        reason: "Push to refs/nostr/ validated".to_string(),
        state: None,
        maintainers: vec![],
        purgatory_events,
    })
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
