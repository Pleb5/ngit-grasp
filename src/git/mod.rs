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

pub mod handlers;
pub mod protocol;
pub mod subprocess;

use std::path::PathBuf;

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
    let identifier = if repo_part.ends_with(".git") {
        &repo_part[..repo_part.len() - 4]
    } else {
        repo_part
    };
    
    Some((npub, identifier, subpath))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_repo_path() {
        let path = resolve_repo_path(
            "/data/git",
            "npub1abc123",
            "my-repo"
        );
        assert_eq!(
            path,
            PathBuf::from("/data/git/npub1abc123/my-repo.git")
        );
    }

    #[test]
    fn test_resolve_repo_path_with_git_suffix() {
        let path = resolve_repo_path(
            "/data/git",
            "npub1abc123",
            "my-repo.git"
        );
        assert_eq!(
            path,
            PathBuf::from("/data/git/npub1abc123/my-repo.git")
        );
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
}