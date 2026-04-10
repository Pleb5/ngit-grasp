//! Cleanup Empty Repositories
//!
//! Scans the LMDB database for kind 30617 (repository announcement) events whose
//! corresponding bare git repository on disk is empty (no refs) or missing entirely.
//! For each such repository, also removes any kind 30618 (state) events for the same
//! (pubkey, identifier) coordinate.
//!
//! ## Rationale
//!
//! A relay should not store announcement or state events for a repository that has no
//! git data. If the bare repo is empty or absent, the events are stale and should be
//! removed so the relay does not serve them.
//!
//! ## Usage
//!
//! ```text
//! # Dry-run (default): print what would be deleted
//! ngit-grasp cleanup-empty-repos --relay-data-path /var/lib/ngit-grasp/relay \
//!                                 --git-data-path   /var/lib/ngit-grasp/git
//!
//! # Execute: delete the bare repos and remove events from the DB
//! ngit-grasp cleanup-empty-repos --relay-data-path /var/lib/ngit-grasp/relay \
//!                                 --git-data-path   /var/lib/ngit-grasp/git \
//!                                 --execute
//! ```
//!
//! The relay service should be stopped before running with `--execute` to avoid
//! races with the live relay process.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Args;
use nostr_lmdb::NostrLmdb;
use nostr_sdk::prelude::*;

use crate::nostr::events::RepositoryAnnouncement;

/// Arguments for the `cleanup-empty-repos` subcommand.
#[derive(Debug, Args)]
pub struct CleanupArgs {
    /// Path to the LMDB relay data directory (contains the nostr event database).
    ///
    /// Defaults to `./data/relay` (same default as the relay itself).
    #[arg(long, env = "NGIT_RELAY_DATA_PATH", default_value = "./data/relay")]
    pub relay_data_path: String,

    /// Path to the git data directory (contains bare repositories).
    ///
    /// Defaults to `./data/git` (same default as the relay itself).
    #[arg(long, env = "NGIT_GIT_DATA_PATH", default_value = "./data/git")]
    pub git_data_path: String,

    /// Actually delete empty repositories and remove their events from the database.
    ///
    /// Without this flag the command runs in dry-run mode and only prints what
    /// would be deleted. Stop the relay service before using this flag.
    #[arg(long, default_value_t = false)]
    pub execute: bool,
}

/// A repository that has an empty (or missing) bare git repo on disk.
#[derive(Debug)]
struct EmptyRepo {
    /// The kind 30617 event
    announcement: Event,
    /// Derived npub (bech32) of the owner
    npub: String,
    /// Repository identifier (d-tag value)
    identifier: String,
    /// Absolute path to the bare repo directory
    repo_path: PathBuf,
    /// Whether the directory exists at all (vs exists but is empty)
    repo_exists: bool,
    /// Any kind 30618 state events found in the local DB for this coordinate
    state_events: Vec<Event>,
}

