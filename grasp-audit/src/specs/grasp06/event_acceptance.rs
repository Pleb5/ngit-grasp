//! GRASP-06 event-acceptance relaxation tests
//!
//! Spec: <https://github.com/DanConwayDev/grasp/blob/main/06.md>
//!
//! Covers 06.md lines 21–24: PR and PR Update events that would otherwise be
//! rejected under GRASP-01 for not referencing an accepted repository
//! announcement MUST instead be accepted, provided the event:
//!
//! 1. has an `a` tag of the form `30617:<pubkey>:<identifier>`, AND
//! 2. has a `clone` tag naming this service's
//!    `/prs/<signer-npub>/<identifier>.git` endpoint.
//!
//! Both tests build kind-1618 PR events for a coord this relay has **no
//! accepted announcement** for. The only difference between them is the
//! `clone` tag: test 5 names this relay's `/prs/` endpoint (expected to be
//! accepted under the relaxation), test 6 names a different host (expected
//! to remain rejected).
//!
//! ## Why kind 1618 only
//!
//! The spec treats PRs (kind 1618) and PR Updates (kind 1619) identically.
//! rust-nostr already handles kind discrimination; covering 1619 separately
//! would only test that we typed the same number twice. Skipping it.

use crate::specs::grasp06::SpecRef;
use crate::{AuditClient, TestResult};
use nostr_sdk::prelude::*;

pub struct EventAcceptanceTests;

impl EventAcceptanceTests {
    /// Test: a PR event for a coord this relay has no announcement for, but
    /// carrying a `clone` tag naming this relay's `/prs/<signer-npub>/<d>.git`,
    /// MUST be accepted (purgatory or served).
    ///
    /// Spec: 06.md lines 21–24.
    ///
    /// ## Setup (no fixtures)
    ///
    /// - Random hex pubkey + random identifier for the `a` tag — guarantees
    ///   no accepted announcement for the coord exists on the relay.
    /// - `pr_author_keys` as signer (distinct from owner/maintainer; matches
    ///   the existing PR fixture conventions).
    /// - `c` tag: synthetic 64-hex commit hash. The spec contract here is
    ///   event acceptance, not git-data release from purgatory.
    /// - `clone` tag: this relay's `/prs/<pr-author-npub>/<d>.git`.
    ///
    /// ## TDD posture
    ///
    /// Pre-implementation this FAILS: GRASP-01's PR policy rejects PR events
    /// without a matching accepted announcement, and Phase 6's relaxation
    /// branch hasn't landed yet. Once that branch lands the test turns green.
    pub async fn test_pr_event_accepted_when_clone_tag_names_prs_endpoint(
        client: &AuditClient,
    ) -> TestResult {
        TestResult::new(
            "pr_event_accepted_when_clone_tag_names_prs_endpoint",
            SpecRef::Grasp06RelaxAcceptPrEvent,
            "MUST accept PR event for un-announced coord when its clone tag names this \
             relay's /prs/<signer-npub>/<identifier>.git endpoint",
        )
        .run(|| async {
            // 1. Resolve this relay's HTTP base URL — used to build the
            //    `clone` tag value that must match the relay's own /prs/
            //    endpoint.
            let ws_url = client
                .relay_url()
                .await
                .map_err(|e| format!("Failed to get relay URL: {}", e))?;
            let http_url = AuditClient::ws_to_http_url(&ws_url)
                .map_err(|e| format!("Failed to convert WebSocket URL to HTTP: {}", e))?;

            // 2. Build a fresh, never-announced coordinate. Using a random
            //    pubkey + UUID identifier ensures no accepted announcement
            //    for this `a` value could exist on the relay — so the test
            //    exercises specifically the GRASP-06 relaxation branch, not
            //    the standard GRASP-01 acceptance path.
            let target_pubkey_hex = Keys::generate().public_key().to_hex();
            let identifier = format!("audit-grasp06-{}", uuid::Uuid::new_v4());
            let a_tag_value = format!("30617:{}:{}", target_pubkey_hex, identifier);

            // 3. Build the `clone` URL. Signer is pr_author_keys (the
            //    contributor identity); identifier matches the `a` tag's
            //    d-tag, as the spec requires.
            let pr_author_npub = client
                .pr_author_keys()
                .public_key()
                .to_bech32()
                .map_err(|e| format!("Failed to bech32-encode pr_author npub: {}", e))?;
            let clone_url = format!(
                "{}/prs/{}/{}.git",
                http_url.trim_end_matches('/'),
                pr_author_npub,
                identifier
            );

            // 4. Synthesise a 64-hex commit for the `c` tag. No matching git
            //    data exists; the spec contract here is event acceptance,
            //    not purgatory release.
            let commit_hex = Keys::generate().public_key().to_hex();

            // 5. Build and sign the PR event. Signed by pr_author_keys so
            //    the eventual /prs/ URL's `<signer-npub>` matches the
            //    event's signer, which is the invariant the relaxation
            //    enforces.
            let event = client
                .event_builder(
                    Kind::GitPullRequest,
                    "grasp-06 audit: PR for un-announced coord, clone tag on /prs/",
                )
                .tag(Tag::custom(TagKind::custom("a"), vec![a_tag_value]))
                .tag(Tag::custom(TagKind::custom("c"), vec![commit_hex]))
                .tag(Tag::custom(TagKind::custom("clone"), vec![clone_url]))
                .build(client.pr_author_keys())
                .map_err(|e| format!("Failed to build PR event: {}", e))?;

            // 6. Send. send_event_and_note_purgatory returns Ok on
            //    acceptance regardless of whether the event was served or
            //    placed in purgatory — both satisfy the spec's "MUST accept".
            //    A rejection comes back as Err with the relay's message.
            client
                .send_event_and_note_purgatory(event)
                .await
                .map_err(|e| {
                    format!(
                        "Expected relay to ACCEPT the PR event (per GRASP-06 06.md lines 21–24): \
                         clone tag named this relay's /prs/<signer-npub>/<d>.git and the `a` tag \
                         was well-formed, but the relay rejected it. Relay error: {}",
                        e
                    )
                })?;

            Ok(())
        })
        .await
    }

