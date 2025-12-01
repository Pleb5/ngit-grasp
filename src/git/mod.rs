//! Git Smart HTTP Backend
//!
//! This module implements Git Smart HTTP protocol support for ngit-grasp.
//! It provides handlers for clone, fetch, and push operations over HTTP.
//!
//! # Architecture
//!
//! - `protocol` - Git pkt-line format parsing and utilities
//! - `subprocess` - Git process spawning and management
//! - `handlers` - HTTP request handlers for Git operations
//!
//! # URL Patterns
//!
//! The following URL patterns are supported:
//! - `GET /<npub>/<identifier>.git/info/refs?service=git-upload-pack` - Clone/fetch advertisement
//! - `GET /<npub>/<identifier>.git/info/refs?service=git-receive-pack` - Push advertisement
//! - `POST /<npub>/<identifier>.git/git-upload-pack` - Clone/fetch operation
//! - `POST /<npub>/<identifier>.git/git-receive-pack` - Push operation

pub mod authorization;
pub mod handlers;
pub mod protocol;
pub mod subprocess;

use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info};

/// Parse a Git repository path from URL components
///
/// Converts /<npub>/<identifier>.git/* to a filesystem path
///
/// # Arguments
/// * `git_data_path` - Base directory for Git repositories
/// * `npub` - The npub (Nostr public key in bech32 format)
/// * `identifier` - The repository identifier
///
/// # Returns
/// Path to the bare Git repository
pub fn resolve_repo_path(git_data_path: &str, npub: &str, identifier: &str) -> PathBuf {
    // Remove .git suffix if present
    let identifier = identifier.strip_suffix(".git").unwrap_or(identifier);

    PathBuf::from(git_data_path)
        .join(npub)
        .join(format!("{}.git", identifier))
}

/// Check if a commit exists in the repository
///
/// # Arguments
/// * `repo_path` - Path to the bare git repository
/// * `commit_hash` - The commit hash to check
///
/// # Returns
/// True if the commit exists in the repository, false otherwise
pub fn commit_exists(repo_path: &Path, commit_hash: &str) -> bool {
    let output = Command::new("git")
        .args(["cat-file", "-t", commit_hash])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                let obj_type = String::from_utf8_lossy(&result.stdout);
                // Object exists and is a commit
                obj_type.trim() == "commit"
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

/// Set the repository HEAD to point to a branch
///
/// This updates the HEAD symbolic ref to point to the specified branch.
/// Per GRASP-01: "MUST set repository HEAD per repository state announcement
/// as soon as the git data related to that branch has been received."
///
/// # Arguments
/// * `repo_path` - Path to the bare git repository
/// * `head_ref` - The ref to set HEAD to (e.g., "refs/heads/main")
///
/// # Returns
/// Ok(()) if successful, Err with error message otherwise
pub fn set_repository_head(repo_path: &Path, head_ref: &str) -> Result<(), String> {
    // Validate the ref format
    if !head_ref.starts_with("refs/heads/") {
        return Err(format!(
            "Invalid HEAD ref: {} (must start with refs/heads/)",
            head_ref
        ));
    }

    debug!("Setting HEAD to {} in {}", head_ref, repo_path.display());

    let output = Command::new("git")
        .args(["symbolic-ref", "HEAD", head_ref])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute git symbolic-ref: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git symbolic-ref failed: {}", stderr));
    }

    info!("Updated HEAD to {} in {}", head_ref, repo_path.display());
    Ok(())
}

/// Try to set repository HEAD from a repository state event
///
/// This function checks if the HEAD branch's commit is available in the repository
/// and sets HEAD if it is. This should be called:
/// 1. When a repository state event is received (in case git data already exists)
/// 2. After git data is received (in case a state event was already received)
///
/// # Arguments
/// * `repo_path` - Path to the bare git repository
/// * `head_ref` - The ref to set HEAD to (e.g., "refs/heads/main")
/// * `head_commit` - The commit hash that the HEAD branch should point to
///
/// # Returns
/// Ok(true) if HEAD was set, Ok(false) if commit not yet available, Err on failure
pub fn try_set_head_if_available(
    repo_path: &Path,
    head_ref: &str,
    head_commit: &str,
) -> Result<bool, String> {
    // Check if repository exists
    if !repo_path.exists() {
        debug!(
            "Repository not found at {}, cannot set HEAD",
            repo_path.display()
        );
        return Ok(false);
    }

    // Check if the commit exists in the repository
    if !commit_exists(repo_path, head_commit) {
        debug!(
            "Commit {} not found in {}, HEAD not set yet",
            head_commit,
            repo_path.display()
        );
        return Ok(false);
    }

    // Commit exists, set HEAD
    set_repository_head(repo_path, head_ref)?;
    Ok(true)
}