/// Run the cleanup-empty-repos subcommand.
pub async fn run(args: &CleanupArgs) -> Result<()> {
    let relay_data_path = Path::new(&args.relay_data_path);
    let git_data_path = Path::new(&args.git_data_path);

    if args.execute {
        println!("=== cleanup-empty-repos (EXECUTE MODE) ===");
        println!("WARNING: This will permanently delete data. The relay should be stopped.");
        println!();
    } else {
        println!("=== cleanup-empty-repos (DRY-RUN MODE) ===");
        println!("Pass --execute to actually delete. Stop the relay first.");
        println!();
    }

    println!("Relay data path : {}", relay_data_path.display());
    println!("Git data path   : {}", git_data_path.display());
    println!();

    // Open the LMDB database
    println!("Opening LMDB database...");
    let database: Arc<dyn NostrDatabase> = Arc::new(
        NostrLmdb::open(relay_data_path)
            .await
            .with_context(|| format!("Failed to open LMDB at {}", relay_data_path.display()))?,
    );
    println!("Database opened.");
    println!();

    // Query all kind 30617 events
    let filter = Filter::new().kind(Kind::GitRepoAnnouncement);
    let announcements = database
        .query(filter)
        .await
        .context("Failed to query kind 30617 events")?;

    println!("Found {} kind 30617 announcement(s) in database.", announcements.len());
    println!();

    // Identify empty repos
    let mut empty_repos: Vec<EmptyRepo> = Vec::new();

    for event in announcements.iter() {
        let announcement = match RepositoryAnnouncement::from_event(event.clone()) {
            Ok(a) => a,
            Err(e) => {
                eprintln!(
                    "  WARN: Could not parse announcement {} (skipping): {}",
                    event.id.to_hex(),
                    e
                );
                continue;
            }
        };

        let npub = announcement.owner_npub();
        let identifier = announcement.identifier.clone();
        let repo_path = git_data_path.join(&announcement.repo_path());

        let (repo_exists, is_empty) = check_repo_empty(&repo_path);

        if !is_empty {
            // Repo has git data — leave it alone
            continue;
        }

        // Look up any kind 30618 state events for this (pubkey, identifier) in the local DB
        let state_filter = Filter::new()
            .kind(Kind::RepoState)
            .author(event.pubkey)
            .identifier(identifier.clone());

        let state_events = database
            .query(state_filter)
            .await
            .with_context(|| {
                format!(
                    "Failed to query kind 30618 for {}/{}",
                    npub, identifier
                )
            })?;

        empty_repos.push(EmptyRepo {
            announcement: event.clone(),
            npub,
            identifier,
            repo_path,
            repo_exists,
            state_events: state_events.into_iter().collect(),
        });
    }

    if empty_repos.is_empty() {
        println!("No empty repositories found. Nothing to do.");
        return Ok(());
    }

    // Print report
    println!(
        "Found {} repository/repositories with empty or missing git data:\n",
        empty_repos.len()
    );

    for (i, repo) in empty_repos.iter().enumerate() {
        let repo_status = if repo.repo_exists {
            "exists but empty (no refs)"
        } else {
            "missing from disk"
        };
        println!(
            "  [{:>3}] {}/{} — git repo {}",
            i + 1,
            repo.npub,
            repo.identifier,
            repo_status,
        );
        println!(
            "         30617 event : {}",
            repo.announcement.id.to_hex()
        );
        if repo.state_events.is_empty() {
            println!("         30618 events: none in local DB");
        } else {
            for se in &repo.state_events {
                println!("         30618 event : {}", se.id.to_hex());
            }
        }
        println!(
            "         repo path   : {}",
            repo.repo_path.display()
        );
    }

    println!();

    if !args.execute {
        println!(
            "DRY-RUN: {} repository/repositories would be cleaned up.",
            empty_repos.len()
        );
        println!("Run with --execute to perform the cleanup (stop the relay first).");
        return Ok(());
    }

    // Execute: delete repos and remove events
    println!("Executing cleanup...");
    println!();

    let mut deleted_repos = 0usize;
    let mut failed_repos = 0usize;
    let mut deleted_announcements = 0usize;
    let mut deleted_state_events = 0usize;

    for repo in &empty_repos {
        println!("Cleaning up {}/{}...", repo.npub, repo.identifier);

        // 1. Delete the bare repo directory (if it exists)
        if repo.repo_exists {
            match std::fs::remove_dir_all(&repo.repo_path) {
                Ok(()) => {
                    println!("  Deleted git repo: {}", repo.repo_path.display());
                    deleted_repos += 1;

                    // Remove the parent npub directory if now empty
                    if let Some(npub_dir) = repo.repo_path.parent() {
                        if npub_dir.exists() {
                            match std::fs::read_dir(npub_dir) {
                                Ok(mut entries) => {
                                    if entries.next().is_none() {
                                        if let Err(e) = std::fs::remove_dir(npub_dir) {
                                            eprintln!(
                                                "  WARN: Could not remove empty npub dir {}: {}",
                                                npub_dir.display(),
                                                e
                                            );
                                        } else {
                                            println!(
                                                "  Removed empty npub dir: {}",
                                                npub_dir.display()
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!(
                                        "  WARN: Could not read npub dir {}: {}",
                                        npub_dir.display(),
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "  ERROR: Failed to delete git repo {}: {}",
                        repo.repo_path.display(),
                        e
                    );
                    failed_repos += 1;
                    // Continue — still try to remove the DB events
                }
            }
        }

        // 2. Remove the kind 30617 announcement from the DB
        //    Use a filter matching the specific event ID so we only delete this exact event.
        let announcement_filter = Filter::new()
            .kind(Kind::GitRepoAnnouncement)
            .id(repo.announcement.id);

        match database.delete(announcement_filter).await {
            Ok(()) => {
                println!("  Deleted 30617 event: {}", repo.announcement.id.to_hex());
                deleted_announcements += 1;
            }
            Err(e) => {
                eprintln!(
                    "  ERROR: Failed to delete 30617 event {}: {}",
                    repo.announcement.id.to_hex(),
                    e
                );
            }
        }

        // 3. Remove any kind 30618 state events for this coordinate
        if !repo.state_events.is_empty() {
            let state_filter = Filter::new()
                .kind(Kind::RepoState)
                .author(repo.announcement.pubkey)
                .identifier(repo.identifier.clone());

            match database.delete(state_filter).await {
                Ok(()) => {
                    for se in &repo.state_events {
                        println!("  Deleted 30618 event: {}", se.id.to_hex());
                        deleted_state_events += 1;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "  ERROR: Failed to delete 30618 events for {}/{}: {}",
                        repo.npub, repo.identifier, e
                    );
                }
            }
        }
    }

    println!();
    println!("=== Cleanup complete ===");
    println!("  Git repos deleted       : {}", deleted_repos);
    if failed_repos > 0 {
        println!("  Git repos failed        : {} (see errors above)", failed_repos);
    }
    println!("  30617 events removed    : {}", deleted_announcements);
    println!("  30618 events removed    : {}", deleted_state_events);

    Ok(())
}

/// Check whether a bare git repository is empty (has no refs).
///
/// Returns `(exists, is_empty)`:
/// - `(false, true)` — path does not exist (treated as empty)
/// - `(true, true)`  — path exists but `git for-each-ref` returns no output
/// - `(true, false)` — path exists and has at least one ref
fn check_repo_empty(repo_path: &Path) -> (bool, bool) {
    if !repo_path.exists() {
        return (false, true);
    }

    // Run `git for-each-ref --git-dir=<path>` — empty output means no refs
    let output = Command::new("git")
        .args(["for-each-ref", "--format=%(refname)"])
        .arg("--git-dir")
        .arg(repo_path)
        .output();

    match output {
        Ok(out) => {
            // Trim whitespace; if nothing remains, the repo is empty
            let stdout = String::from_utf8_lossy(&out.stdout);
            let is_empty = stdout.trim().is_empty();
            (true, is_empty)
        }
        Err(_) => {
            // Could not run git — treat as empty to be safe (will be reported)
            (true, true)
        }
    }
}
