//! GRASP-06 `/prs/` `git-receive-pack` handler.
//!
//! Spec 06.md line 15:
//!
//! > MUST accept pushes to `refs/nostr/<event-id>`. MUST reject pushes to any
//! > other ref namespace.
//!
//! The handler:
//!
//! 1. Pre-scans the pkt-line ref-update list and rejects the entire push (via
//!    an `ERR` pkt-line on a 200 response) if *any* ref name is not of the
//!    exact shape `refs/nostr/<64-lowercase-hex>`. The repo on disk is not
//!    touched in this path — probe pushes that would have been rejected
//!    leave no trace.
//! 2. Acquires the per-path coordination state from [`RepoInitLocks`]
//!    briefly: under its mutex it runs the on-demand `git init --bare`
//!    and increments the `in_flight` counter, then releases the mutex.
//!    Steps 3 and 4 run *without* the per-path lock so concurrent pushes
//!    to the same `(submitter, identifier)` proceed in parallel — git's
//!    own ref locking handles intra-push concurrency, and the
//!    `in_flight` counter is what off-push cleanup paths consult to know
//!    a push is active.
//! 3. Runs `git-receive-pack` against the repo, mirroring the subprocess
//!    plumbing in [`crate::git::handlers::handle_receive_pack`].
//! 4. For each accepted `refs/nostr/<event-id>` ref, validates against the
//!    database first, then purgatory. On signer / `a`-tag identifier /
//!    `c`-tag commit mismatch the ref is deleted and (for purgatory
//!    placeholders) the placeholder is discarded. When neither the database
//!    nor purgatory knows about the event, a PR placeholder is added so the
//!    standard 30-minute purgatory sweep can clean it up if the event never
//!    arrives.
//! 5. Re-acquires the per-path mutex briefly, decrements `in_flight`,
//!    and — if no other push is in flight and the repo has zero refs
//!    left — removes the bare directory. Always runs, including on
//!    receive-pack protocol errors, so a failed push that has just
//!    initialised an empty repo does not leak it.
//! 6. Triggers the standard purgatory-release path via
//!    [`crate::git::sync::process_newly_available_git_data`] so PR events
//!    already in purgatory waiting for these commits get promoted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Response, StatusCode};
use nostr_relay_builder::LocalRelay;
use nostr_sdk::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::git::authorization::{extract_commit_tag, get_pr_event_by_id, parse_pushed_refs};
use crate::git::handlers::{build_git_protocol_error_response, is_git_protocol_error, GitError};
use crate::git::protocol::GitService;
use crate::git::subprocess::GitSubprocess;
use crate::git::sync::{extract_identifier_from_pr_event, process_newly_available_git_data};
use crate::git::{delete_ref, list_refs};
use crate::grasp06::endpoint::PrsUrl;
use crate::grasp06::paths::prs_repo_path;
use crate::nostr::builder::{Nip34WritePolicy, SharedDatabase};
use crate::purgatory::Purgatory;
use crate::sync::rejected_index::RejectedEventsIndex;

/// Per-`(submitter, identifier)` coordination state shared between
/// `/prs/` receive-pack pushes and the cleanup paths that may delete the
/// bare repo.
///
/// `mu` is held only briefly:
///
/// * by the receive handler to perform `git init --bare` and register
///   the request as in-flight (`fetch_add` on `in_flight`),
/// * by the receive handler again at end-of-push to decrement
///   `in_flight` and, if no other push is in flight and the repo has
///   zero refs, remove the bare directory,
/// * by off-push cleanup paths (PR-event validation discard, purgatory
///   expiry) for the duration of one `delete_ref` + optional
///   `remove_dir_all`.
///
/// `git-receive-pack` itself and per-ref validation run *without* the
/// mutex held, so two pushes to the same path proceed in parallel; git's
/// own ref locking handles intra-push concurrency.
///
/// Off-push cleanup paths only `rm -rf` the bare repo when both
/// `in_flight.load() == 0` *and* `list_refs` returns empty while they
/// hold `mu` — the same mutex that gates `in_flight` updates — so a
/// repo can never be deleted while a push is mid-receive.
pub struct PrsPathState {
    pub mu: Mutex<()>,
    pub in_flight: AtomicUsize,
}

