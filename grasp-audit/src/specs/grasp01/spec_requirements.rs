//! GRASP-01 Specification Requirements
//!
//! Embedded specification requirements from the GRASP-01 spec document.
//! This is the single source of truth for spec text displayed in audit reports.

/// GRASP spec repository commit ID that this version is based on
pub const GRASP_COMMIT_ID: &str = "1fdb8f7";

/// Reference to a specific GRASP-01 specification requirement
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpecRef {
    NostrRelayNip01Compliant,
    NostrRelayRejectMissingCloneRelays,
    NostrRelayMayRejectOtherCriteria,
    NostrRelayMustAcceptTaggedEvents,
    NostrRelayMayRejectSpamCuration,
    PurgatoryAcceptUntilGitData,
    Nip11ServeDocument,
    Nip11ListSupportedGrasps,
    Nip11ListRepoAcceptanceCriteria,
    Nip11ListCurationPolicy,
    GitServeRepository,
    GitAcceptPushesAlignState,
    GitSetHeadOnReceive,
    GitAcceptRefsNostrEventId,
    GitIncludeAllowSha1InWant,
    GitServeWebpage,
    CorsAllowOrigin,
    CorsAllowMethods,
    CorsAllowHeaders,
    CorsOptionsResponse,
}

/// A single specification requirement
#[derive(Debug, Clone)]
pub struct SpecRequirement {
    /// Unique reference to this requirement
    pub spec_ref: SpecRef,
    /// Line number in the spec document
    pub line: u32,
    /// Section name (e.g., "Nostr Relay", "Git Smart HTTP Service", "CORS Support")
    pub section: &'static str,
    /// The full requirement text
    pub text: &'static str,
    /// Requirement level: MUST, SHOULD, or MAY
    pub level: RequirementLevel,
}

/// Requirement level per RFC 2119
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementLevel {
    Must,
    Should,
    May,
}

impl std::fmt::Display for RequirementLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequirementLevel::Must => write!(f, "MUST"),
            RequirementLevel::Should => write!(f, "SHOULD"),
            RequirementLevel::May => write!(f, "MAY"),
        }
    }
}

impl SpecRef {
    /// Get the spec reference string in format "GRASP-01:section:line"
    pub fn spec_ref_string(self) -> &'static str {
        match self {
            SpecRef::NostrRelayNip01Compliant => "GRASP-01:nostr-relay:7",
            SpecRef::NostrRelayRejectMissingCloneRelays => "GRASP-01:nostr-relay:9",
            SpecRef::NostrRelayMayRejectOtherCriteria => "GRASP-01:nostr-relay:11",
            SpecRef::NostrRelayMustAcceptTaggedEvents => "GRASP-01:nostr-relay:13",
            SpecRef::NostrRelayMayRejectSpamCuration => "GRASP-01:nostr-relay:18",
            SpecRef::PurgatoryAcceptUntilGitData => "GRASP-01:purgatory:22",
            SpecRef::Nip11ServeDocument => "GRASP-01:nip-11:26",
            SpecRef::Nip11ListSupportedGrasps => "GRASP-01:nip-11:28",
            SpecRef::Nip11ListRepoAcceptanceCriteria => "GRASP-01:nip-11:29",
            SpecRef::Nip11ListCurationPolicy => "GRASP-01:nip-11:30",
            SpecRef::GitServeRepository => "GRASP-01:git-http:34",
            SpecRef::GitAcceptPushesAlignState => "GRASP-01:git-http:36",
            SpecRef::GitSetHeadOnReceive => "GRASP-01:git-http:39",
            SpecRef::GitAcceptRefsNostrEventId => "GRASP-01:git-http:45",
            SpecRef::GitIncludeAllowSha1InWant => "GRASP-01:git-http:56",
            SpecRef::GitServeWebpage => "GRASP-01:git-http:58",
            SpecRef::CorsAllowOrigin => "GRASP-01:cors:64",
            SpecRef::CorsAllowMethods => "GRASP-01:cors:65",
            SpecRef::CorsAllowHeaders => "GRASP-01:cors:66",
            SpecRef::CorsOptionsResponse => "GRASP-01:cors:67",
        }
    }
}

