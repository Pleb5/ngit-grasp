//! GRASP-06 cross-service mirror tests
//!
//! Spec source: design doc at
//! `docs/explanation/grasp-06-contributor-pr-submission.md`, "Cross-service
//! mirror" section. The contract is derived from the design — the 06.md spec
//! itself does not name this behaviour — but the design decision is settled
//! and the tests below pin it down so a regression would be visible.
//!
//! Two tests, complementary directions:
//!
//! - [`MirroringTests::test_prs_push_mirrors_to_announced_repo`]
//!   A `refs/nostr/<event-id>` push received at `/prs/<contributor>/<id>.git`,
//!   when paired with a PR event whose `a` tag references an accepted
//!   announcement on this relay, MUST appear at the announced repo too. The
//!   announced repo is the natural location for clients browsing the project.
//!
//! - [`MirroringTests::test_standard_push_does_not_mirror_to_prs`]
//!   The reverse direction MUST NOT mirror: a push to the standard
//!   `/<npub>/<id>.git` endpoint must not appear under `/prs/`. The /prs/
//!   namespace is a contributor-submission side-channel; conflating
//!   maintainer pushes into it would invent hosting locations the maintainer
//!   never declared.
//!
//! ## Fixture reuse
//!
//! Test 7 reuses [`FixtureKind::ValidRepoServed`] (which transitively triggers
//! `OwnerStateDataPushed` — see `src/fixtures.rs` deps map) to obtain an
//! accepted, served repo announcement to mirror INTO. Test 8 reuses
//! [`FixtureKind::PREvent2Served`] which already pushes a PR ref to the
//! standard endpoint; that is exactly the "push to standard, assert absent
//! from /prs/" precondition for the reverse-mirror check.
//!
//! ## Timing
//!
//! Mirroring is kicked off asynchronously from
//! `process_newly_available_git_data` after the /prs/ receive-pack completes.
//! Pre-implementation there is nothing to wait for; post-implementation a
//! short fixed wait is sufficient because the work is local to the relay
//! process (no network round-trip). We use 2 seconds — generous enough to
//! avoid flake on CI, short enough to keep the test suite responsive.

use crate::specs::grasp06::SpecRef;
use crate::{
    create_commit, init_local_repo, try_push_to_ref, AuditClient, FixtureKind, TestContext,
    TestResult,
};
use nostr_sdk::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub struct MirroringTests;

