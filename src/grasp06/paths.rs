//! On-disk path conventions for the GRASP-06 `/prs/` endpoint.
//!
//! Repositories submitted via `/prs/<npub>/<identifier>.git` are stored
//! under a dedicated subtree of the git data directory so existing
//! subsystems (cleanup, sync, listings) can exclude them with a single
//! path-prefix check. The submitter pubkey is stored as **hex** on disk;
//! the identifier is stored verbatim (URL percent-decoding happens at
//! parse time in [`crate::grasp06::endpoint`]).
//!
//! ```text
//! <git_data_path>/
//!   prs/                           <-- PRS_DISK_PREFIX
//!     <submitter-hex>/
//!       <identifier>.git/
//! ```

use std::path::{Path, PathBuf};

/// URL prefix used in HTTP paths: `/prs/<npub>/<identifier>.git/...`.
pub const PRS_URL_PREFIX: &str = "prs";

/// Directory name used on disk under `<git_data_path>`. Kept identical to
/// `PRS_URL_PREFIX` so an operator scanning the filesystem can match the
/// URL space at a glance.
pub const PRS_DISK_PREFIX: &str = "prs";

/// Return the base directory under which all `/prs/` repos live.
pub fn prs_base_path(git_data_path: &Path) -> PathBuf {
    git_data_path.join(PRS_DISK_PREFIX)
}

/// Build the on-disk path for `/prs/<npub>/<identifier>.git`.
///
/// `submitter_hex` is the 64-character lowercase hex of the submitter's
/// public key. `identifier` is the percent-decoded NIP-34 `d` value as
/// stored on disk (verbatim).
pub fn prs_repo_path(git_data_path: &Path, submitter_hex: &str, identifier: &str) -> PathBuf {
    let identifier = identifier.strip_suffix(".git").unwrap_or(identifier);
    prs_base_path(git_data_path)
        .join(submitter_hex)
        .join(format!("{}.git", identifier))
}

/// True if `path` is inside the `/prs/` subtree of `git_data_path`.
///
/// Used by future cleanup and sync subsystems to skip these repos —
/// they have their own lifecycle (zero-ref cleanup, no announcement
/// promotion) and must not be treated as standard owner repos.
pub fn is_prs_repo_path(path: &Path, git_data_path: &Path) -> bool {
    path.starts_with(prs_base_path(git_data_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prs_base_path_appends_prs() {
        assert_eq!(
            prs_base_path(Path::new("/data/git")),
            PathBuf::from("/data/git/prs")
        );
    }

    #[test]
    fn prs_repo_path_uses_hex_pubkey() {
        let p = prs_repo_path(Path::new("/data/git"), "deadbeef", "my-repo");
        assert_eq!(p, PathBuf::from("/data/git/prs/deadbeef/my-repo.git"));
    }

    #[test]
    fn prs_repo_path_strips_trailing_git() {
        let p = prs_repo_path(Path::new("/data/git"), "deadbeef", "my-repo.git");
        assert_eq!(p, PathBuf::from("/data/git/prs/deadbeef/my-repo.git"));
    }

    #[test]
    fn is_prs_repo_path_detects_subtree() {
        let root = Path::new("/data/git");
        assert!(is_prs_repo_path(
            &PathBuf::from("/data/git/prs/abc/repo.git"),
            root
        ));
        assert!(!is_prs_repo_path(
            &PathBuf::from("/data/git/npub1xyz/repo.git"),
            root
        ));
    }
}
