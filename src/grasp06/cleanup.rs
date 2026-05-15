//! Periodic sweep of zero-ref `/prs/` repositories.
//!
//! The receive-pack handler in [`crate::grasp06::receive`] already removes
//! a `/prs/<submitter>/<identifier>.git` bare repo when post-push
//! validation leaves it with zero refs (probe pushes that produced no
//! valid state). This module backs that up with a periodic walker that
//! catches repos which became zero-ref through other paths — for example
//! a refs/nostr/<event-id> deleted later by mismatched-event logic when
//! the placeholder finally fails validation.
//!
//! ## Removal criteria
//!
//! A `<git_data_path>/prs/<hex>/<id>.git` directory is removed when **all**
//! of the following hold:
//!
//! 1. it has zero refs (via [`crate::git::list_refs`]),
//! 2. no PR purgatory entry is still scoped to `(submitter=<hex>, identifier=<id>)`
//!    (via [`Purgatory::has_prs_scope`]), and
//! 3. its directory mtime is older than the purgatory TTL
//!    ([`crate::purgatory::DEFAULT_EXPIRY`]).
//!
//! The mtime check is a defensive guard against racing with an in-flight
//! push that has created the directory but not yet finished writing the
//! first ref.
//!
//! ## Cadence
//!
//! The task runs every 10 minutes in production. When `NGIT_TEST=1` is set
//! it tightens to 1 second so integration tests can observe a sweep within
//! a few seconds of the TTL elapsing.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use nostr_sdk::prelude::*;
use tracing::{debug, info, warn};

use crate::git::list_refs;
use crate::grasp06::paths::prs_base_path;
use crate::purgatory::{Purgatory, DEFAULT_EXPIRY};

/// Production sweep cadence.
const SWEEP_INTERVAL: Duration = Duration::from_secs(600);

/// Test-mode sweep cadence — short enough for an integration test to wait
/// out the TTL and observe a sweep within a few seconds.
const SWEEP_INTERVAL_TEST: Duration = Duration::from_secs(1);

/// Spawn the periodic `/prs/` cleanup task.
///
/// The task runs until the process exits. It is a thin loop around
/// [`sweep_once`] — extract that for unit-level tests.
pub fn spawn(git_data_path: std::path::PathBuf, purgatory: Arc<Purgatory>) {
    let interval = if std::env::var("NGIT_TEST").as_deref() == Ok("1") {
        SWEEP_INTERVAL_TEST
    } else {
        SWEEP_INTERVAL
    };
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // The first `tick()` returns immediately; skip it so the first
        // sweep happens one interval after startup, not at startup.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let removed = sweep_once(&git_data_path, &purgatory, DEFAULT_EXPIRY);
            if removed > 0 {
                info!("/prs/ cleanup sweep removed {} empty repo(s)", removed);
            }
        }
    });
    info!(
        "/prs/ cleanup task started ({}s interval)",
        interval.as_secs()
    );
}