    /// Test: a PR event for a coord this relay has no announcement for, with
    /// a `clone` tag pointing somewhere OTHER than this relay's `/prs/`
    /// endpoint, MUST remain rejected.
    ///
    /// Spec: 06.md lines 23–24 — the relaxation applies only when the event's
    /// `clone` tag names this relay's `/prs/` endpoint.
    ///
    /// ## TDD posture
    ///
    /// This test PASSES today, pre-implementation: GRASP-01's PR policy
    /// already rejects PR events without a matching accepted announcement,
    /// regardless of clone-tag content. Once GRASP-06's relaxation branch
    /// lands it stays green — the relaxation must NOT fire for foreign clone
    /// tags. If a future change accidentally widens the relaxation (e.g.
    /// accepting any PR with an `a` tag and any `clone` tag), this test will
    /// catch it.
    ///
    /// In the audit CLI report this test is skipped when GRASP-06 isn't
    /// advertised: the rejection is unambiguous proof of GRASP-01 rejection
    /// (which is fine and necessary) but doesn't tell us whether the
    /// relaxation branch would correctly NOT fire if it existed. Only
    /// meaningful when the feature is enabled.
    pub async fn test_pr_event_rejected_when_clone_tag_does_not_name_prs_endpoint(
        client: &AuditClient,
    ) -> TestResult {
        TestResult::new(
            "pr_event_rejected_when_clone_tag_does_not_name_prs_endpoint",
            SpecRef::Grasp06RelaxRequiresCloneTag,
            "MUST NOT relax PR acceptance when the event's clone tag does not name this \
             relay's /prs/<signer-npub>/<identifier>.git endpoint",
        )
        .run(|| async {
            // 1. Same setup shape as test 5: fresh coord with no
            //    announcement, synthetic 64-hex commit, signed by
            //    pr_author_keys.
            let target_pubkey_hex = Keys::generate().public_key().to_hex();
            let identifier = format!("audit-grasp06-{}", uuid::Uuid::new_v4());
            let a_tag_value = format!("30617:{}:{}", target_pubkey_hex, identifier);
            let commit_hex = Keys::generate().public_key().to_hex();

            // 2. Foreign clone URL — points at a host that is definitely not
            //    this relay. Path shape mimics a /prs/ URL so an
            //    over-permissive relaxation that ignored the host check
            //    would still wrongly accept; this catches that bug.
            let pr_author_npub = client
                .pr_author_keys()
                .public_key()
                .to_bech32()
                .map_err(|e| format!("Failed to bech32-encode pr_author npub: {}", e))?;
            let foreign_clone_url = format!(
                "https://other-relay.example.invalid/prs/{}/{}.git",
                pr_author_npub, identifier
            );

            // 3. Build and sign.
            let event = client
                .event_builder(
                    Kind::GitPullRequest,
                    "grasp-06 audit: PR for un-announced coord, clone tag on foreign host",
                )
                .tag(Tag::custom(TagKind::custom("a"), vec![a_tag_value]))
                .tag(Tag::custom(TagKind::custom("c"), vec![commit_hex]))
                .tag(Tag::custom(
                    TagKind::custom("clone"),
                    vec![foreign_clone_url],
                ))
                .build(client.pr_author_keys())
                .map_err(|e| format!("Failed to build PR event: {}", e))?;

            // 4. Send and require rejection. send_event* methods translate
            //    a relay-reported failure into Err — so Ok here means the
            //    relay accepted the event, which is the spec violation we
            //    are guarding against.
            match client.send_event_and_note_purgatory(event).await {
                Ok(_) => Err(
                    "Relay ACCEPTED a PR event for an un-announced coord whose clone tag did NOT \
                     name this relay's /prs/ endpoint. GRASP-06 06.md lines 23–24 require the \
                     relaxation to apply only when the clone tag names this relay; this is either \
                     a missing host check in the relaxation branch or a regression in the \
                     baseline GRASP-01 rejection."
                        .to_string(),
                ),
                Err(_) => Ok(()),
            }
        })
        .await
    }
}
