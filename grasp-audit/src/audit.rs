//! Audit configuration and event tagging

use nostr_sdk::prelude::*;

/// Audit configuration
#[derive(Debug, Clone)]
pub struct AuditConfig {
    /// Unique ID for this audit run
    pub run_id: String,
    
    /// Mode: CI (isolated) or Production (live)
    pub mode: AuditMode,
    
    /// Cleanup timestamp (events can be cleaned after this)
    pub cleanup_after: Timestamp,
    
    /// Whether to actually create events or just query
    pub read_only: bool,
}

/// Audit mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditMode {
    /// Isolated CI/CD tests - only see own events
    CI,
    
    /// Production audit - see all events, minimal writes
    Production,
}

impl AuditConfig {
    /// Create config for CI/CD testing
    pub fn ci() -> Self {
        let run_id = format!("ci-{}", uuid::Uuid::new_v4());
        Self {
            run_id,
            mode: AuditMode::CI,
            cleanup_after: Timestamp::now() + 3600, // 1 hour from now
            read_only: false,
        }
    }
    
    /// Create config for production audit
    pub fn production() -> Self {
        let run_id = format!("prod-audit-{}", Timestamp::now().as_u64());
        Self {
            run_id,
            mode: AuditMode::Production,
            cleanup_after: Timestamp::now() + 300, // 5 minutes from now
            read_only: true, // Default to read-only for production
        }
    }
    
    /// Create config with custom run ID
    pub fn with_run_id(run_id: String, mode: AuditMode) -> Self {
        Self {
            run_id,
            mode,
            cleanup_after: Timestamp::now() + 3600,
            read_only: mode == AuditMode::Production,
        }
    }
    
    /// Get audit tags for an event
    pub fn audit_tags(&self) -> Vec<Tag> {
        use nostr_sdk::prelude::{Alphabet, SingleLetterTag};
        
        vec![
            // Use single-letter tags for filtering support
            // "g" = grasp-audit marker
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::G)),
                vec!["grasp-audit"]
            ),
            // "r" = audit run ID
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::R)),
                vec![self.run_id.clone()]
            ),
            // "c" = cleanup timestamp
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::C)),
                vec![self.cleanup_after.to_string()]
            ),
        ]
    }
}

/// Builder for audit events
pub struct AuditEventBuilder {
    kind: Kind,
    content: String,
    tags: Vec<Tag>,
    config: AuditConfig,
}

impl AuditEventBuilder {
    /// Create a new audit event builder
    pub fn new(kind: Kind, content: impl Into<String>, config: AuditConfig) -> Self {
        Self {
            kind,
            content: content.into(),
            tags: Vec::new(),
            config,
        }
    }
    
    /// Add a tag
    pub fn tag(mut self, tag: Tag) -> Self {
        self.tags.push(tag);
        self
    }
    
    /// Add multiple tags
    pub fn tags(mut self, tags: Vec<Tag>) -> Self {
        self.tags.extend(tags);
        self
    }
    
    /// Build the event with audit tags
    pub fn build(self, keys: &Keys) -> anyhow::Result<Event> {
        let mut all_tags = self.tags;
        all_tags.extend(self.config.audit_tags());
        
        let event = EventBuilder::new(self.kind, self.content)
            .tags(all_tags)
            .sign_with_keys(keys)?;
        
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_ci_config() {
        let config = AuditConfig::ci();
        assert_eq!(config.mode, AuditMode::CI);
        assert!(!config.read_only);
        assert!(config.run_id.starts_with("ci-"));
    }
    
    #[test]
    fn test_production_config() {
        let config = AuditConfig::production();
        assert_eq!(config.mode, AuditMode::Production);
        assert!(config.read_only);
        assert!(config.run_id.starts_with("prod-audit-"));
    }
    
    #[test]
    fn test_audit_tags() {
        use nostr_sdk::prelude::{Alphabet, SingleLetterTag};
        
        let config = AuditConfig::ci();
        let tags = config.audit_tags();
        
        assert_eq!(tags.len(), 3);
        
        let g_tag = SingleLetterTag::lowercase(Alphabet::G);
        let r_tag = SingleLetterTag::lowercase(Alphabet::R);
        let c_tag = SingleLetterTag::lowercase(Alphabet::C);
        
        // Check "g" tag (grasp-audit marker)
        assert!(tags.iter().any(|t| {
            if let TagKind::SingleLetter(letter) = t.kind() {
                letter == g_tag
            } else {
                false
            }
        }));
        
        // Check "r" tag (audit run ID)
        assert!(tags.iter().any(|t| {
            if let TagKind::SingleLetter(letter) = t.kind() {
                letter == r_tag
            } else {
                false
            }
        }));
        
        // Check "c" tag (cleanup timestamp)
        assert!(tags.iter().any(|t| {
            if let TagKind::SingleLetter(letter) = t.kind() {
                letter == c_tag
            } else {
                false
            }
        }));
    }
    
    #[test]
    fn test_audit_event_builder() {
        let config = AuditConfig::ci();
        let keys = Keys::generate();
        
        let event = AuditEventBuilder::new(Kind::TextNote, "test", config.clone())
            .tag(Tag::custom(TagKind::Custom("test".into()), vec!["value"]))
            .build(&keys)
            .unwrap();
        
        // Should have our custom tag + 3 audit tags
        assert!(event.tags.len() >= 4);
        
        // Verify event is valid
        assert!(event.verify().is_ok());
    }
}