impl PrsPathState {
    fn new() -> Self {
        Self {
            mu: Mutex::new(()),
            in_flight: AtomicUsize::new(0),
        }
    }
}

/// Shared per-path state map for the GRASP-06 `/prs/` endpoint. See
/// [`PrsPathState`] for the locking discipline.
pub type RepoInitLocks = Arc<DashMap<PathBuf, Arc<PrsPathState>>>;

/// Create a fresh, empty [`RepoInitLocks`] for use as an [`HttpService`]
/// field.
///
/// [`HttpService`]: crate::http
pub fn new_repo_init_locks() -> RepoInitLocks {
    Arc::new(DashMap::new())
}

/// Look up (or insert) the [`PrsPathState`] for `repo_path` in `locks`.
/// Used by off-push cleanup paths so they take the same Arc the receive
/// handler will see.
pub fn path_state(locks: &RepoInitLocks, repo_path: &Path) -> Arc<PrsPathState> {
    locks
        .entry(repo_path.to_path_buf())
        .or_insert_with(|| Arc::new(PrsPathState::new()))
        .value()
        .clone()
}

/// Handle `POST /prs/<npub>/<identifier>.git/git-receive-pack`.
///
/// See the module-level docs for the full algorithm. All application-level
/// rejections are returned as HTTP 200 with an `ERR` pkt-line so the git
/// client can display the message and exit non-zero.
#[allow(clippy::too_many_arguments)]
pub async fn handle_prs_receive_pack(
    prs: &PrsUrl,
    request_body: Bytes,
    database: SharedDatabase,
    relay: LocalRelay,
    purgatory: Arc<Purgatory>,
    write_policy: Arc<Nip34WritePolicy>,
    rejected_events_index: Arc<RejectedEventsIndex>,
    git_data_path: &str,
    git_protocol: Option<&str>,
    repo_init_locks: RepoInitLocks,
) -> Result<Response<Full<Bytes>>, GitError> {
    // 1. Pre-scan refs and reject the whole push if any ref name is not
    //    `refs/nostr/<64-lowercase-hex>`. We use the same parser as the
    //    standard receive-pack path so behaviour stays in lock-step.
    let pushed_refs = parse_pushed_refs(&request_body);
    if pushed_refs.is_empty() {
        warn!(
            "/prs/ receive-pack: no parsable refs in push to {}/{}",
            prs.submitter.to_hex(),
            prs.identifier
        );
        return Ok(build_git_protocol_error_response(
            GitService::ReceivePack,
            "no ref updates found in push",
        ));
    }

    for (_, _, ref_name) in &pushed_refs {
        if let Some(reason) = invalid_ref_reason(ref_name) {
            warn!(
                "/prs/ receive-pack: rejecting push to {}/{} — {}",
                prs.submitter.to_hex(),
                prs.identifier,
                reason
            );
            return Ok(build_git_protocol_error_response(
                GitService::ReceivePack,
                &format!(
                    "GRASP-06: only pushes to refs/nostr/<event-id> are accepted ({})",
                    reason
                ),
            ));
        }
    }

    // 2. Acquire the per-path coordination state and, under its mutex,
    //    initialise the bare repo on demand and register this request as
    //    in-flight. The mutex is then released — `git-receive-pack` and
    //    per-ref validation run WITHOUT the lock so concurrent pushes to
    //    the same `(submitter, identifier)` proceed in parallel. Cleanup
    //    paths consult `in_flight` (under the same mutex) before
    //    deleting the bare repo, so a repo can never vanish mid-receive.
    let repo_path = prs_repo_path(
        Path::new(git_data_path),
        &prs.submitter.to_hex(),
        &prs.identifier,
    );
    let state = path_state(&repo_init_locks, &repo_path);

    {
        let _g = state.mu.lock().await;
        if let Err(e) = ensure_repo_initialised(&repo_path).await {
            error!(
                "/prs/ receive-pack: failed to initialise repo at {}: {}",
                repo_path.display(),
                e
            );
            return Err(e);
        }
        state.in_flight.fetch_add(1, Ordering::Relaxed);
    }

    // 3 + 4: run receive-pack and validate refs without the per-path
    //        mutex held. Wrapping in an async block lets us catch every
    //        exit path with the same end-of-push cleanup below.
    let process_result: Result<Response<Full<Bytes>>, GitError> = async {
        let response = run_receive_pack(&repo_path, &request_body, git_protocol).await?;

        // If the push itself failed with a protocol error (e.g. a stale
        // OID or a corrupt pack) we return that ERR pkt-line straight
        // back to the client without doing any post-push validation. The
        // pre-scan in step 1 already gated ref-name shape, so we only
        // land here for git-level failures.
        if response.status() != StatusCode::OK {
            return Ok(response);
        }

        // 4. Per-ref post-push validation. Iterate over the same parsed
        //    ref list — at this point every ref name is known to be
        //    `refs/nostr/<event-id>`, so the strip is infallible.
        for (_, new_oid, ref_name) in &pushed_refs {
            let event_id_hex = ref_name
                .strip_prefix("refs/nostr/")
                .expect("ref shape validated above");
            validate_pushed_nostr_ref(
                &database,
                &purgatory,
                &repo_path,
                prs,
                event_id_hex,
                new_oid,
            )
            .await;
        }

        Ok(response)
    }
    .await;

    // 5. End-of-push cleanup. Always runs — including on receive-pack
    //    protocol errors and `?`-propagated transport errors — so a
    //    failed push that has just initialised an empty repo does not
    //    leak it. Decrements `in_flight`; if no other push is in flight
    //    and the repo has zero refs left, removes the bare directory.
    {
        let _g = state.mu.lock().await;
        state.in_flight.fetch_sub(1, Ordering::Relaxed);
        if state.in_flight.load(Ordering::Relaxed) == 0 {
            if let Ok(refs) = list_refs(&repo_path) {
                if refs.is_empty() {
                    if let Err(e) = std::fs::remove_dir_all(&repo_path) {
                        warn!(
                            "/prs/ receive-pack: failed to clean up empty repo {}: {}",
                            repo_path.display(),
                            e
                        );
                    } else {
                        debug!(
                            "/prs/ receive-pack: removed empty repo {} (no refs after push)",
                            repo_path.display()
                        );
                    }
                }
            }
        }
    }

    let response = process_result?;

    // 6. Drive the standard purgatory-release pipeline so PR events
    //    already waiting on these commits can be promoted out of
    //    purgatory. Only fires on a successful push, and only if the
    //    repo still exists (it may have been removed in step 5).
    if response.status() == StatusCode::OK && repo_path.exists() {
        let new_oids: HashSet<String> = pushed_refs
            .iter()
            .filter(|(_, new_oid, _)| new_oid != "0000000000000000000000000000000000000000")
            .map(|(_, new_oid, _)| new_oid.clone())
            .collect();

        if let Err(e) = process_newly_available_git_data(
            &repo_path,
            &new_oids,
            &database,
            Some(&relay),
            &purgatory,
            Path::new(git_data_path),
            Some(&write_policy),
            Some(&rejected_events_index),
        )
        .await
        {
            warn!(
                "/prs/ receive-pack: post-push processing failed for {}/{}: {}",
                prs.submitter.to_hex(),
                prs.identifier,
                e
            );
        }
    }

    Ok(response)
}