/// Run one sweep of `<git_data_path>/prs/`, returning the number of
/// directories removed.
///
/// `ttl` is the purgatory TTL — repos younger than this are skipped even
/// if they currently have zero refs, because they may belong to an
/// in-flight push.
pub fn sweep_once(git_data_path: &Path, purgatory: &Purgatory, ttl: Duration) -> usize {
    let base = prs_base_path(git_data_path);
    let now = SystemTime::now();

    let hex_entries = match std::fs::read_dir(&base) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => {
            warn!("/prs/ cleanup: failed to read {}: {}", base.display(), e);
            return 0;
        }
    };

    let mut removed = 0usize;

    for hex_entry in hex_entries.flatten() {
        let hex_path = hex_entry.path();
        if !hex_path.is_dir() {
            continue;
        }
        let submitter_hex = hex_entry.file_name().to_string_lossy().into_owned();
        let submitter = match PublicKey::from_hex(&submitter_hex) {
            Ok(pk) => pk,
            Err(_) => {
                // A non-hex first-level entry under /prs/ shouldn't exist;
                // skip rather than touch something we don't understand.
                debug!(
                    "/prs/ cleanup: skipping unrecognised subdir {}",
                    hex_path.display()
                );
                continue;
            }
        };

        let repo_entries = match std::fs::read_dir(&hex_path) {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    "/prs/ cleanup: failed to read {}: {}",
                    hex_path.display(),
                    e
                );
                continue;
            }
        };

        let mut hex_dir_now_empty = true;
        for repo_entry in repo_entries.flatten() {
            let repo_path = repo_entry.path();
            if !repo_path.is_dir() {
                hex_dir_now_empty = false;
                continue;
            }
            let dir_name = repo_entry.file_name().to_string_lossy().into_owned();
            let Some(identifier) = dir_name.strip_suffix(".git") else {
                hex_dir_now_empty = false;
                continue;
            };

            if !is_removable(&repo_path, &submitter, identifier, purgatory, ttl, now) {
                hex_dir_now_empty = false;
                continue;
            }

            match std::fs::remove_dir_all(&repo_path) {
                Ok(()) => {
                    info!("/prs/ cleanup: removed empty repo {}", repo_path.display());
                    removed += 1;
                }
                Err(e) => {
                    warn!(
                        "/prs/ cleanup: failed to remove {}: {}",
                        repo_path.display(),
                        e
                    );
                    hex_dir_now_empty = false;
                }
            }
        }

        // Drop the now-empty <hex> dir too so /prs/ doesn't accumulate
        // empty submitter directories over time.
        if hex_dir_now_empty {
            if let Err(e) = std::fs::remove_dir(&hex_path) {
                // Concurrent activity may have re-populated this dir
                // between the walk and the remove — that's fine, the
                // next sweep will pick it up.
                debug!(
                    "/prs/ cleanup: leaving submitter dir {} (remove failed: {})",
                    hex_path.display(),
                    e
                );
            }
        }
    }

    removed
}

