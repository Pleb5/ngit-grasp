//! Test isolation utilities

use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique test ID
pub fn generate_test_id() -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    format!("test-{}-{}", timestamp, counter)
}

/// Generate a unique audit run ID for CI
pub fn generate_ci_run_id() -> String {
    format!("ci-{}", uuid::Uuid::new_v4())
}

/// Generate a unique audit run ID for production
pub fn generate_prod_run_id() -> String {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    format!("prod-audit-{}", timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_generate_test_id() {
        let id1 = generate_test_id();
        let id2 = generate_test_id();
        
        assert_ne!(id1, id2);
        assert!(id1.starts_with("test-"));
    }
    
    #[test]
    fn test_generate_ci_run_id() {
        let id = generate_ci_run_id();
        assert!(id.starts_with("ci-"));
    }
    
    #[test]
    fn test_generate_prod_run_id() {
        let id = generate_prod_run_id();
        assert!(id.starts_with("prod-audit-"));
    }
}