/// All GRASP-01 specification requirements
pub const GRASP_01_REQUIREMENTS: &[SpecRequirement] = &[
    // Nostr Relay section
    SpecRequirement {
        spec_ref: SpecRef::NostrRelayNip01Compliant,
        line: 7,
        section: "Nostr Relay",
        text: "MUST serve a NIP-01 compliant nostr relay at `/` that accepts git repository announcements and their corresponding repo state announcements.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::NostrRelayRejectMissingCloneRelays,
        line: 9,
        section: "Nostr Relay",
        text: "MUST reject git repository announcements that do not list the service in both `clone` and `relays` tags unless implementing `GRASP-05`.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::NostrRelayMayRejectOtherCriteria,
        line: 11,
        section: "Nostr Relay",
        text: "MAY reject git repository announcements based on other criteria such as pre-payment, quotas, WoT, whitelist, SPAM prevention, etc.",
        level: RequirementLevel::May,
    },
    SpecRequirement {
        spec_ref: SpecRef::NostrRelayMustAcceptTaggedEvents,
        line: 13,
        section: "Nostr Relay",
        text: "MUST accept other events that tag, or are tagged by, either: 1. accepted git repository announcements; or 2. accepted issues or patches",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::NostrRelayMayRejectSpamCuration,
        line: 18,
        section: "Nostr Relay",
        text: "MAY reject or delete events for generic SPAM prevention reasons or curation eg. WoT, whitelist, user bans and banned topics.",
        level: RequirementLevel::May,
    },
    SpecRequirement {
        spec_ref: SpecRef::PurgatoryAcceptUntilGitData,
        line: 22,
        section: "Purgatory",
        text: "New repository announcements, repo state announcements, PRs and PR Updates SHOULD be accepted with message \"purgatory: won't be served until git data arrives\" and kept in purgatory (not served) until the related git data arrives and otherwise discarded after 30 minutes.",
        level: RequirementLevel::Should,
    },
    SpecRequirement {
        spec_ref: SpecRef::Nip11ServeDocument,
        line: 26,
        section: "NIP-11",
        text: "MUST serve a NIP-11 document",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::Nip11ListSupportedGrasps,
        line: 28,
        section: "NIP-11",
        text: "MUST list each supported GRASP under `supported_grasps` in format `GRASP-XX` eg `GRASP-01` as a string array",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::Nip11ListRepoAcceptanceCriteria,
        line: 29,
        section: "NIP-11",
        text: "MUST list repository acceptance criteria under `repo_acceptance_criteria` as a human readable string",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::Nip11ListCurationPolicy,
        line: 30,
        section: "NIP-11",
        text: "MUST list brief summary of curation policy under `curation` if events are curated beyond generic SPAM prevention; otherwise `curation` MUST be omitted",
        level: RequirementLevel::Must,
    },
    // Git Smart HTTP Service section
    SpecRequirement {
        spec_ref: SpecRef::GitServeRepository,
        line: 34,
        section: "Git Smart HTTP Service",
        text: "MUST serve a git repository via an unauthenticated git smart http service at `/<npub>/<identifier>.git` for each git repository announcement the relay serves or has in purgatory.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::GitAcceptPushesAlignState,
        line: 36,
        section: "Git Smart HTTP Service",
        text: "MUST accept pushes via this service that fully align the git repository state with a repo state announcement in purgatory that is authorised for this repository, respecting the recursive maintainer set.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::GitSetHeadOnReceive,
        line: 39,
        section: "Git Smart HTTP Service",
        text: "As soon as the `receive-pack` is successful, the server MUST: 1. Release the event (and related repository announcement) from purgatory. 2. Align the repository HEAD with the repo state announcement. 3. Synchronize git state with other git repositories on the server for which this state event is authoritative.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::GitAcceptRefsNostrEventId,
        line: 45,
        section: "Git Smart HTTP Service",
        text: "MUST accept pushes via this service to `refs/nostr/<event-id>` but SHOULD reject if the event exists in purgatory listing a different tip, and MAY reject based on criteria such as size, SPAM prevention, etc.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::GitIncludeAllowSha1InWant,
        line: 56,
        section: "Git Smart HTTP Service",
        text: "MUST include `allow-reachable-sha1-in-want` and `allow-tip-sha1-in-want` in advertisement and serve available oids.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::GitServeWebpage,
        line: 58,
        section: "Git Smart HTTP Service",
        text: "SHOULD serve a webpage at the same endpoint linking to git nostr client(s) to browse the repository and a 404 page for repositories it doesn't host.",
        level: RequirementLevel::Should,
    },
    // CORS Support section
    SpecRequirement {
        spec_ref: SpecRef::CorsAllowOrigin,
        line: 64,
        section: "CORS Support",
        text: "Set `Access-Control-Allow-Origin: *` on ALL responses",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::CorsAllowMethods,
        line: 65,
        section: "CORS Support",
        text: "Set `Access-Control-Allow-Methods: GET, POST` on ALL responses",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::CorsAllowHeaders,
        line: 66,
        section: "CORS Support",
        text: "Set `Access-Control-Allow-Headers: Content-Type` on ALL responses",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        spec_ref: SpecRef::CorsOptionsResponse,
        line: 67,
        section: "CORS Support",
        text: "Respond to OPTIONS requests with 204 No Content",
        level: RequirementLevel::Must,
    },
];

