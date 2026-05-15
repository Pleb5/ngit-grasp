//! GRASP-06 — Contributor Pull Request Submission.
//!
//! Spec: <https://github.com/DanConwayDev/grasp/blob/main/06.md>
//! Design: `docs/explanation/grasp-06-contributor-pr-submission.md`
//!
//! This module is gated behind [`crate::config::Config::grasp06_enable`].
//! When disabled, `/prs/*` requests fall through to the standard 404 path.
//!
//! ## Current scope
//!
//! - URL parsing for `/prs/<npub>/<identifier>.git/<subpath>`.
//! - On-disk path conventions for future phases.
//! - Empty-bare-repo synthesis for `info/refs` and `git-upload-pack`.
//! - A stub `git-receive-pack` handler that returns an HTTP 200 ERR pkt-line.
//!
//! Real receive-pack, purgatory scoping, event-acceptance relaxation, and
//! cross-service mirroring are not yet implemented.

pub mod endpoint;
pub mod fetch;
pub mod paths;