/// Return `Some(reason)` if `ref_name` is not exactly
/// `refs/nostr/<64-lowercase-hex>`.
///
/// The shape is deliberately strict: anything else (including upper-case
/// hex, short/long event IDs, or `refs/heads/*`) is "any other ref
/// namespace" per the spec and must be rejected.
fn invalid_ref_reason(ref_name: &str) -> Option<String> {
    let Some(event_id) = ref_name.strip_prefix("refs/nostr/") else {
        return Some(format!("ref {} is outside refs/nostr/", ref_name));
    };
    if event_id.len() != 64 {
        return Some(format!(
            "event-id segment of {} is {} chars, expected 64",
            ref_name,
            event_id.len()
        ));
    }
    if !event_id
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Some(format!(
            "event-id segment of {} is not lowercase hex",
            ref_name
        ));
    }
    None
}

/// `mkdir -p` the parent and `git init --bare --initial-branch=main
/// --quiet` into `repo_path` if it does not already exist.
///
/// The caller must hold the per-path mutex from [`PrsPathState`] for
/// `repo_path` before invoking this function.
async fn ensure_repo_initialised(repo_path: &Path) -> Result<(), GitError> {
    if repo_path.exists() {
        return Ok(());
    }

    if let Some(parent) = repo_path.parent() {
        std::fs::create_dir_all(parent).map_err(GitError::IoError)?;
    }

    let output = std::process::Command::new("git")
        .args(["init", "--bare", "--initial-branch=main", "--quiet"])
        .arg(repo_path)
        .output()
        .map_err(GitError::ProcessSpawnFailed)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(
            "/prs/ git init --bare failed at {}: {}",
            repo_path.display(),
            stderr.trim()
        );
        return Err(GitError::GitFailed(output.status.code()));
    }

    info!(
        "/prs/ initialised bare repo at {} on demand",
        repo_path.display()
    );
    Ok(())
}