/// Get a requirement by line number
pub fn get_requirement(line: u32) -> Option<&'static SpecRequirement> {
    GRASP_01_REQUIREMENTS.iter().find(|r| r.line == line)
}

/// Get a requirement by its SpecRef
pub fn get_requirement_by_ref(spec_ref: SpecRef) -> Option<&'static SpecRequirement> {
    GRASP_01_REQUIREMENTS
        .iter()
        .find(|r| r.spec_ref == spec_ref)
}

/// Get all requirements for a section
pub fn get_requirements_for_section(section: &str) -> Vec<&'static SpecRequirement> {
    GRASP_01_REQUIREMENTS
        .iter()
        .filter(|r| r.section == section)
        .collect()
}

/// Get all unique section names in order
pub fn get_sections() -> Vec<&'static str> {
    let mut sections = Vec::new();
    for req in GRASP_01_REQUIREMENTS {
        if !sections.contains(&req.section) {
            sections.push(req.section);
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_requirement() {
        let req = get_requirement(7).expect("Line 7 should exist");
        assert_eq!(req.section, "Nostr Relay");
        assert!(req.text.contains("NIP-01"));
    }

    #[test]
    fn test_get_requirement_by_ref() {
        let req = get_requirement_by_ref(SpecRef::NostrRelayNip01Compliant)
            .expect("SpecRef should exist");
        assert_eq!(req.line, 7);
        assert_eq!(req.spec_ref, SpecRef::NostrRelayNip01Compliant);
    }

    #[test]
    fn test_get_sections() {
        let sections = get_sections();
        assert_eq!(sections.len(), 5);
        assert_eq!(sections[0], "Nostr Relay");
        assert_eq!(sections[1], "Purgatory");
        assert_eq!(sections[2], "NIP-11");
        assert_eq!(sections[3], "Git Smart HTTP Service");
        assert_eq!(sections[4], "CORS Support");
    }

    #[test]
    fn test_requirement_count() {
        assert_eq!(GRASP_01_REQUIREMENTS.len(), 20);
    }

    #[test]
    fn test_spec_ref_unique() {
        let mut refs = std::collections::HashSet::new();
        for req in GRASP_01_REQUIREMENTS {
            assert!(
                refs.insert(req.spec_ref),
                "Duplicate SpecRef found: {:?}",
                req.spec_ref
            );
        }
    }
}
