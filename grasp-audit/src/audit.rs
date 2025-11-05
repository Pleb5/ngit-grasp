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
    
    /// Get audit tags that are automatically added to all events
    ///
    /// These tags are automatically added to all events created via [`AuditEventBuilder`].
    /// They provide isolation, cleanup scheduling, and easy discovery of audit events.
    ///
    /// # Tag Format
    ///
    /// All tags use the `"t"` (hashtag) format for maximum relay compatibility:
    ///
    /// 1. `["t", "grasp-audit-test-event"]` - Identifies all audit-related events
    /// 2. `["t", "audit-{run_id}"]` - Unique identifier for this audit run
    ///    - CI mode: `audit-ci-{uuid}`
    ///    - Production mode: `audit-prod-audit-{timestamp}`
    /// 3. `["t", "audit-cleanup-after-{unix_timestamp}"]` - Cleanup timestamp
    ///    - CI mode: Current time + 3600 seconds (1 hour)
    ///    - Production mode: Current time + 300 seconds (5 minutes)
    ///
    /// # Purpose
    ///
    /// - **Isolation**: Each test run has a unique ID for event filtering in CI mode
    /// - **Cleanup**: Events marked for cleanup after timestamp (enables direct DB cleanup)
    /// - **Discovery**: Easy to query all audit events via hashtag
    /// - **No deletion trails**: Avoids NIP-09 deletion events by using direct cleanup
    ///
    /// # Example
    ///
    /// ```rust
    /// use grasp_audit::AuditConfig;
    ///
    /// let config = AuditConfig::ci();
    /// let tags = config.audit_tags();
    ///
    /// // Tags will look like:
    /// // [
    /// //   ["t", "grasp-audit-test-event"],
    /// //   ["t", "audit-ci-a1b2c3d4-e5f6-7890-abcd-ef1234567890"],
    /// //   ["t", "audit-cleanup-after-1730822334"]
    /// // ]
    /// ```
    pub fn audit_tags(&self) -> Vec<Tag> {
        use nostr_sdk::prelude::{Alphabet, SingleLetterTag};
        
        // Use "t" tags for categorization (standard NIP-01 hashtag type)
        let t_tag = SingleLetterTag::lowercase(Alphabet::T);
        
        vec![
            Tag::custom(
                TagKind::SingleLetter(t_tag),
                vec!["grasp-audit-test-event"]
            ),
            Tag::custom(
                TagKind::SingleLetter(t_tag),
                vec![format!("audit-{}", self.run_id)]
            ),
            Tag::custom(
                TagKind::SingleLetter(t_tag),
                vec![format!("audit-cleanup-after-{}", self.cleanup_after.as_u64())]
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
        
        let t_tag = SingleLetterTag::lowercase(Alphabet::T);
        
        // All tags should be "t" tags (hashtags)
        for tag in &tags {
            if let TagKind::SingleLetter(letter) = tag.kind() {
                assert_eq!(letter, t_tag);
            } else {
                panic!("Expected SingleLetter tag");
            }
        }
        
        // Check for "t" tag with "grasp-audit-test-event"
        assert!(tags.iter().any(|t| {
            t.content() == Some("grasp-audit-test-event")
        }));
        
        // Check for "t" tag with "audit-{run_id}"
        assert!(tags.iter().any(|t| {
            t.content().map(|c| c.starts_with("audit-ci-")).unwrap_or(false)
        }));
        
        // Check for "t" tag with "audit-cleanup-after-{timestamp}"
        assert!(tags.iter().any(|t| {
            t.content().map(|c| c.starts_with("audit-cleanup-after-")).unwrap_or(false)
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