/// Spawn `git-receive-pack`, stream stdin/stdout/stderr, and convert the
/// result into an HTTP response — protocol errors become 200 + ERR
/// pkt-line, transport errors bubble up as [`GitError`]. Mirrors
/// [`crate::git::handlers::handle_receive_pack`]'s subprocess plumbing.
async fn run_receive_pack(
    repo_path: &Path,
    request_body: &Bytes,
    git_protocol: Option<&str>,
) -> Result<Response<Full<Bytes>>, GitError> {
    let mut git = GitSubprocess::spawn(GitService::ReceivePack, repo_path, false, git_protocol)
        .map_err(GitError::ProcessSpawnFailed)?;

    if let Some(mut stdin) = git.take_stdin() {
        stdin
            .write_all(request_body)
            .await
            .map_err(GitError::IoError)?;
        drop(stdin);
    }

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

    let status = git.wait().await.map_err(GitError::IoError)?;

    if !status.success() {
        let stderr_str = String::from_utf8_lossy(&stderr_output);
        if is_git_protocol_error(status.code(), &stderr_output) {
            warn!(
                "/prs/ git-receive-pack protocol error (returning ERR pkt-line): {}",
                stderr_str.trim()
            );
            return Ok(build_git_protocol_error_response(
                GitService::ReceivePack,
                &stderr_str,
            ));
        }
        error!(
            "/prs/ git-receive-pack failed (transport): {}",
            stderr_str.trim()
        );
        return Err(GitError::GitFailed(status.code()));
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

/// Validate one accepted `refs/nostr/<event-id>` ref against the database
/// and purgatory, deleting the ref (and discarding the purgatory
/// placeholder, where applicable) on any mismatch. If no event is known at
/// all, register a placeholder so the 30-minute purgatory sweep can clean
/// the ref up if the event never arrives.
async fn validate_pushed_nostr_ref(
    database: &SharedDatabase,
    purgatory: &Purgatory,
    repo_path: &Path,
    prs: &PrsUrl,
    event_id_hex: &str,
    pushed_commit: &str,
) {
    let ref_name = format!("refs/nostr/{}", event_id_hex);

    // Parsing the event id this late ensures the on-disk ref shape and the
    // in-memory key formats agree.
    let event_id = match EventId::parse(event_id_hex) {
        Ok(id) => id,
        Err(e) => {
            // Should be unreachable thanks to the pre-scan, but if it does
            // happen we delete the ref defensively rather than leave an
            // unparseable placeholder around.
            warn!(
                "/prs/ post-push: unexpected unparseable event id in ref {}: {}",
                ref_name, e
            );
            let _ = delete_ref(repo_path, &ref_name);
            return;
        }
    };

    // 4a. Database first.
    match get_pr_event_by_id(database, &event_id).await {
        Ok(Some(event)) => {
            if let Some(reason) = describe_pr_event_mismatch(&event, prs, pushed_commit) {
                warn!(
                    "/prs/ post-push: deleting {} — DB event mismatch ({})",
                    ref_name, reason
                );
                let _ = delete_ref(repo_path, &ref_name);
            } else {
                debug!("/prs/ post-push: {} validated against DB event", ref_name);
            }
            return;
        }
        Ok(None) => {}
        Err(e) => {
            warn!(
                "/prs/ post-push: DB query failed for {} (treating as not-found): {}",
                ref_name, e
            );
        }
    }

    // 4b. Purgatory.
    if let Some(entry) = purgatory.find_pr(event_id_hex) {
        match entry.event {
            Some(event) => {
                if let Some(reason) = describe_pr_event_mismatch(&event, prs, pushed_commit) {
                    warn!(
                        "/prs/ post-push: deleting {} — purgatory event mismatch ({})",
                        ref_name, reason
                    );
                    let _ = delete_ref(repo_path, &ref_name);
                    purgatory.remove_pr(event_id_hex);
                } else {
                    debug!(
                        "/prs/ post-push: {} validated against purgatory event",
                        ref_name
                    );
                }
            }
            None => {
                // Existing placeholder. The pushed commit fills in the
                // commit half of the entry; the standard 30-minute sweep
                // will discard it if the event never arrives.
                debug!(
                    "/prs/ post-push: {} matched existing purgatory placeholder",
                    ref_name
                );
            }
        }
        return;
    }

    // 4c. Neither DB nor purgatory know about this event. Register a
    //     placeholder scoped to the URL's submitter + identifier so the
    //     event-side validator can reject mismatched events (and an
    //     attacker can't push to their own /prs/ namespace and have an
    //     unrelated event of the same id later "claim" the ref). The
    //     standard 30-minute sweep cleans the ref up if the corresponding
    //     PR event never arrives.
    purgatory.add_prs_pr_placeholder(
        event_id_hex.to_string(),
        pushed_commit.to_string(),
        prs.submitter,
        prs.identifier.clone(),
    );
    debug!(
        "/prs/ post-push: added scoped PR placeholder for {} awaiting matching event",
        ref_name
    );
}

/// Cross-check a known PR/PR-Update event against the URL submitter and the
/// pushed commit. Returns `None` if everything matches, or `Some(reason)`
/// describing the first mismatch encountered.
fn describe_pr_event_mismatch(event: &Event, prs: &PrsUrl, pushed_commit: &str) -> Option<String> {
    if event.pubkey != prs.submitter {
        return Some(format!(
            "signer {} does not match URL submitter {}",
            event.pubkey.to_hex(),
            prs.submitter.to_hex()
        ));
    }
    match extract_identifier_from_pr_event(event) {
        Some(id) if id == prs.identifier => {}
        Some(id) => {
            return Some(format!(
                "a-tag identifier {} does not match URL identifier {}",
                id, prs.identifier
            ))
        }
        None => return Some("event has no parsable a-tag identifier".to_string()),
    }
    match extract_commit_tag(event) {
        Some(c) if c == pushed_commit => None,
        Some(c) => Some(format!(
            "c-tag commit {} does not match pushed commit {}",
            c, pushed_commit
        )),
        None => Some("event has no c-tag commit".to_string()),
    }
}