/// Get the commit hash that a ref points to
///
/// # Arguments
/// * `repo_path` - Path to the bare git repository
/// * `ref_name` - The ref name (e.g., "refs/nostr/<event-id>")
///
/// # Returns
/// Some(commit_hash) if the ref exists, None otherwise
pub fn get_ref_commit(repo_path: &Path, ref_name: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", ref_name])
        .current_dir(repo_path)
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Delete a git ref from the repository
///
/// # Arguments
/// * `repo_path` - Path to the bare git repository
/// * `ref_name` - The ref name to delete (e.g., "refs/nostr/<event-id>")
///
/// # Returns
/// Ok(()) if successful, Err with error message otherwise
pub fn delete_ref(repo_path: &Path, ref_name: &str) -> Result<(), String> {
    debug!("Deleting ref {} from {}", ref_name, repo_path.display());

    let output = Command::new("git")
        .args(["update-ref", "-d", ref_name])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute git update-ref: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git update-ref -d failed: {}", stderr));
    }

    info!("Deleted ref {} from {}", ref_name, repo_path.display());
    Ok(())
}

/// Validate refs/nostr/<event-id> ref against expected commit
///
/// If the ref exists but points to a different commit than expected,
/// the ref is deleted. This is called when a PR event is received to
/// ensure refs/nostr refs are consistent with their corresponding events.
///
/// # Arguments
/// * `repo_path` - Path to the bare git repository
/// * `event_id` - The event ID (hex string)
/// * `expected_commit` - The commit hash from the event's `c` tag
///
/// # Returns
/// Ok(true) if ref was deleted (mismatch), Ok(false) if no action taken, Err on failure
pub fn validate_nostr_ref(
    repo_path: &Path,
    event_id: &str,
    expected_commit: &str,
) -> Result<bool, String> {
    let ref_name = format!("refs/nostr/{}", event_id);

    // Check if repository exists
    if !repo_path.exists() {
        debug!(
            "Repository not found at {}, skipping ref validation",
            repo_path.display()
        );
        return Ok(false);
    }

    // Check if the ref exists
    let current_commit = match get_ref_commit(repo_path, &ref_name) {
        Some(commit) => commit,
        None => {
            debug!("Ref {} does not exist in {}", ref_name, repo_path.display());
            return Ok(false);
        }
    };

    // Compare commits
    if current_commit == expected_commit {
        debug!(
            "Ref {} points to correct commit {} in {}",
            ref_name,
            expected_commit,
            repo_path.display()
        );
        return Ok(false);
    }

    // Commit mismatch - delete the ref
    info!(
        "Deleting mismatched ref {} in {}: expected {}, found {}",
        ref_name,
        repo_path.display(),
        expected_commit,
        current_commit
    );
    delete_ref(repo_path, &ref_name)?;
    Ok(true)
}