/// Decide whether `<git_data_path>/prs/<submitter_hex>/<identifier>.git`
/// is safe to delete right now.
fn is_removable(
    repo_path: &Path,
    submitter: &PublicKey,
    identifier: &str,
    purgatory: &Purgatory,
    ttl: Duration,
    now: SystemTime,
) -> bool {
    // 1. Zero refs.
    match list_refs(repo_path) {
        Ok(refs) if refs.is_empty() => {}
        Ok(_) => return false,
        Err(e) => {
            warn!(
                "/prs/ cleanup: list_refs failed for {} ({}); leaving in place",
                repo_path.display(),
                e
            );
            return false;
        }
    }

    // 2. No live purgatory scope.
    if purgatory.has_prs_scope(submitter, identifier) {
        return false;
    }

    // 3. Older than TTL — protects against deleting a repo that was just
    //    created by an in-flight push.
    let mtime = match std::fs::metadata(repo_path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(e) => {
            warn!(
                "/prs/ cleanup: stat failed for {} ({}); leaving in place",
                repo_path.display(),
                e
            );
            return false;
        }
    };
    match now.duration_since(mtime) {
        Ok(age) => age >= ttl,
        // mtime in the future — clock skew, leave it alone.
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::Duration;
    use tempfile::tempdir;

    fn init_bare(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        let status = Command::new("git")
            .args(["init", "--bare", "--quiet"])
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn backdate(path: &Path, secs_ago: u64) {
        let ts = SystemTime::now() - Duration::from_secs(secs_ago);
        // `File::set_modified` operates on an opened handle. Open the dir's
        // canonical path through a sentinel file we control to avoid OS
        // differences with opening a directory directly.
        let sentinel = path.join("_backdate_sentinel");
        std::fs::write(&sentinel, b"x").unwrap();
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(&sentinel)
            .unwrap();
        f.set_modified(ts).unwrap();
        // Also backdate the directory itself via `set_modified` on the
        // sentinel's parent through an open. On Linux directory mtimes
        // update on child changes, so simply backdating now after the
        // child writes is sufficient.
        let dir = std::fs::File::open(path).unwrap();
        dir.set_modified(ts).unwrap();
        let _ = std::fs::remove_file(&sentinel);
        // The remove above bumps the dir mtime — redo it.
        let dir = std::fs::File::open(path).unwrap();
        dir.set_modified(ts).unwrap();
    }

    #[test]
    fn sweep_removes_old_zero_ref_repo_without_scope() {
        let tmp = tempdir().unwrap();
        let git_data_path = tmp.path();
        let signer = Keys::generate();
        let hex = signer.public_key().to_hex();
        let repo = crate::grasp06::paths::prs_repo_path(git_data_path, &hex, "abandoned");
        init_bare(&repo);
        backdate(&repo, 7200);

        let purgatory = Purgatory::new(tmp.path().join("relay-state"));

        // With a tiny TTL the directory should be considered old enough.
        let removed = sweep_once(git_data_path, &purgatory, Duration::from_secs(1));
        assert_eq!(removed, 1, "abandoned repo must be removed");
        assert!(!repo.exists(), "repo dir must be gone");
    }

    #[test]
    fn sweep_keeps_repo_within_ttl() {
        let tmp = tempdir().unwrap();
        let git_data_path = tmp.path();
        let signer = Keys::generate();
        let hex = signer.public_key().to_hex();
        let repo = crate::grasp06::paths::prs_repo_path(git_data_path, &hex, "fresh");
        init_bare(&repo);

        let purgatory = Purgatory::new(tmp.path().join("relay-state"));

        // TTL of one hour vs. just-created repo → must be retained.
        let removed = sweep_once(git_data_path, &purgatory, Duration::from_secs(3600));
        assert_eq!(removed, 0);
        assert!(repo.exists());
    }

    #[test]
    fn sweep_keeps_repo_with_active_purgatory_scope() {
        let tmp = tempdir().unwrap();
        let git_data_path = tmp.path();
        let signer = Keys::generate();
        let hex = signer.public_key().to_hex();
        let repo = crate::grasp06::paths::prs_repo_path(git_data_path, &hex, "pending");
        init_bare(&repo);
        backdate(&repo, 7200);

        let purgatory = Purgatory::new(tmp.path().join("relay-state"));
        // Register a placeholder scoped to this (submitter, identifier).
        purgatory.add_prs_pr_placeholder(
            "a".repeat(64),
            "b".repeat(40),
            signer.public_key(),
            "pending".to_string(),
        );

        let removed = sweep_once(git_data_path, &purgatory, Duration::from_secs(1));
        assert_eq!(removed, 0, "repo with active scope must be retained");
        assert!(repo.exists());
    }

    #[test]
    fn sweep_keeps_repo_with_a_ref() {
        let tmp = tempdir().unwrap();
        let git_data_path = tmp.path();
        let signer = Keys::generate();
        let hex = signer.public_key().to_hex();
        let repo = crate::grasp06::paths::prs_repo_path(git_data_path, &hex, "has-ref");
        init_bare(&repo);

        // Create an empty commit + write a refs/nostr/ ref to it.
        let oid = Command::new("git")
            .args(["--git-dir"])
            .arg(&repo)
            .args(["commit-tree", "-m", "x"])
            .arg(
                // empty tree object SHA — git ships this as a constant
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
            )
            .output()
            .unwrap();
        assert!(oid.status.success(), "commit-tree failed: {:?}", oid);
        let oid = String::from_utf8(oid.stdout).unwrap().trim().to_string();
        let status = Command::new("git")
            .args(["--git-dir"])
            .arg(&repo)
            .args(["update-ref", "refs/nostr/deadbeef", &oid])
            .status()
            .unwrap();
        assert!(status.success());

        backdate(&repo, 7200);

        let purgatory = Purgatory::new(tmp.path().join("relay-state"));
        let removed = sweep_once(git_data_path, &purgatory, Duration::from_secs(1));
        assert_eq!(removed, 0, "repo with refs must be retained");
        assert!(repo.exists());
    }
}
