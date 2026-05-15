//! GRASP-06 Specification Requirements
//!
//! Embedded specification requirements from the GRASP-06 spec document.
//! Spec: <https://github.com/DanConwayDev/grasp/blob/main/06.md>
//!
//! This is the single source of truth for spec text displayed in audit reports.

/// GRASP spec repository commit ID this version is based on.
///
/// Update this when bumping to a newer spec revision so reports indicate
/// which spec line numbers are being referenced.
pub const GRASP_06_COMMIT_ID: &str = "DRAFT";

/// Reference to a specific GRASP-06 specification requirement.
///
/// Each variant maps 1:1 to a single MUST/SHOULD line in `06.md`, or to an
/// audit-derived invariant that follows directly from the spec (e.g. the
/// "advertise-or-404" gate, which is implied by NIP-11 discovery semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpecRef {
    /// Audit-derived: if NIP-11 does not advertise GRASP-06, the /prs/ namespace
    /// must 404 so clients can rely on NIP-11 for capability discovery.
    Grasp06NotAdvertised404,
    /// 06.md line 13 — MUST respond to upload-pack on any well-formed path as
    /// if serving an empty bare repository.
    Grasp06FetchEmptyRepo,
    /// 06.md line 15 — MUST accept pushes to `refs/nostr/<event-id>`.
    Grasp06AcceptRefsNostrPush,
    /// 06.md line 15 — MUST reject pushes to any other ref namespace.
    Grasp06RejectNonNostrRefs,
    /// 06.md lines 21–24 — MUST accept PR/PR-Update events that would otherwise
    /// be rejected by GRASP-01, when they carry a matching `a` tag AND a `clone`
    /// tag naming this relay's /prs/ endpoint.
    Grasp06RelaxAcceptPrEvent,
    /// 06.md lines 23–24 — the relaxation applies only when the event's `clone`
    /// tag names this relay's /prs/ endpoint; otherwise the event is rejected.
    Grasp06RelaxRequiresCloneTag,
    /// Design-doc derived (docs/explanation/grasp-06-contributor-pr-submission.md
    /// "Cross-service mirror") — refs accepted via /prs/ are mirrored into any
    /// matching accepted-announcement repos on this relay.
    Grasp06MirrorToAnnouncedRepo,
    /// Design-doc derived — the reverse direction does NOT mirror: a push to
    /// the standard `/<npub>/<id>.git` endpoint must not appear under /prs/.
    Grasp06NoReverseMirror,
}

impl SpecRef {
    /// Get the spec reference string in format "GRASP-06:section:line".
    pub fn spec_ref_string(self) -> &'static str {
        match self {
            SpecRef::Grasp06NotAdvertised404 => "GRASP-06:audit:nip11-gate",
            SpecRef::Grasp06FetchEmptyRepo => "GRASP-06:git-http:13",
            SpecRef::Grasp06AcceptRefsNostrPush => "GRASP-06:git-http:15",
            SpecRef::Grasp06RejectNonNostrRefs => "GRASP-06:git-http:15",
            SpecRef::Grasp06RelaxAcceptPrEvent => "GRASP-06:event-acceptance:21",
            SpecRef::Grasp06RelaxRequiresCloneTag => "GRASP-06:event-acceptance:23",
            SpecRef::Grasp06MirrorToAnnouncedRepo => "GRASP-06:design:mirror-forward",
            SpecRef::Grasp06NoReverseMirror => "GRASP-06:design:mirror-no-reverse",
        }
    }
}

impl crate::result::SpecRefStr for SpecRef {
    fn spec_ref_string(&self) -> &'static str {
        SpecRef::spec_ref_string(*self)
    }
}