impl MirroringTests {
    /// Test 7: a `refs/nostr/<event-id>` push to `/prs/<contributor>/<id>.git`
    /// MUST be mirrored into the announced repo on this relay.
    ///
    /// Spec ref: [`SpecRef::Grasp06MirrorToAnnouncedRepo`] (design-doc derived).
    ///
    /// ## Setup
    ///
    /// 1. Ensure an accepted, served announcement at `<owner-npub>/<repo-id>.git`
    ///    via `FixtureKind::ValidRepoServed` (transitively pushes owner git data).
    /// 2. Materialise a throwaway commit locally and capture its hash. We
    ///    intentionally do NOT depend on the deterministic-commit helper
    ///    here — the spec contract is "c tag equals pushed commit", not
    ///    "commit equals some constant". Tying the test to
    ///    `PR_TEST_COMMIT_HASH` would couple it to commit-machinery
    ///    invariants this test doesn't care about; any hash will do as long
    ///    as we use the same one in both places.
    /// 3. Build a kind-1618 PR event signed by `pr_author_keys`:
    ///    - `a` tag references the owner's coord (`30617:<owner-hex>:<repo-id>`).
    ///    - `c` tag = the hash from step 2.
    ///    - `clone` tag = this relay's `/prs/<pr-author-npub>/<repo-id>.git`.
    /// 4. Publish the PR event. Acceptance via the standard GRASP-01 path is
    ///    expected (the coord IS announced); purgatory is fine too — the spec
    ///    contract under test is the mirror, not the acceptance path.
    /// 5. Push `HEAD:refs/nostr/<pr-event-id>` from the local repo to the
    ///    /prs/ URL.
    /// 6. Wait 2 s for the relay's async mirror step to complete.
    /// 7. Verify the mirror: fetch `refs/nostr/<pr-event-id>` from the
    ///    standard `<owner>/<repo-id>.git` URL, then `git cat-file -e` on
    ///    the expected commit hash — proves both that the ref is present AND
    ///    that its commit object is reachable (the user's verification
    ///    choice — "clone the maintainer repo and verify commit reachable").
    ///
    /// ## TDD posture
    ///
    /// Pre-implementation this fails: there is no /prs/ route at all, so the
    /// push gets 404. Once Phase 4 (receive-pack) and Phase 7 (mirror hook)
    /// land, the test turns green.
    pub async fn test_prs_push_mirrors_to_announced_repo(client: &AuditClient) -> TestResult {
        TestResult::new(
            "prs_push_mirrors_to_announced_repo",
            SpecRef::Grasp06MirrorToAnnouncedRepo,
            "refs accepted via /prs/ MUST be mirrored into any matching accepted-announcement \
             repos on this relay",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            // 1. Ensure an accepted, served announcement on the standard
            //    endpoint. ValidRepoServed transitively triggers
            //    OwnerStateDataPushed so the maintainer's git data is also
            //    present — that's the "destination" the mirror copies into.
            let repo = ctx
                .get_fixture(FixtureKind::ValidRepoServed)
                .await
                .map_err(|e| format!("Failed to build ValidRepoServed fixture: {}", e))?;

            // 2. Resolve the coordinates needed for the test:
            //    - owner identity (signer of the announcement)
            //    - repo identifier (`d` tag of the announcement)
            //    - this relay's HTTP base for building URLs.
            let owner_pubkey_hex = repo.pubkey.to_hex();
            let owner_npub = repo
                .pubkey
                .to_bech32()
                .map_err(|e| format!("Failed to bech32-encode owner npub: {}", e))?;
            let repo_id = extract_repo_id(&repo)?;

            let ws_url = client
                .relay_url()
                .await
                .map_err(|e| format!("Failed to get relay URL: {}", e))?;
            let http_base = AuditClient::ws_to_http_url(&ws_url)
                .map_err(|e| format!("Failed to convert WebSocket URL to HTTP: {}", e))?;
            let http_base = http_base.trim_end_matches('/').to_string();

            let pr_author_npub = client
                .pr_author_keys()
                .public_key()
                .to_bech32()
                .map_err(|e| format!("Failed to bech32-encode pr_author npub: {}", e))?;

            let prs_url = format!("{}/prs/{}/{}.git", http_base, pr_author_npub, repo_id);
            let standard_url = format!("{}/{}/{}.git", http_base, owner_npub, repo_id);

            // 3. Local commit first. Materialising the commit BEFORE the PR
            //    event lets us pin the event's `c` tag to the exact hash we
            //    are about to push, decoupling this test from any
            //    deterministic-commit invariants. The commit content is
            //    arbitrary — only the hash matters.
            let workspace = TempPath::new("grasp06-mirror-fwd-");
            init_local_repo(&workspace.path, &prs_url)
                .map_err(|e| format!("Failed to init local workspace: {}", e))?;

            let commit_hash =
                create_commit(&workspace.path, "grasp-06 audit: PR commit for mirror test")
                    .map_err(|e| format!("Failed to create local commit: {}", e))?;

            // 4. Build & publish the PR event. The c tag pins the commit
            //    hash we materialised in step 3 — purgatory release for
            //    the matching push requires this match.
            let pr_event = client
                .event_builder(
                    Kind::GitPullRequest,
                    "grasp-06 audit: PR via /prs/ should mirror to announced repo",
                )
                .tag(Tag::custom(
                    TagKind::custom("a"),
                    vec![format!("30617:{}:{}", owner_pubkey_hex, repo_id)],
                ))
                .tag(Tag::custom(TagKind::custom("c"), vec![commit_hash.clone()]))
                .tag(Tag::custom(TagKind::custom("clone"), vec![prs_url.clone()]))
                .build(client.pr_author_keys())
                .map_err(|e| format!("Failed to build PR event: {}", e))?;
            let pr_event_id = pr_event.id.to_hex();

            // send_event_and_note_purgatory is OK both for served and
            // purgatory acceptance — it only Errs on relay rejection, which
            // would be a setup failure for this test.
            client
                .send_event_and_note_purgatory(pr_event)
                .await
                .map_err(|e| format!("Relay rejected the PR event during test setup: {}", e))?;

            // 5. Push HEAD:refs/nostr/<pr-event-id> to the /prs/ URL.
            let refname = format!("refs/nostr/{}", pr_event_id);
            let push_ok = try_push_to_ref(&workspace.path, &refname).map_err(|e| {
                format!(
                    "git push HEAD:{} to {} failed to execute: {}",
                    refname, prs_url, e
                )
            })?;
            if !push_ok {
                return Err(format!(
                    "git push HEAD:{} to {} was rejected by the relay",
                    refname, prs_url
                ));
            }

            // 6. Wait for the async mirror to run.
            tokio::time::sleep(Duration::from_secs(2)).await;

            // 7. Verify the ref appears at <owner>/<repo-id>.git AND its
            //    commit object is reachable. This is the "clone the
            //    maintainer repo and verify commit reachable" check.
            let verify = TempPath::new("grasp06-mirror-fwd-verify-");
            init_local_repo(&verify.path, &standard_url)
                .map_err(|e| format!("Failed to init verify workspace: {}", e))?;

            // Fetch exactly the ref we expect to have been mirrored. If the
            // mirror didn't run the fetch fails (or returns nothing); if it
            // did, the ref lands locally pointing at the expected commit.
            let fetch_refspec = format!("{}:{}", refname, refname);
            let fetch_out = run_git(&verify.path, &["fetch", "origin", &fetch_refspec]);
            match fetch_out {
                Ok(out) if out.status.success() => {}
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    return Err(format!(
                        "Failed to fetch {} from {}: {} — mirror does not appear to have run",
                        refname,
                        standard_url,
                        stderr.trim()
                    ));
                }
                Err(e) => return Err(format!("Failed to execute git fetch: {}", e)),
            }

            // Object-reachability: cat-file -e exits 0 iff the object exists
            // in the local object store (which now includes whatever the
            // mirror copied over).
            let cat_out = run_git(&verify.path, &["cat-file", "-e", &commit_hash])
                .map_err(|e| format!("Failed to execute git cat-file: {}", e))?;
            if !cat_out.status.success() {
                let stderr = String::from_utf8_lossy(&cat_out.stderr);
                return Err(format!(
                    "Mirror's commit object {} is not reachable in {}: {}",
                    commit_hash,
                    standard_url,
                    stderr.trim()
                ));
            }

            // Also sanity-check that the local ref now points at the same
            // commit — guards against a (currently hypothetical) bug where
            // the relay mirrors objects but writes a stale ref value.
            let rev_out = run_git(&verify.path, &["rev-parse", &refname])
                .map_err(|e| format!("Failed to execute git rev-parse: {}", e))?;
            if !rev_out.status.success() {
                let stderr = String::from_utf8_lossy(&rev_out.stderr);
                return Err(format!(
                    "git rev-parse {} failed after fetch: {}",
                    refname,
                    stderr.trim()
                ));
            }
            let mirrored_hash = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();
            if mirrored_hash != commit_hash {
                return Err(format!(
                    "Mirrored ref {} points at {} but expected {}",
                    refname, mirrored_hash, commit_hash
                ));
            }

            Ok(())
        })
        .await
    }

    /// Test 8: a `refs/nostr/<event-id>` push to the standard
    /// `<npub>/<id>.git` endpoint MUST NOT appear under `/prs/`.
    ///
    /// Spec ref: [`SpecRef::Grasp06NoReverseMirror`] (design-doc derived).
    ///
    /// ## Setup
    ///
    /// 1. Use [`FixtureKind::PREvent2Served`] which:
    ///    - publishes a kind-1618 PR event signed by `pr_author_keys`,
    ///    - pushes a matching `refs/nostr/<pr-event-id>` to the standard
    ///      `<owner>/<repo-id>.git` endpoint,
    ///    - confirms the event is served (i.e. released from purgatory).
    ///
    ///    No /prs/ activity is involved.
    /// 2. Probe `/prs/<pr-author-npub>/<repo-id>.git` with `git ls-remote`
    ///    filtered to `refs/nostr/<pr-event-id>`.
    ///
    /// ## Pass condition
    ///
    /// `ls-remote` exits 0 with empty stdout for the filtered refname.
    /// Per 06.md line 13 the endpoint MUST respond as an empty bare repo
    /// when nothing was pushed to it; an absent ref + clean exit is exactly
    /// the "empty repo" shape. A non-empty stdout for the filter means the
    /// reverse-mirror fired — that's the regression this test guards.
    ///
    /// ## TDD posture
    ///
    /// Pre-implementation this fails: `/prs/` doesn't route, so `ls-remote`
    /// errors with 404 and the test reports the failure as a setup error.
    /// Once Phase 3 (routing) and the empty-repo synthesis land, the
    /// endpoint becomes reachable and this test goes green — and stays
    /// green as long as the relay never adds a reverse-mirror branch.
    pub async fn test_standard_push_does_not_mirror_to_prs(client: &AuditClient) -> TestResult {
        TestResult::new(
            "standard_push_does_not_mirror_to_prs",
            SpecRef::Grasp06NoReverseMirror,
            "the reverse direction MUST NOT mirror: a push to /<npub>/<id>.git must not appear \
             under /prs/",
        )
        .run(|| async {
            let ctx = TestContext::new(client);

            // 1. Ensure a PR ref is present at the standard endpoint. The
            //    fixture asserts the event is served, so the standard push
            //    has unambiguously happened by the time we get here.
            let pr_event = ctx
                .get_fixture(FixtureKind::PREvent2Served)
                .await
                .map_err(|e| format!("Failed to build PREvent2Served fixture: {}", e))?;
            let pr_event_id = pr_event.id.to_hex();

            // 2. The fixture's PR event is keyed against the owner's
            //    coord. Extract the d-tag from the underlying repo
            //    announcement (ValidRepoServed) so we can build the matching
            //    /prs/<pr-author>/<repo-id>.git URL.
            let repo = ctx
                .get_fixture(FixtureKind::ValidRepoServed)
                .await
                .map_err(|e| format!("Failed to build ValidRepoServed fixture: {}", e))?;
            let repo_id = extract_repo_id(&repo)?;

            // 3. Build the /prs/ URL for the contributor (pr_author).
            let ws_url = client
                .relay_url()
                .await
                .map_err(|e| format!("Failed to get relay URL: {}", e))?;
            let http_base = AuditClient::ws_to_http_url(&ws_url)
                .map_err(|e| format!("Failed to convert WebSocket URL to HTTP: {}", e))?;
            let http_base = http_base.trim_end_matches('/').to_string();
            let pr_author_npub = client
                .pr_author_keys()
                .public_key()
                .to_bech32()
                .map_err(|e| format!("Failed to bech32-encode pr_author npub: {}", e))?;
            let prs_url = format!("{}/prs/{}/{}.git", http_base, pr_author_npub, repo_id);

            // 4. ls-remote with a refname filter. We do NOT introduce a
            //    test-local wait here — the standard push completed before
            //    PREvent2Served returned, so any reverse-mirror would have
            //    had ample time to run. A negative result here is a real
            //    "didn't happen, won't happen" signal.
            let refname = format!("refs/nostr/{}", pr_event_id);
            let out = Command::new("git")
                .args(["ls-remote", &prs_url, &refname])
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .map_err(|e| format!("Failed to execute git ls-remote {}: {}", prs_url, e))?;

            if !out.status.success() {
                // A failed ls-remote almost certainly means the /prs/
                // endpoint doesn't exist (pre-implementation 404) or the
                // relay refused. Either way this test can't make its
                // assertion meaningfully, so surface it as a clear setup
                // failure rather than passing trivially.
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(format!(
                    "git ls-remote {} {} failed (exit {}): {} — /prs/ endpoint must be reachable \
                     for this test to assert the reverse-mirror contract (06.md line 13 requires \
                     an empty-bare-repo response for any well-formed /prs/ path)",
                    prs_url,
                    refname,
                    out.status.code().unwrap_or(-1),
                    stderr.trim()
                ));
            }

            // 5. Pass condition: empty stdout. With the refname filter set,
            //    any non-empty line is exactly the reverse-mirror regression
            //    we are guarding against.
            let stdout = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            if !lines.is_empty() {
                return Err(format!(
                    "Reverse mirror appears to have fired: standard endpoint push for PR event \
                     {} is also visible at {} — `git ls-remote ... {}` returned:\n  {}",
                    pr_event_id,
                    prs_url,
                    refname,
                    lines.join("\n  ")
                ));
            }

            Ok(())
        })
        .await
    }
}

// =============================================================================
// Helpers — kept module-local; promote to the audit lib if a second consumer
// wants them.
// =============================================================================

/// Extract the `d` tag (repo identifier) from a kind-30617 repo announcement.
fn extract_repo_id(repo: &Event) -> Result<String, String> {
    repo.tags
        .iter()
        .find(|t| t.kind() == TagKind::d())
        .and_then(|t| t.content())
        .map(str::to_string)
        .ok_or_else(|| "Missing `d` tag in repo announcement".to_string())
}

/// A temp directory that wipes itself on drop. Used here to keep the verify
/// and push workspaces from leaking on test failure paths.
struct TempPath {
    path: PathBuf,
}

impl TempPath {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!("{}{}", prefix, uuid::Uuid::new_v4()));
        let _ = fs::remove_dir_all(&path);
        Self { path }
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Run a `git` command in `cwd` and return the [`std::process::Output`]
/// unchanged. The caller decides whether non-zero exit is failure.
fn run_git(cwd: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
}