/// Get the current HEAD ref from a repository
///
/// # Arguments
/// * `repo_path` - Path to the bare git repository
///
/// # Returns
/// The current HEAD ref (e.g., "refs/heads/main") or None if not set
pub fn get_repository_head(repo_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["symbolic-ref", "HEAD"])
        .current_dir(repo_path)
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Extract npub and identifier from a Git URL path
///
/// Parses paths like `/<npub>/<identifier>.git/info/refs`
///
/// Returns (npub, identifier, subpath) where subpath is the part after .git/
pub fn parse_git_url(path: &str) -> Option<(&str, &str, &str)> {
    // Remove leading slash
    let path = path.strip_prefix('/').unwrap_or(path);

    // Split into components
    let parts: Vec<&str> = path.splitn(3, '/').collect();

    if parts.len() < 3 {
        return None;
    }

    let npub = parts[0];
    let repo_part = parts[1];
    let subpath = parts[2];

    // Extract identifier (remove .git suffix if present for the middle part)
    let identifier = repo_part.strip_suffix(".git").unwrap_or(repo_part);

    Some((npub, identifier, subpath))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a test bare repository with optional commits
    fn create_test_repo() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("test.git");

        // Initialize bare repository
        Command::new("git")
            .args(["init", "--bare", repo_path.to_str().unwrap()])
            .output()
            .unwrap();

        (temp_dir, repo_path)
    }

    /// Create a test repository with a commit on a branch
    fn create_test_repo_with_commit() -> (TempDir, PathBuf, String) {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = temp_dir.path().join("work");
        let bare_repo = temp_dir.path().join("test.git");

        // Initialize bare repository
        Command::new("git")
            .args([
                "init",
                "--bare",
                "--initial-branch=main",
                bare_repo.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        // Clone to working directory
        Command::new("git")
            .args([
                "clone",
                bare_repo.to_str().unwrap(),
                work_dir.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        // Configure git for commits
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&work_dir)
            .output()
            .unwrap();

        // Create a file and commit
        fs::write(work_dir.join("README.md"), "# Test").unwrap();
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&work_dir)
            .output()
            .unwrap();

        // Get commit hash
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&work_dir)
            .output()
            .unwrap();
        let commit_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Push to bare repo
        Command::new("git")
            .args(["push", "origin", "main"])
            .current_dir(&work_dir)
            .output()
            .unwrap();

        (temp_dir, bare_repo, commit_hash)
    }

    #[test]
    fn test_resolve_repo_path() {
        let path = resolve_repo_path("/data/git", "npub1abc123", "my-repo");
        assert_eq!(path, PathBuf::from("/data/git/npub1abc123/my-repo.git"));
    }

    #[test]
    fn test_resolve_repo_path_with_git_suffix() {
        let path = resolve_repo_path("/data/git", "npub1abc123", "my-repo.git");
        assert_eq!(path, PathBuf::from("/data/git/npub1abc123/my-repo.git"));
    }

    #[test]
    fn test_parse_git_url_info_refs() {
        let (npub, id, subpath) = parse_git_url("/npub1abc/repo.git/info/refs").unwrap();
        assert_eq!(npub, "npub1abc");
        assert_eq!(id, "repo");
        assert_eq!(subpath, "info/refs");
    }

    #[test]
    fn test_parse_git_url_upload_pack() {
        let (npub, id, subpath) = parse_git_url("/npub1abc/repo.git/git-upload-pack").unwrap();
        assert_eq!(npub, "npub1abc");
        assert_eq!(id, "repo");
        assert_eq!(subpath, "git-upload-pack");
    }

    #[test]
    fn test_parse_git_url_invalid() {
        assert!(parse_git_url("/npub1abc").is_none());
        assert!(parse_git_url("/npub1abc/repo").is_none());
    }

    #[test]
    fn test_commit_exists_nonexistent() {
        let (_temp_dir, repo_path) = create_test_repo();
        assert!(!commit_exists(
            &repo_path,
            "deadbeef1234567890abcdef1234567890abcdef"
        ));
    }

    #[test]
    fn test_commit_exists_with_commit() {
        let (_temp_dir, repo_path, commit_hash) = create_test_repo_with_commit();
        assert!(commit_exists(&repo_path, &commit_hash));
    }

    #[test]
    fn test_set_repository_head() {
        let (_temp_dir, repo_path, _commit_hash) = create_test_repo_with_commit();

        // Default HEAD might be refs/heads/master
        let result = set_repository_head(&repo_path, "refs/heads/main");
        assert!(result.is_ok());

        let head = get_repository_head(&repo_path);
        assert_eq!(head, Some("refs/heads/main".to_string()));
    }

    #[test]
    fn test_set_repository_head_invalid_ref() {
        let (_temp_dir, repo_path) = create_test_repo();

        // Invalid ref format should fail
        let result = set_repository_head(&repo_path, "main");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must start with refs/heads/"));
    }

    #[test]
    fn test_try_set_head_if_available_commit_missing() {
        let (_temp_dir, repo_path) = create_test_repo();

        let result = try_set_head_if_available(
            &repo_path,
            "refs/heads/main",
            "deadbeef1234567890abcdef1234567890abcdef",
        );

        // Should return Ok(false) - commit not found
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_try_set_head_if_available_success() {
        let (_temp_dir, repo_path, commit_hash) = create_test_repo_with_commit();

        let result = try_set_head_if_available(&repo_path, "refs/heads/main", &commit_hash);

        // Should return Ok(true) - HEAD was set
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Verify HEAD was set
        let head = get_repository_head(&repo_path);
        assert_eq!(head, Some("refs/heads/main".to_string()));
    }
}
