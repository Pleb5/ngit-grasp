//! GRASP-01 Specification Requirements
//!
//! Embedded specification requirements from the GRASP-01 spec document.
//! This is the single source of truth for spec text displayed in audit reports.

/// GRASP spec repository commit ID that this version is based on
pub const GRASP_COMMIT_ID: &str = "1fdb8f7";

/// A single specification requirement
#[derive(Debug, Clone)]
pub struct SpecRequirement {
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

/// All GRASP-01 specification requirements
pub const GRASP_01_REQUIREMENTS: &[SpecRequirement] = &[
    // Nostr Relay section
    SpecRequirement {
        line: 7,
        section: "Nostr Relay",
        text: "MUST serve a NIP-01 compliant nostr relay at `/` that accepts git repository announcements and their corresponding repo state announcements.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 9,
        section: "Nostr Relay",
        text: "MUST reject git repository announcements that do not list the service in both `clone` and `relays` tags unless implementing `GRASP-05`.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 11,
        section: "Nostr Relay",
        text: "MAY reject git repository announcements based on other criteria such as pre-payment, quotas, WoT, whitelist, SPAM prevention, etc.",
        level: RequirementLevel::May,
    },
    SpecRequirement {
        line: 13,
        section: "Nostr Relay",
        text: "MUST accept other events that tag, or are tagged by, either: 1. accepted git repository announcements; or 2. accepted issues or patches",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 18,
        section: "Nostr Relay",
        text: "MAY reject or delete events for generic SPAM prevention reasons or curation eg. WoT, whitelist, user bans and banned topics.",
        level: RequirementLevel::May,
    },
    SpecRequirement {
        line: 20,
        section: "Nostr Relay",
        text: "MUST serve a NIP-11 document",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 22,
        section: "Nostr Relay",
        text: "MUST list each supported GRASP under `supported_grasps` in format `GRASP-XX` eg `GRASP-01` as a string array",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 23,
        section: "Nostr Relay",
        text: "MUST list repository acceptance criteria under `repo_acceptance_criteria` as a human readable string",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 24,
        section: "Nostr Relay",
        text: "MUST list brief summary of curation policy under `curation` if events are curated beyond generic SPAM prevention; otherwise `curation` MUST be omitted",
        level: RequirementLevel::Must,
    },
    // Git Smart HTTP Service section
    SpecRequirement {
        line: 28,
        section: "Git Smart HTTP Service",
        text: "MUST serve a git repository via an unauthenticated git smart http service at `/<npub>/<identifier>.git` for each accepted git repository announcement.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 30,
        section: "Git Smart HTTP Service",
        text: "MUST accept pushes via this service that match the latest repo state announcement on the relay, respecting the recursive maintainer set.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 32,
        section: "Git Smart HTTP Service",
        text: "MUST set repository HEAD per repo state announcement as soon as the git data related to that branch has been received.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 34,
        section: "Git Smart HTTP Service",
        text: "MUST accept pushes via this service to `refs/nostr/<event-id>` but SHOULD reject if event exists on relay listing a different tip and MAY reject based on criteria such as size, SPAM prevention, etc. SHOULD delete and MAY garbage collect these refs if no corresponding git PR event or git PR update event, with a `c` tag that matches the ref tip, is accepted by relay within 20 minutes.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 36,
        section: "Git Smart HTTP Service",
        text: "MUST include `allow-reachable-sha1-in-want` and `allow-tip-sha1-in-want` in advertisement and serve available oids.",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 38,
        section: "Git Smart HTTP Service",
        text: "SHOULD serve a webpage at the same endpoint linking to git nostr client(s) to browse the repository and a 404 page for repositories it doesn't host.",
        level: RequirementLevel::Should,
    },
    // CORS Support section
    SpecRequirement {
        line: 44,
        section: "CORS Support",
        text: "Set `Access-Control-Allow-Origin: *` on ALL responses",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 45,
        section: "CORS Support",
        text: "Set `Access-Control-Allow-Methods: GET, POST` on ALL responses",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 46,
        section: "CORS Support",
        text: "Set `Access-Control-Allow-Headers: Content-Type` on ALL responses",
        level: RequirementLevel::Must,
    },
    SpecRequirement {
        line: 47,
        section: "CORS Support",
        text: "Respond to OPTIONS requests with 204 No Content",
        level: RequirementLevel::Must,
    },
];

/// Get a requirement by line number
pub fn get_requirement(line: u32) -> Option<&'static SpecRequirement> {
    GRASP_01_REQUIREMENTS.iter().find(|r| r.line == line)
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
    fn test_get_sections() {
        let sections = get_sections();
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0], "Nostr Relay");
        assert_eq!(sections[1], "Git Smart HTTP Service");
        assert_eq!(sections[2], "CORS Support");
    }

    #[test]
    fn test_requirement_count() {
        assert_eq!(GRASP_01_REQUIREMENTS.len(), 19);
    }
}
